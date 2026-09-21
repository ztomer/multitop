//! The terminal event loop.
//!
//! Generic over the backend and the event source so the whole loop can be
//! driven from a test: every defect this file has ever shipped lived here, and
//! until this seam existed none of them could be caught by anything but a
//! person watching a real terminal.

use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{Event, MouseEventKind};
use ratatui::layout::Rect;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_stream::StreamExt as _;

use crate::app::{App, Msg};
use crate::config::Server;
use crate::diag::Phase;

use super::boot::boot_app;
use super::dims::{self, AgentDims};
use super::handle_key::handle_key;
use super::spawn::spawn_monitor;
use super::tasks::Tasks;
use super::terminal::{next_terminal_signal, SignalAction, TerminalSignals};

use super::RESIZE_DEBOUNCE;

/// Why the event loop stopped, and what it killed on the way out.
///
/// Not a `Result`: the killed-host list and the error are both needed by the
/// caller, and a `Result` can only carry one of them. It carried the error.
pub struct LoopOutcome {
    /// Hosts whose upgrade was still running when the loop ended.
    pub killed: Vec<String>,
    /// The terminal failure that ended the loop, if it was not a clean quit.
    pub error: Option<std::io::Error>,
}

/// One task per panel at startup: a custom `[[panels]] command` runs every
/// 250 ms via Exec (rendered as a Fetch card), everything else gets a monitor
/// stream. Iterates panels, not servers, so `panel.gen` is always available
/// and the task list stays aligned with the panel list -- the one place that
/// decides how many monitors exist.
fn spawn_initial_tasks(
    app: &mut App,
    tasks: &mut Tasks,
    dims_rx: &Arc<watch::Receiver<(u16, u16)>>,
    tx: &Sender<Msg>,
) {
    // Iterate panels, not servers, so `panel.gen` is always available and
    // the task list stays aligned with the panel list — the one place that
    // decides how many monitors exist.
    for (i, panel) in app.panels.iter().enumerate() {
        if let Some(cmd) = panel.server.custom_command.clone() {
            // `[[panels]] command="…"` per roadmap Phase 3 — runs every 250 ms
            // via Exec pty, rendered as a Fetch card (`render_payload.rs:20`).
            let gen = panel.gen;
            let server = panel.server.clone();
            let pass = panel.sudo_password.clone();
            tasks.set_aux(
                i,
                crate::tasks::spawn_custom(i, gen, server, cmd, pass, tx.clone()),
            );
            // Custom panels start as Fetch so the card is visible without `f`.
            // The monitor stream is not needed for them.
            // Note: `app.panels[i].mode` is still Monitor at this point; set
            // through mutable borrow after the loop to avoid borrow checker.
        } else {
            tasks.monitors[i] = Some(spawn_monitor(
                i,
                panel.gen,
                app.panels_epoch,
                panel.server.clone(),
                dims_rx.clone(),
                app.sort,
                tx.clone(),
            ));
        }
    }
    for (i, panel) in app.panels.iter_mut().enumerate() {
        if panel.server.custom_command.is_some() {
            panel.mode = crate::panel::Mode::Fetch;
        }
        let _ = i;
    }
}

/// A click selects the pane under it; the wheel scrolls it. Everything else
/// -- and there is a lot of it, since `EnableMouseCapture`'s `?1003h` asks
/// for *any*-event tracking -- is discarded before the layout is computed.
/// Returns whether the screen changed. `size` is the terminal's, or the last
/// measured one when the query fails.
fn handle_mouse(
    app: &mut App,
    mouse: crossterm::event::MouseEvent,
    size: Option<(u16, u16)>,
) -> bool {
    if !matches!(
        mouse.kind,
        MouseEventKind::Down(crossterm::event::MouseButton::Left)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
    ) {
        return false;
    }
    let Some((w, h)) = size else {
        return false;
    };
    let term_area = Rect::new(0, 0, w, h);
    // The same list `ui::draw` lays the grid out from, so the rectangles
    // being tested are the ones on screen.
    let Some(target_panel) =
        panel_at_pos(mouse.column, mouse.row, term_area, &app.filtered_indices())
    else {
        return false;
    };
    match mouse.kind {
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            app.selected_panel = target_panel;
            app.persist_state();
        }
        MouseEventKind::ScrollUp => app.scroll_panel_up(target_panel, 3),
        MouseEventKind::ScrollDown => app.scroll_panel_down(target_panel, 3),
        _ => return false,
    }
    true
}

