//! The async runtime: terminal event loop plus one SSH task per panel.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use crossterm::execute;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_stream::StreamExt as _;

use crate::app::{App, Command, Msg, VaultState};
use crate::config::Server;
use crate::ui;
use ratatui::layout::Rect;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);
pub(super) const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];

use std::path::PathBuf;

/// Run the multitop application.
///
/// # Errors
///
/// Returns an error if the terminal cannot be initialized or the event loop fails.
pub async fn run(
    servers: Vec<Server>,
    config_path: PathBuf,
    initial_theme: Option<String>,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = event_loop(&mut terminal, servers, config_path, initial_theme).await;
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

/// How long to wait for a biometric result before giving up and offering the
/// password prompt instead.
const BIOMETRIC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

pub struct Tasks {
    monitors: Vec<Option<JoinHandle<()>>>,
    pub aux: Vec<Option<JoinHandle<()>>>,
    /// Tracks whether each aux task is an upgrade task. Upgrade tasks are not
    /// aborted when switching views, so long-running upgrades continue in the
    /// background until they complete.
    pub aux_is_upgrade: Vec<bool>,
}

impl Tasks {
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self {
            monitors: (0..n).map(|_| None).collect(),
            aux: (0..n).map(|_| None).collect(),
            aux_is_upgrade: (0..n).map(|_| false).collect(),
        }
    }

    /// Aborting a task drops the `Child` it owns, and every child is spawned
    /// with `kill_on_drop`, so this also terminates the SSH process.
    fn abort_all(&mut self, app: &mut App) {
        for h in self
            .monitors
            .iter_mut()
            .chain(self.aux.iter_mut())
            .flatten()
        {
            h.abort();
        }
        for p in &mut app.panels {
            if p.upgrade_state == crate::panel::UpgradeState::STARTED {
                p.upgrade_state = crate::panel::UpgradeState::DONE;
            }
        }
    }
}

use multitop_agent::SortBy;

