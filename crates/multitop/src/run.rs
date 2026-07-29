//! The async runtime: terminal event loop plus one SSH task per panel.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};
use tokio_stream::StreamExt as _;

use crate::app::{error_line, App, Command, Msg};
use crate::config::Server;
use crate::ssh::Mode;
use crate::stream;
use crate::ui;
use ratatui::layout::Rect;

/// How long to wait after the last resize event before restarting the agents
/// at the new size. Dragging a window edge emits a burst of events.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);

/// Backoff between reconnection attempts after a dropped SSH session.
const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];


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

struct Tasks {
    monitors: Vec<Option<JoinHandle<()>>>,
    aux: Vec<Option<JoinHandle<()>>>,
}

impl Tasks {
    fn new(n: usize) -> Self {
        Tasks {
            monitors: (0..n).map(|_| None).collect(),
            aux: (0..n).map(|_| None).collect(),
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
    if let Ok(cfg) = crate::config::load(&config_path) {
        app.upgrade_history_lines = cfg.upgrade_history_lines;
    }
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
        tasks.monitors[i] = Some(spawn_monitor(i, server.clone(), dims_rx.clone(), app.sort, tx.clone()));
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

    // Any panel prompting for sudo intercepts keys, but only the selected
    // panel receives typed characters.  Number keys switch the active panel.
    if app.panels.iter().any(|p| p.prompt_sudo) {
        let sel = app.selected_panel;
        match key.code {
            KeyCode::Char(c @ '1'..='9') => {
                let target = (c as usize) - ('1' as usize);
                if target < app.panels.len() {
                    app.selected_panel = target;
                }
            }
            KeyCode::Esc => {
                app.panels[sel].prompt_sudo = false;
                app.panels[sel].password_input.clear();
            }
            KeyCode::Enter if app.panels[sel].prompt_sudo => {
                let pass = app.panels[sel].password_input.clone();
                app.panels[sel].prompt_sudo = false;
                app.panels[sel].password_input.clear();
                if !pass.trim().is_empty() {
                    app.panels[sel].sudo_password = Some(pass.clone());
                    if let Some(ref path) = app.config_path {
                        crate::config::save_sudo_password(path, &app.panels[sel].server.host, &pass);
                    }
                    if servers[sel].upgrade_cmd.is_some() {
                        let gen = app.bump(sel);
                        let pal = app.current_theme();
                        app.panels[sel].view = vec![format!("{}\u{2192} Upgrade running...{}", pal.meter_mid(), pal.reset)];
                        let handle = crate::tasks::spawn_upgrade(
                            sel, gen, servers[sel].clone(), Some(pass), tx.clone(),
                        );
                        if let Some(old) = tasks.aux[sel].replace(handle) {
                            old.abort();
                        }
                    }
                }
            }
            KeyCode::Backspace if app.panels[sel].prompt_sudo => {
                app.panels[sel].password_input.pop();
            }
            KeyCode::Char(c) if app.panels[sel].prompt_sudo => {
                app.panels[sel].password_input.push(c);
            }
            _ => {}
        }
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
        KeyCode::Char('p') | KeyCode::Char('P') => {
            if app.selected_panel < app.panels.len() {
                app.panels[app.selected_panel].prompt_sudo = true;
                app.panels[app.selected_panel].password_input.clear();
            }
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
        KeyCode::Char('u') | KeyCode::Char('U') => app.run_upgrade(),
        _ => return,
    };

    for cmd in cmds {
        let (idx, handle) = match cmd {
            Command::RunFetch { panel, gen } => (
                panel,
                crate::tasks::spawn_fetch(panel, gen, servers[panel].clone(), dims, app.sort, tx.clone()),
            ),
            Command::RunDocker { panel, gen } => (
                panel,
                crate::tasks::spawn_docker(panel, gen, servers[panel].clone(), dims, app.sort, tx.clone()),
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
        // Supersede whatever that panel was running.
        if let Some(old) = tasks.aux[idx].replace(handle) {
            old.abort();
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
        tasks.monitors[i] = Some(spawn_monitor(i, server.clone(), dims_rx.clone(), app.sort, tx.clone()));
    }
    if app.in_docker() {
        let dims = *dims_rx.borrow();
        for (i, panel) in app.panels.iter().enumerate() {
            if panel.mode == crate::app::Mode::Docker {
                let gen = panel.gen;
                if let Some(old) = tasks.aux[i].replace(crate::tasks::spawn_docker(i, gen, servers[i].clone(), dims, app.sort, tx.clone())) {
                    old.abort();
                }
            }
        }
    }
}

/// Long-lived: streams monitor frames and reconnects on failure.
///
/// This task keeps running through Docker and Upgrade views, so stats stay
/// warm and switching back is instant.
fn spawn_monitor(
    idx: usize,
    server: Server,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut failures = 0usize;
        loop {
            let status_tx = tx.clone();
            let notify = move |text: String| {
                let _ = status_tx.try_send(Msg::Frame {
                    panel: idx,
                    lines: vec![text],
                });
            };

            match stream::connect(&server, Mode::Monitor, sort, notify).await {
                Ok(mut stream) => {
                    failures = 0;
                    let mut errbuf = Vec::new();
                    while let Ok(Some(payload)) = stream::next_packet(&mut stream, &mut errbuf).await {
                        let dims = *dims_rx.borrow();
                        if tx.send(Msg::Packet { panel: idx, gen: 0, payload, dims }).await.is_err() {
                            return;
                        }
                    }

                    let detail = errbuf
                        .last()
                        .cloned()
                        .unwrap_or_else(|| format!("Connection to {} closed", server.host));
                    let _ = tx
                        .send(Msg::Frame {
                            panel: idx,
                            lines: vec![error_line(detail)],
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Msg::Frame {
                            panel: idx,
                            lines: vec![error_line(e)],
                        })
                        .await;
                }
            }

            let wait = RECONNECT_BACKOFF[failures.min(RECONNECT_BACKOFF.len() - 1)];
            failures += 1;
            sleep(Duration::from_secs(wait)).await;
        }
    })
}



pub use crate::render_payload::render_payload;