/// Apply `first` and a bounded burst of whatever else is queued, then run the
/// upgrade watchdog. Returns whether the screen changed.
///
/// A burst of frames should cost one draw, not one each -- and a message that
/// changes nothing on screen should cost none. The agent streams one Packet
/// per panel per tick even when every panel is showing another view; draining
/// those without a redraw is the difference between a smooth idle TUI and a
/// flickering one.
///
/// Bounded: a single producer (e.g. an upgrade streaming output) can flood the
/// channel faster than the loop can drain it. An unbounded drain here would
/// starve the `events.next()` branch and the UI would read keys but not act
/// on them. Capping the work per select poll gives key events their turn; the
/// next poll drains the rest. Counted up rather than down: `budget -= 1`
/// underflows and panics the moment the constant is ever set to zero.
fn drain_messages(
    app: &mut App,
    first: Msg,
    rx: &mut mpsc::Receiver<Msg>,
    diag: &crate::diag::Diag,
    tasks: &Tasks,
) -> bool {
    let mut change = app.apply(first);
    diag.bump_applied();
    let mut drained = 1;
    for _ in 0..crate::consts::MSG_DRAIN_BUDGET {
        let Ok(msg) = rx.try_recv() else { break };
        change |= app.apply(msg);
        diag.bump_applied();
        drained += 1;
    }
    diag.bump_drained(drained);
    // Watchdog: a task that dies without sending AuxDone leaves its panel
    // stuck in STARTED forever. Detect a finished handle and record the
    // interruption.
    let dead: Vec<usize> = app
        .panels
        .iter()
        .enumerate()
        .filter(|(_, p)| p.upgrade_state == crate::panel::UpgradeState::STARTED)
        .filter(|(i, _)| {
            tasks.upgrades[*i]
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
        })
        .map(|(i, _)| i)
        .collect();
    for i in dead {
        app.mark_upgrade_interrupted(i);
    }
    change
}

/// Re-render every panel when the terminal's size (or the pane count) has
/// changed; a failed size query keeps the last one -- see `size_change`.
fn refit<B>(terminal: &ratatui::Terminal<B>, dims: &mut AgentDims, app: &mut App)
where
    B: ratatui::backend::Backend,
{
    if let Some(d) = dims::size_change(terminal.size(), dims, app.visible_panes()) {
        app.rerender_all(d);
    }
}

/// A dump was asked for. The signal thread has already written the
/// signal-tier file; this is the richer tier only the loop can produce,
/// because only it may touch `App`. State tier present => the loop is alive;
/// absent while the signal tier exists => the loop is wedged.
fn dump_state_tier(
    diag: &crate::diag::Diag,
    app: &App,
    tasks: &Tasks,
    seq: u64,
    sig: &'static str,
) {
    let snap = crate::diag::snapshot_app(app, tasks);
    diag.store_snapshot(snap.clone());
    if let Some(path) = diag.write_state_tier(seq, sig, &snap) {
        // Through `diag::report`, never `eprintln!`: stderr is the terminal
        // this loop is drawing on, and this is the noisiest of the five sites
        // -- one wrapped path across the frame for every signal.
        crate::diag::report(&format!("diag: {sig}: wrote {}", path.display()));
    }
}

/// The terminal's size, or the last measured one when the query fails: a
/// zero area matches no pane, so a failed query used to swallow the click
/// silently -- and a click that does nothing reads as a dead button.
fn terminal_size<B>(terminal: &ratatui::Terminal<B>, dims: &AgentDims) -> Option<(u16, u16)>
where
    B: ratatui::backend::Backend,
{
    terminal
        .size()
        .ok()
        .map(|s| (s.width, s.height))
        .or_else(|| dims.last_size())
}

