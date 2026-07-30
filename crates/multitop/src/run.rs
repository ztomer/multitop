//! The async runtime: terminal event loop plus one SSH task per panel.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_stream::StreamExt as _;

use crate::app::{App, Command, Msg};
use crate::config::Server;
use crate::ui;
use ratatui::layout::Rect;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);
pub(super) const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];

use std::path::PathBuf;

pub async fn run(
    servers: Vec<Server>,
    config_path: PathBuf,
    initial_theme: Option<String>,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, servers, config_path, initial_theme).await;
    ratatui::restore();
    result
}

pub struct Tasks {
    monitors: Vec<Option<JoinHandle<()>>>,
    pub aux: Vec<Option<JoinHandle<()>>>,
    /// Tracks whether each aux task is an upgrade task. Upgrade tasks are not
    /// aborted when switching views, so long-running upgrades continue in the
    /// background until they complete.
    pub aux_is_upgrade: Vec<bool>,
}

impl Tasks {
    pub fn new(n: usize) -> Self {
        Tasks {
            monitors: (0..n).map(|_| None).collect(),
            aux: (0..n).map(|_| None).collect(),
            aux_is_upgrade: (0..n).map(|_| false).collect(),
        }
    }

    /// Aborting a task drops the `Child` it owns, and every child is spawned
    /// with `kill_on_drop`, so this also terminates the SSH process.
    fn abort_all(&mut self) {
        for h in self
            .monitors
            .iter_mut()
            .chain(self.aux.iter_mut())
            .flatten()
        {
            h.abort();
        }
    }
}

use multitop_agent::SortBy;

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    servers: Vec<Server>,
    config_path: PathBuf,
    initial_theme: Option<String>,
) -> std::io::Result<()> {
    let n = servers.len();
    let mut app = App::new(servers.clone());
    app.config_path = Some(config_path.clone());
    // Passwords are loaded on-demand via Panel::ensure_sudo_password() when
    // the user initiates an upgrade, not at startup. This avoids triggering
    // OS keychain access dialogs on every app launch.
    if let Ok(cfg) = crate::config::load(&config_path) {
        app.upgrade_history_lines = cfg.upgrade_history_lines;
        app.show_sparklines = cfg.show_sparklines;
    }
    let state = crate::state::load_state(&config_path);
    app.last_update = state.last_update;
    if let Some(ref tname) = initial_theme {
        if let Some(idx) = multitop_agent::color::THEMES
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(tname))
        {
            app.theme_idx = idx;
        }
    }
    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let mut tasks = Tasks::new(n);
    let mut events = crossterm::event::EventStream::new();

    let mut dims = ui::agent_dims(terminal.size()?, n);
    let (dims_tx, dims_rx) = watch::channel(dims);
    let dims_rx = Arc::new(dims_rx);
    for (i, server) in servers.iter().enumerate() {
        tasks.monitors[i] = Some(spawn_monitor(
            i,
            server.clone(),
            dims_rx.clone(),
            app.sort,
            tx.clone(),
        ));
    }

    let mut resize_at: Option<Instant> = None;
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|f| ui::draw(f, &app))?;
            dirty = false;
        }

        let resize_wait = async {
            match resize_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            biased;

            maybe = events.next() => {
                match maybe {
                    Some(Ok(Event::Key(key))) => {
                        handle_key(key, &mut app, &servers, dims, dims_rx.clone(), &tx, &mut tasks);
                        dirty = true;
                    }
                    Some(Ok(Event::Mouse(mouse))) => {
                        let term_size = terminal.size().unwrap_or_default();
                        let term_area = Rect::new(0, 0, term_size.width, term_size.height);
                        let target_panel = panel_at_pos(mouse.column, mouse.row, term_area, app.panels.len());
                        match mouse.kind {
                            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                                app.selected_panel = target_panel;
                                dirty = true;
                            }
                            MouseEventKind::ScrollUp => {
                                app.scroll_panel_up(target_panel, 3);
                                dirty = true;
                            }
                            MouseEventKind::ScrollDown => {
                                app.scroll_panel_down(target_panel, 3);
                                dirty = true;
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Event::Resize(..))) => {
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
                app.apply(msg);
                // A burst of frames should cost one draw, not one each.
                while let Ok(msg) = rx.try_recv() {
                    app.apply(msg);
                }
                dirty = true;
            }
            _ = resize_wait, if resize_at.is_some() => {
                resize_at = None;
                let new_dims = ui::agent_dims(terminal.size()?, n);
                if new_dims != dims {
                    dims = new_dims;
                    let _ = dims_tx.send(new_dims);
                    // Re-render panels at the new size so logos and stats adapt.
                    app.rerender_all(new_dims);
                }
                dirty = true;
            }
        }

        if app.should_quit {
            tasks.abort_all();
            return Ok(());
        }
    }
}

fn panel_at_pos(x: u16, y: u16, total_area: Rect, panel_count: usize) -> usize {
    if panel_count == 0 {
        return 0;
    }
    let (areas, _) = crate::ui::regions(total_area, panel_count);
    for (i, a) in areas.iter().enumerate() {
        if x >= a.x && x < a.x + a.width && y >= a.y && y < a.y + a.height {
            return i;
        }
    }
    0
}

