//! Key dispatch: turn a `KeyEvent` into state transitions and commands.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc::Sender;
use tokio::sync::watch;

use crate::app::Overlay;
use crate::app::{App, Msg};

use super::commands::execute_cmds;
use super::confirm_keys::confirmation_keys;
use super::nav_keys::{clamp_selection_to_filter, focus_and_navigation_keys};
use super::palette::execute_palette_command;
use super::tasks::Tasks;
use multitop_agent::SortBy;

/// Dispatch one key press.
///
/// Public so integration tests can drive the real key path rather than calling
/// the `App` methods it happens to reach today — the `u` flow is a sequence of
/// presses, and testing the pieces would not catch the sequence regressing.
pub fn handle_key(
    key: KeyEvent,
    app: &mut App,
    dims: (u16, u16),
    dims_rx: &Arc<watch::Receiver<(u16, u16)>>,
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

    if overlay_keys(key, app, dims, tx, tasks) {
        return;
    }
    if confirmation_keys(key, app, dims, tx, tasks) {
        return;
    }
    if vault_and_password_keys(key, app, tx, tasks) {
        return;
    }
    if filter_keys(key, app) {
        return;
    }
    if focus_and_navigation_keys(key, app, dims, dims_rx, tx, tasks) {
        return;
    }
    let cmds = match key.code {
        KeyCode::Char('f' | 'F') => app.toggle_fetch(dims),
        KeyCode::Char('d' | 'D') => app.toggle_docker(dims),
        KeyCode::Char('g' | 'G') => app.toggle_graphs(dims),
        KeyCode::Char('h' | 'H') => app.toggle_alerts(dims),
        KeyCode::Char('p' | 'P') => app.toggle_ops(dims),
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
                let loads = app.enter_upgrade_view();
                app.dispatch_credential_loads(loads, tx);
                for i in app.filtered_indices() {
                    if app.panels[i].upgradable.is_none()
                        && app.panels[i].server.upgrade_cmd.is_some()
                    {
                        let _ = crate::tasks::spawn_upgradable_check(
                            i,
                            app.panels[i].gen,
                            app.panels[i].server.clone(),
                            tx.clone(),
                        );
                    }
                }
            } else if app.upgrades_in_flight() {
                // Already running — don't start another.
            } else if !app.upgrade_runnable() {
                // Every host lacks an upgrade_cmd. Confirming could only skip
                // all of them, so say so in the pane instead of opening a
                // modal that cannot do anything.
                app.note_nothing_to_upgrade();
            } else if let Some((vault, epoch)) = app.begin_vault_unlock() {
                // The vault is locked and this machine can open it with one
                // touch. The Touch ID prompt is the whole interaction; if it is
                // refused or the sensor is unavailable, `VaultBiometricFailed`
                // falls back to the master password. One prompt either way.
                //
                // The handle is not kept. There is nothing to abort it with that
                // the epoch does not already do: `Esc` retires this attempt, so
                // whatever the sensor eventually says arrives stamped with a
                // dead epoch and is dropped. A task waiting on a system prompt
                // cannot be cancelled from here anyway.
                drop(super::spawn::spawn_biometric_unlock(
                    vault,
                    epoch,
                    tx.clone(),
                ));
            } else if app.show_vault_password_prompt() {
                // Locked, but not by touch on this machine: the master password
                // prompt is up and there is nothing more to start.
            } else {
                app.set_show_upgrade_modal(true);
            }
            Vec::new()
        }
        _ => return,
    };

    // View per host, sort, and filter are now per-panel state that survives
    // restarts, so any view switch persists the new layout.
    if !cmds.is_empty()
        || matches!(
            key.code,
            KeyCode::Char(
                'f' | 'F' | 'd' | 'D' | 'g' | 'G' | 'h' | 'H' | 'p' | 'P' | 's' | 'S' | 'u' | 'U'
            )
        )
    {
        app.persist_state();
    }

    execute_cmds(cmds, app, dims, tx, tasks);
}