#[allow(clippy::too_many_lines)]
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
        if cfg.show_sparklines {
            app.toggle_sparklines();
        }
        // A sudo password in config.toml is plaintext on disk and was never
        // even read. Move any we find into the OS credential store and delete
        // them from the file, once, so the unsupported mechanism cannot linger.
        if !cfg.plaintext_passwords.is_empty() {
            crate::password_actions::port_plaintext_passwords(
                &mut app,
                &config_path,
                &cfg.plaintext_passwords,
            );
        }
    }
    let state = crate::state::load_state(&config_path);
    app.last_update = state.last_update;
    app.host_updates = state.hosts;
    app.upgrade_started_at = state.upgrade_started_at;
    // Initialize vault if vault.bin exists
    app.vault = crate::vault::create_vault(&config_path).map(std::sync::Arc::new);
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
            () = resize_wait, if resize_at.is_some() => {
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

        if app.should_quit() {
            tasks.abort_all(&mut app);
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

/// Dispatch one key press.
///
/// Public so integration tests can drive the real key path rather than calling
/// the `App` methods it happens to reach today — the `u` flow is a sequence of
/// presses, and testing the pieces would not catch the sequence regressing.
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn handle_key(
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

    // While the biometric prompt is up, ignore other keys -- except the ones
    // that get the user out. The outcome normally arrives as a `VaultUnlocked` /
    // `VaultBiometricFailed` message, but if that task dies or hangs, every key
    // including quit was being swallowed and the app could only be killed.
    if app.vault_awaiting_biometric() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q' | 'Q')) {
            app.cancel_vault_biometric();
        }
        return;
    }

    // Same again while a password is being verified off-thread: show progress,
    // swallow stray keys, but never trap the user.
    if app.vault_verifying() {
        if matches!(key.code, KeyCode::Esc) {
            app.cancel_vault_verify();
        }
        return;
    }

    if app.show_upgrade_modal() {
        match key.code {
            KeyCode::Char('u' | 'U' | 'y' | 'Y') | KeyCode::Enter => {
                let cmds = app.confirm_upgrade();
                execute_cmds(cmds, app, servers, dims, tx, tasks);
            }
            KeyCode::Esc | KeyCode::Char('q' | 'Q' | 'n' | 'N') => {
                app.set_show_upgrade_modal(false);
            }
            _ => {}
        }
        return;
    }

    if app.vault_creating() {
        match key.code {
            KeyCode::Enter => {
                let master = std::mem::take(app.vault_password_input_mut());
                if master.is_empty() {
                    app.vault_state = VaultState::Creating {
                        error: Some("Master password cannot be empty".into()),
                    };
                    return;
                }
                let Some(path) = app.vault_path() else {
                    app.vault_state = VaultState::Creating {
                        error: Some("No config directory to create the vault in".into()),
                    };
                    return;
                };
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let vault = multitop_vault::Vault::new(multitop_vault::VaultConfig {
                        vault_path: path,
                        argon2_params: None,
                    });
                    let msg = match vault.initialize(&master).await {
                        // Unlock with the same password we just set, so the
                        // vault is immediately usable and can take the password
                        // whose save started all this.
                        Ok(()) => match vault.unlock_with_password(&master) {
                            Ok(unlocked) => Msg::VaultCreated(Box::new(unlocked)),
                            Err(e) => Msg::VaultCreateFailed(e.to_string()),
                        },
                        Err(e) => Msg::VaultCreateFailed(e.to_string()),
                    };
                    let _ = tx2.send(msg).await;
                });
            }
            KeyCode::Esc => {
                // Declining leaves the password in the OS credential store,
                // which still works; only the encrypted vault is skipped.
                app.vault_state = VaultState::Locked;
                app.vault_password_input_mut().clear();
            }
            KeyCode::Backspace => {
                app.vault_password_input_mut().pop();
            }
            KeyCode::Char(c) => app.vault_password_input_mut().push(c),
            _ => {}
        }
        return;
    }

    if app.show_vault_password_prompt() {
        match key.code {
            KeyCode::Enter => {
                let password = std::mem::take(app.vault_password_input_mut());
                if !password.is_empty() {
                    if let Some(vault) = app.vault.clone() {
                        // Argon2id is tuned to a quarter of system RAM, capped at
                        // 1 GiB, so unwrapping the key takes real time. Running it
                        // here froze the entire UI -- no redraw, no keys, no
                        // messages -- until it finished. Hand it to a blocking
                        // thread and let the result come back as a message.
                        app.set_vault_unlocking();
                        let tx2 = tx.clone();
                        tokio::task::spawn_blocking(move || {
                            let msg = match vault.unlock_with_password(&password) {
                                Ok(unlocked) => Msg::VaultUnlocked(unlocked),
                                Err(e) => Msg::VaultUnlockFailed(e.to_string()),
                            };
                            let _ = tx2.blocking_send(msg);
                        });
                    }
                }
            }
            KeyCode::Esc => {
                app.set_show_vault_password_prompt(false);
                app.vault_password_input_mut().clear();
                app.set_vault_password_error(None);
            }
            KeyCode::Backspace => {
                app.vault_password_input_mut().pop();
            }
            KeyCode::Char(c) => app.vault_password_input_mut().push(c),
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
        KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
            app.quit();
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
            return;
        }
        KeyCode::Char('e' | 'E') => {
            crate::passwords::open(app, app.selected_panel, false);
            return;
        }
        KeyCode::Char(c @ '1'..='9') => {
            let idx = ((c as usize) - ('1' as usize)).min(app.panels.len().saturating_sub(1));
            app.selected_panel = idx;
            return;
        }
        KeyCode::Char('c' | 'C') => {
            let old_sort = app.sort;
            app.sort = SortBy::Cpu;
            if old_sort != app.sort {
                restart_all_agents(app, servers, dims_rx, tx, tasks);
            }
            return;
        }
        KeyCode::Char('m' | 'M') => {
            let old_sort = app.sort;
            app.sort = SortBy::Mem;
            if old_sort != app.sort {
                restart_all_agents(app, servers, dims_rx, tx, tasks);
            }
            return;
        }
        KeyCode::Char('t' | 'T') => {
            app.cycle_theme();
            if let Some(ref path) = app.config_path {
                crate::config::save_theme(path, app.current_theme().name);
            }
            app.rerender_all(dims);
            return;
        }
        KeyCode::Up | KeyCode::Char('k' | 'K') => {
            app.scroll_up(1);
            return;
        }
        KeyCode::Down | KeyCode::Char('j' | 'J') => {
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
        KeyCode::Char('f' | 'F') => app.toggle_fetch(),
        KeyCode::Char('d' | 'D') => app.toggle_docker(),
        KeyCode::Char('s' | 'S') => app.switch_stats(),
        // `u` is deliberately two presses, and the rule does not depend on
        // whether an upgrade has run before:
        //
        //   not in the Upgrade view  ->  switch to it, change nothing else
        //   already in it            ->  start (vault, then confirm modal)
        //
        // The first press is always inert, so the user always sees each host's
        // command, history and credential state before anything can happen.
        KeyCode::Char('u' | 'U') => {
            // Switching *into* the view is always allowed, including while an
            // upgrade is running: the run continues in the background either
            // way, and being unable to look at it was the worst time not to be
            // able to. Only starting a new run is blocked while one is in
            // flight.
            if !app.in_upgrade() {
                app.enter_upgrade_view();
            } else if app.upgrades_in_flight() {
                // Already running — don't start another.
            } else if !app.upgrade_runnable() {
                // Every host lacks an upgrade_cmd. Confirming could only skip
                // all of them, so say so in the pane instead of opening a
                // modal that cannot do anything.
                app.note_nothing_to_upgrade();
            } else if let Some(vault) = app.begin_vault_unlock() {
                // Vault exists but is locked. Try Touch ID / fingerprint first;
                // the password prompt appears only if biometrics are unavailable
                // or cancelled (a `VaultBiometricFailed` message).
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    // Bounded: the UI blocks input while awaiting the result, so
                    // a Secure Enclave call that never returns would otherwise
                    // wedge the whole app.
                    let attempt =
                        tokio::time::timeout(BIOMETRIC_TIMEOUT, vault.unlock_biometric()).await;
                    let msg = match attempt {
                        Ok(Ok((unlocked, _))) => Msg::VaultUnlocked(unlocked),
                        Ok(Err(_)) | Err(_) => Msg::VaultBiometricFailed,
                    };
                    let _ = tx2.send(msg).await;
                });
            } else {
                app.set_show_upgrade_modal(true);
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
            Command::RunUpgrade { panel, gen } => {
                // Use the panel's stored sudo password (from keychain)
                let password = app.panels[panel].sudo_password.clone();
                (
                    panel,
                    crate::tasks::spawn_upgrade(
                        panel,
                        gen,
                        servers[panel].clone(),
                        password,
                        tx.clone(),
                    ),
                )
            }
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

#[allow(clippy::needless_pass_by_value)]
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