fn handle_key(
    key: KeyEvent,
    app: &mut App,
    servers: &[Server],
    dims: (u16, u16),
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    // Key *releases* also arrive on terminals that report them; acting on
    // both would run every action twice.
    if key.kind != KeyEventKind::Press {
        return;
    }

    if app.show_upgrade_modal {
        match key.code {
            KeyCode::Char('u') | KeyCode::Char('U') | KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let cmds = app.confirm_upgrade();
                execute_cmds(cmds, app, servers, dims, tx, tasks);
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('n') | KeyCode::Char('N') => {
                app.show_upgrade_modal = false;
            }
            _ => {}
        }
        return;
    }

    if app.password_manager.is_some() {
        let action = crate::passwords::handle_key(app, key.code);
        crate::password_actions::apply(action, app, servers, tx, tasks);
        return;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.quit();
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
            return;
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            crate::passwords::open(app, app.selected_panel, false);
            return;
        }
        KeyCode::Char(c @ '1'..='9') => {
            let idx = ((c as usize) - ('1' as usize)).min(app.panels.len().saturating_sub(1));
            app.selected_panel = idx;
            return;
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            let old_sort = app.sort;
            app.sort = SortBy::Cpu;
            if old_sort != app.sort {
                restart_all_agents(app, servers, dims_rx.clone(), tx, tasks);
            }
            return;
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            let old_sort = app.sort;
            app.sort = SortBy::Mem;
            if old_sort != app.sort {
                restart_all_agents(app, servers, dims_rx.clone(), tx, tasks);
            }
            return;
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            app.cycle_theme();
            if let Some(ref path) = app.config_path {
                crate::config::save_theme(path, app.current_theme().name);
            }
            app.rerender_all(dims);
            return;
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
            app.scroll_up(1);
            return;
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
            app.scroll_down(1);
            return;
        }
        KeyCode::PageUp => {
            app.scroll_up(15);
            return;
        }
        KeyCode::PageDown => {
            app.scroll_down(15);
            return;
        }
        KeyCode::Home => {
            app.scroll_to_top();
            return;
        }
        KeyCode::End => {
            app.reset_scroll();
            return;
        }
        _ => {}
    }

    let cmds = match key.code {
        KeyCode::Char('f') | KeyCode::Char('F') => app.toggle_fetch(),
        KeyCode::Char('d') | KeyCode::Char('D') => app.toggle_docker(),
        KeyCode::Char('s') | KeyCode::Char('S') => app.switch_stats(),
        KeyCode::Char('u') | KeyCode::Char('U') => {
            if app.upgrades_in_flight > 0 {
                // Upgrade already running — don't interrupt
            } else if app.had_upgrade {
                if app.in_upgrade() {
                    let cmds = app.run_upgrade();
                    execute_cmds(cmds, app, servers, dims, tx, tasks);
                } else {
                    app.show_upgrade_output();
                }
            } else {
                app.show_upgrade_modal = true;
            }
            Vec::new()
        }
        _ => return,
    };

    execute_cmds(cmds, app, servers, dims, tx, tasks);
}

fn execute_cmds(
    cmds: Vec<Command>,
    app: &App,
    servers: &[Server],
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    for cmd in cmds {
        let (idx, handle) = match cmd {
            Command::RunFetch { panel, gen } => (
                panel,
                crate::tasks::spawn_fetch(
                    panel,
                    gen,
                    servers[panel].clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                ),
            ),
            Command::RunDocker { panel, gen } => (
                panel,
                crate::tasks::spawn_docker(
                    panel,
                    gen,
                    servers[panel].clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                ),
            ),
            Command::RunUpgrade { panel, gen } => (
                panel,
                crate::tasks::spawn_upgrade(
                    panel,
                    gen,
                    servers[panel].clone(),
                    app.panels[panel].sudo_password.clone(),
                    tx.clone(),
                ),
            ),
        };
        // Supersede whatever that panel was running, except for upgrade tasks
        // which should continue running in the background until they complete.
        // This prevents interrupting long-running upgrades when the user switches
        // views (e.g., pressing 's' or 'd' while an upgrade is in progress).
        let is_upgrade = matches!(cmd, Command::RunUpgrade { .. });
        let was_upgrade = tasks.aux_is_upgrade[idx];
        tasks.aux_is_upgrade[idx] = is_upgrade;
        if let Some(old) = tasks.aux[idx].replace(handle) {
            if !was_upgrade {
                old.abort();
            }
        }
    }
}

fn restart_all_agents(
    app: &App,
    servers: &[Server],
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    for (i, server) in servers.iter().enumerate() {
        if let Some(h) = tasks.monitors[i].take() {
            h.abort();
        }
        tasks.monitors[i] = Some(spawn_monitor(
            i,
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
                let was_upgrade = tasks.aux_is_upgrade[i];
                tasks.aux_is_upgrade[i] = false;
                if let Some(old) = tasks.aux[i].replace(crate::tasks::spawn_docker(
                    i,
                    gen,
                    servers[i].clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                )) {
                    if !was_upgrade {
                        old.abort();
                    }
                }
            }
        }
    }
}

/// Long-lived: streams monitor frames and reconnects on failure.
mod spawn;
use spawn::spawn_monitor;