/// The terminal event loop.
pub async fn event_loop<B, S>(
    terminal: &mut ratatui::Terminal<B>,
    events: &mut S,
    dims_tx: watch::Sender<(u16, u16)>,
    servers: Vec<Server>,
    config_path: PathBuf,
    initial_theme: Option<String>,
) -> LoopOutcome
where
    B: ratatui::backend::Backend,
    B::Error: std::error::Error + Send + Sync + 'static,
    S: tokio_stream::Stream<Item = std::io::Result<Event>> + Unpin,
{
    // First, before the config read, the state read and the vault probe: a stop
    // that lands in that window is a stop with no handler to catch it and no
    // `SIGCONT` handler to repair the terminal afterwards. A failure to install
    // is not worth refusing to start over -- it only costs the terminal repair,
    // which is exactly the state the app was in before they existed.
    //
    // Out-of-band diagnostics sit next to it: SIGUSR1/SIGUSR2 dump the loop's
    // phase and a snapshot from a thread that does not touch this loop, so a
    // wedge stays diagnosable from outside it. Installed as early as the config
    // read -- that read touches the OS keychain, which is one of the reasons a
    // loop can stop answering.
    let diag = crate::diag::Diag::new(crate::diag::Diag::default_dir());
    crate::diag::install(&diag);

    #[cfg(unix)]
    let mut signals = TerminalSignals::install().ok();
    #[cfg(not(unix))]
    let mut signals: Option<()> = None;

    let mut app = boot_app(&servers, &config_path, initial_theme.as_deref());
    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let mut tasks = Tasks::new(servers.len());

    let dims_rx = Arc::new(dims_tx.subscribe());
    let mut dims = AgentDims::new(dims_tx, &terminal.size(), app.visible_panes());
    spawn_initial_tasks(&mut app, &mut tasks, &dims_rx, &tx);

    // Tracks which panel list the running tasks were started for.
    let mut known_epoch = app.panels_epoch;
    let mut resize_at: Option<Instant> = None;
    let mut dirty = true;
    // Set when the terminal itself fails. Reported after the killed-upgrade
    // notice rather than instead of it.
    let mut fatal: Option<std::io::Error> = None;

    loop {
        if dirty {
            diag.set_phase(Phase::Drawing);
            if let Err(e) = terminal.draw(|f| crate::ui::draw(f, &mut app)) {
                fatal = Some(std::io::Error::other(e));
                break;
            }
            dirty = false;
        }

        let resize_wait = async {
            match resize_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            action = next_terminal_signal(&mut signals) => match action {
                SignalAction::Quit => app.quit(),
                SignalAction::Resumed => {
                    super::terminal::reclaim_terminal();
                    let _ = terminal.clear();
                    refit(terminal, &mut dims, &mut app);
                    dirty = true;
                }
                SignalAction::Ignore => {}
            },

            maybe = events.next() => {
                diag.set_phase(Phase::HandlingKey);
                match maybe {
                    Some(Ok(Event::Key(key))) => {
                        diag.bump_key();
                        handle_key(key, &mut app, dims.current(), &dims_rx, &tx, &mut tasks);
                        let epoch_changed = app.panels_epoch != known_epoch;
                        if epoch_changed {
                            known_epoch = app.panels_epoch;
                            tasks.fit_to(app.panels.len());
                        }
                        // Any key can change how many panes are on screen --
                        // typing a filter is the common one, and it re-splits
                        // the grid on every keystroke. This is where the render
                        // size follows it (no debounce: `size_change` diffs
                        // the whole input signature, so a keystroke that
                        // changes nothing costs one comparison). After the
                        // size, so restarted agents read the new one rather
                        // than the size the old panel list implied.
                        refit(terminal, &mut dims, &mut app);
                        if epoch_changed {
                            restart_all_agents(&app, &dims_rx, &tx, &mut tasks);
                        }
                        dirty = true;
                    }
                    Some(Ok(Event::Mouse(mouse))) => {
                        dirty |= handle_mouse(&mut app, mouse, terminal_size(terminal, &dims));
                    }
                    Some(Ok(Event::Resize(..))) => {
                        diag.set_phase(Phase::Resizing);
                        resize_at = Some(Instant::now() + RESIZE_DEBOUNCE);
                        dirty = true;
                    }
                    Some(Ok(_)) => {}
                    // The terminal went away; leaving would strand the SSH
                    // children, so exit through the normal path.
                    Some(Err(_)) | None => app.quit(),
                }
            }

            Some(msg) = rx.recv() => {
                diag.set_phase(Phase::Applying);
                dirty |= drain_messages(&mut app, msg, &mut rx, &diag, &tasks);
            }
            () = resize_wait, if resize_at.is_some() => {
                diag.set_phase(Phase::Resizing);
                resize_at = None;
                // Re-render panels at the new size so logos and stats adapt. A
                // failed size query keeps the last one -- see `size_change`; this
                // arm used to make it fatal, which killed every running upgrade.
                refit(terminal, &mut dims, &mut app);
                dirty = true;
            }
        }

        diag.bump_iter();
        if let Some((seq, sig)) = diag.take_request() {
            dump_state_tier(&diag, &app, &tasks, seq, sig);
        }
        diag.set_phase(Phase::Idle);

        if app.should_quit() {
            break;
        }
    }

    // Named before `abort_all` flips the STARTED flags to DONE.
    let killed = app.running_upgrade_hosts();
    tasks.abort_all(&mut app);
    LoopOutcome {
        killed,
        error: fatal,
    }
}