/// Help and the command palette: the overlays own every key while up.
fn overlay_keys(
    key: KeyEvent,
    app: &mut App,
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) -> bool {
    // Help overlay — `?` from anywhere, `Esc`/`?`/`q` to close. Checked before
    // the confirm modal so help can be summoned even with a quit armed, and
    // before the filter so `?` in the filter query doesn't open it.
    if let Overlay::Help { palette_open } = app.overlay {
        if matches!(
            key.code,
            KeyCode::Char('?' | 'h' | 'H' | 'q' | 'Q') | KeyCode::Esc
        ) {
            app.overlay = if palette_open {
                Overlay::CommandPalette
            } else {
                Overlay::None
            };
        }
        return true;
    }
    if matches!(key.code, KeyCode::Char('?')) {
        app.overlay = Overlay::Help {
            palette_open: app.overlay.is_palette(),
        };
        return true;
    }

    // Command palette — `:` from anywhere, `Esc` to close, `Enter` to execute.
    if app.overlay.is_palette() {
        match key.code {
            KeyCode::Esc => {
                app.overlay = Overlay::None;
                app.command_input.clear();
                return true;
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut app.command_input);
                app.overlay = Overlay::None;
                execute_palette_command(&input, app, dims, tx, tasks);
                return true;
            }
            KeyCode::Backspace => {
                app.command_input.pop();
                return true;
            }
            KeyCode::Char(c) => {
                app.command_input.push(c);
                return true;
            }
            _ => return true,
        }
    }
    if matches!(key.code, KeyCode::Char(':')) {
        app.overlay = Overlay::CommandPalette;
        app.command_input.clear();
        return true;
    }

    false
}

/// Vault creation, the vault password prompt and the password manager own every key while up.
fn vault_and_password_keys(
    key: KeyEvent,
    app: &mut App,
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) -> bool {
    if app.vault_creating() {
        // While the creation is in flight the prompt is a progress message, not
        // a field: Argon2id is running and there is nothing to type into. Only
        // Esc, which gives up on waiting, still means anything.
        if app.vault_create_in_flight() {
            if matches!(key.code, KeyCode::Esc) {
                app.cancel_vault_creation();
            }
            return true;
        }
        match key.code {
            KeyCode::Enter => {
                let Some(master) = app.begin_vault_create_attempt() else {
                    return true;
                };
                let epoch = app.vault_epoch;
                let Some(path) = app.vault_path() else {
                    app.fail_vault_creation("No config directory to create the vault in".into());
                    return true;
                };
                let tx2 = tx.clone();
                let vault_config = crate::vault::config_for(path);
                tokio::spawn(async move {
                    let vault = multitop_vault::Vault::new(vault_config);
                    let msg = match vault.initialize(&master) {
                        // Unlock with the same password we just set, so the
                        // vault is immediately usable and can take the password
                        // whose save started all this.
                        Ok(()) => match vault.unlock_with_password(&master) {
                            Ok(unlocked) => Msg::VaultCreated {
                                epoch,
                                unlocked: Box::new(unlocked),
                            },
                            Err(e) => Msg::VaultCreateFailed {
                                epoch,
                                error: e.to_string(),
                            },
                        },
                        Err(e) => Msg::VaultCreateFailed {
                            epoch,
                            error: e.to_string(),
                        },
                    };
                    let _ = tx2.send(msg).await;
                });
            }
            KeyCode::Esc => {
                // Declining leaves the password in the OS credential store,
                // which still works; only the encrypted vault is skipped.
                app.cancel_vault_creation();
            }
            KeyCode::Backspace => {
                app.vault_password_input_mut().pop();
            }
            KeyCode::Char(c) => app.vault_password_input_mut().push(c),
            _ => {}
        }
        return true;
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
                        let epoch = app.set_vault_unlocking();
                        let tx2 = tx.clone();
                        tokio::task::spawn_blocking(move || {
                            let msg = match vault.unlock_with_password(&password) {
                                Ok(unlocked) => Msg::VaultUnlocked {
                                    epoch,
                                    unlocked: Box::new(unlocked),
                                },
                                Err(e) => Msg::VaultUnlockFailed {
                                    epoch,
                                    error: e.to_string(),
                                },
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
        return true;
    }

    if app.password_manager.is_some() {
        let action = crate::passwords::handle_key(app, key.code);
        crate::password_actions::apply(action, app, tx, tasks);
        return true;
    }

    false
}

/// The filter query, and the Ctrl-S / Ctrl-1..3 saved-filter bindings.
fn filter_keys(key: KeyEvent, app: &mut App) -> bool {
    // Typing a query owns every printable key, so this is checked before the
    // single-letter bindings below -- otherwise a host called "docker" could
    // not be typed without switching views half way through.
    if app.is_filtering() {
        // Ctrl-S saves the current query into the 1..3 slots.
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let q = app.filter_query.trim().to_string();
            if !q.is_empty() && !app.saved_filters.contains(&q) {
                if app.saved_filters.len() >= 3 {
                    app.saved_filters.remove(0);
                }
                app.saved_filters.push(q);
                app.persist_state();
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                app.filter_query.clear();
                app.set_filtering(false);
            }
            // Keep what was typed and hand the keys back. Clearing here instead
            // would make the feature useless: the filter would only ever exist
            // while a key was held down.
            KeyCode::Enter => app.set_filtering(false),
            KeyCode::Backspace => {
                app.filter_query.pop();
            }
            KeyCode::Char(c) => app.filter_query.push(c),
            _ => {}
        }
        clamp_selection_to_filter(app);
        app.persist_state();
        return true;
    }
    // Ctrl-S outside filtering also saves the applied query.
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let q = app.filter_query.trim().to_string();
        if !q.is_empty() && !app.saved_filters.contains(&q) {
            if app.saved_filters.len() >= 3 {
                app.saved_filters.remove(0);
            }
            app.saved_filters.push(q);
            app.persist_state();
        }
        return true;
    }
    // Ctrl-1..3 recalls a saved filter (1 is most recent when only one).
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c @ '1'..='3') = key.code {
            let idx = (c as usize) - ('1' as usize);
            if let Some(q) = app.saved_filters.get(idx).cloned() {
                app.filter_query = q;
                clamp_selection_to_filter(app);
                app.persist_state();
            }
            return true;
        }
    }

    false
}