/// Which panel a click landed on, or `None` if it landed on no panel at all.
///
/// `shown` is the list `ui::draw` laid the grid out from, and both the split
/// and the answer have to come from it. Splitting by `panels.len()` while the
/// screen was split by the filtered count meant that with a filter applied the
/// rectangles being tested against were not the ones on screen, and the index
/// they produced was an index into the unfiltered list: a click selected some
/// other host, and a scroll scrolled it.
///
/// `None` rather than a fallback of zero. A click on the keybar, or on the gap
/// under an odd last row, matches no pane -- and answering "panel 0" to that
/// moved the selection to the first host whenever the user clicked the keys
/// row, which is the row that invites clicking.
#[must_use]
pub fn panel_at_pos(x: u16, y: u16, total_area: Rect, shown: &[usize]) -> Option<usize> {
    if shown.is_empty() {
        return None;
    }
    let (areas, _) = crate::ui::regions(total_area, shown.len());
    areas
        .iter()
        .position(|a| x >= a.x && x < a.x + a.width && y >= a.y && y < a.y + a.height)
        .and_then(|slot| shown.get(slot).copied())
}

pub(super) fn restart_all_agents(
    app: &App,
    dims_rx: &Arc<watch::Receiver<(u16, u16)>>,
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    // Sized from the panel list here rather than trusted to have been sized
    // elsewhere, and iterated over the panels rather than a caller's copy of
    // them: the two are the same list, and only one of them cannot go stale.
    tasks.fit_to(app.panels.len());
    for (i, panel) in app.panels.iter().enumerate() {
        let server = &panel.server;
        if let Some(h) = tasks.monitors[i].take() {
            h.abort();
        }
        tasks.monitors[i] = Some(spawn_monitor(
            i,
            panel.gen,
            app.panels_epoch,
            server.clone(),
            dims_rx.clone(),
            app.sort,
            tx.clone(),
        ));
    }
    if app.in_docker() {
        let dims = *dims_rx.borrow();
        for (i, panel) in app.panels.iter().enumerate() {
            if panel.mode == crate::app::Mode::Docker {
                let gen = panel.gen;
                tasks.set_aux(
                    i,
                    crate::tasks::spawn_docker(
                        i,
                        gen,
                        app.panels_epoch,
                        panel.server.clone(),
                        dims,
                        app.sort,
                        tx.clone(),
                    ),
                );
            }
        }
    }
}