/// The per-process actions on the selected host's top process (`x`, `o`,
/// `r`): each arms a confirmation or dispatches its own command, and the
/// key is consumed either way.
pub(super) fn process_keys(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('x' | 'X') => {
            // Top process on the selected host, per current sort, as `host:pid:name`
            // guarded by the same Confirm pattern as Upgrade (`Confirm::Kill`).
            if let Some(panel) = app.panels.get(app.selected_panel) {
                if let Some(multitop_agent::proto::Payload::Monitor(snap)) = &panel.last_monitor {
                    let mut procs = snap.procs.clone();
                    match app.sort {
                        SortBy::Cpu => procs.sort_by(|a, b| {
                            b.cpu
                                .partial_cmp(&a.cpu)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        }),
                        SortBy::Mem => procs.sort_by_key(|a| std::cmp::Reverse(a.mem)),
                    }
                    if let Some(top) = procs.first() {
                        app.kill_confirm = Some(crate::app::ExecConfirm {
                            panel: app.selected_panel,
                            pid: top.pid,
                            name: top.name.clone(),
                            kind: crate::app::ExecKind::Kill,
                        });
                    }
                }
            }
        }
        KeyCode::Char('o' | 'O') => {
            if let Some(panel) = app.panels.get(app.selected_panel) {
                if let Some(multitop_agent::proto::Payload::Monitor(snap)) = &panel.last_monitor {
                    let mut procs = snap.procs.clone();
                    match app.sort {
                        SortBy::Cpu => procs.sort_by(|a, b| {
                            b.cpu
                                .partial_cmp(&a.cpu)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        }),
                        SortBy::Mem => procs.sort_by_key(|a| std::cmp::Reverse(a.mem)),
                    }
                    if let Some(top) = procs.first() {
                        app.kill_confirm = Some(crate::app::ExecConfirm {
                            panel: app.selected_panel,
                            pid: top.pid,
                            name: top.name.clone(),
                            kind: crate::app::ExecKind::Journal,
                        });
                    }
                }
            }
        }
        KeyCode::Char('r' | 'R') => {
            if let Some(panel) = app.panels.get(app.selected_panel) {
                if let Some(multitop_agent::proto::Payload::Monitor(snap)) = &panel.last_monitor {
                    let mut procs = snap.procs.clone();
                    match app.sort {
                        SortBy::Cpu => procs.sort_by(|a, b| {
                            b.cpu
                                .partial_cmp(&a.cpu)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        }),
                        SortBy::Mem => procs.sort_by_key(|a| std::cmp::Reverse(a.mem)),
                    }
                    if let Some(top) = procs.first() {
                        app.kill_confirm = Some(crate::app::ExecConfirm {
                            panel: app.selected_panel,
                            pid: top.pid,
                            name: top.name.clone(),
                            kind: crate::app::ExecKind::Renice,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}
