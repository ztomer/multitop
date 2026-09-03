//! Key dispatch: turn a `KeyEvent` into state transitions and commands.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc::Sender;
use tokio::sync::watch;

use crate::app::{App, Command, Confirm, Msg};

use super::tasks::Tasks;
use multitop_agent::SortBy;

/// Dispatch one key press.
///
/// Public so integration tests can drive the real key path rather than calling
/// the `App` methods it happens to reach today — the `u` flow is a sequence of
/// presses, and testing the pieces would not catch the sequence regressing.
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn handle_key(
    key: KeyEvent,
    app: &mut App,
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

    // Help overlay — `?` from anywhere, `Esc`/`?`/`q` to close. Checked before
    // the confirm modal so help can be summoned even with a quit armed, and
    // before the filter so `?` in the filter query doesn't open it.
    if app.help_visible {
        if matches!(
            key.code,
            KeyCode::Char('?' | 'h' | 'H' | 'q' | 'Q') | KeyCode::Esc
        ) {
            app.help_visible = false;
        }
        return;
    }
    if matches!(key.code, KeyCode::Char('?')) {
        app.help_visible = true;
        return;
    }

    // Command palette — `:` from anywhere, `Esc` to close, `Enter` to execute.
    if app.command_palette_visible {
        match key.code {
            KeyCode::Esc => {
                app.command_palette_visible = false;
                app.command_input.clear();
                return;
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut app.command_input);
                app.command_palette_visible = false;
                execute_palette_command(&input, app, dims, tx, tasks);
                return;
            }
            KeyCode::Backspace => {
                app.command_input.pop();
                return;
            }
            KeyCode::Char(c) => {
                app.command_input.push(c);
                return;
            }
            _ => return,
        }
    }
    if matches!(key.code, KeyCode::Char(':')) {
        app.command_palette_visible = true;
        app.command_input.clear();
        return;
    }

    // Which confirmation is in force is `App::active_confirm`'s answer, not a
    // second copy of the priority. This used to test `show_upgrade_modal` first
    // while `ui::keybar_content` tested `quit_armed` first, so with both set the
    // screen named one set of keys and this ran the other -- and the one that
    // lost was the confirmation guarding a running dpkg.
    match app.active_confirm() {
        // A quit armed by Esc/q/Ctrl-C while upgrades were in flight. `q`
        // confirms, Esc stands down. Every other key is ignored until one of the
        // two: the row is a modal in all but shape, and letting stray keys
        // through while it is up would be acting on a screen the user has asked
        // a question of.
        Some(Confirm::Quit) => {
            match key.code {
                // Only the keys the row names, plus Ctrl-C, which means the same
                // thing everywhere. `Enter` and `y` used to confirm too, and they
                // are exactly the wrong keys to accept here: this press kills a
                // running dpkg transaction on N production hosts, and `Enter` is
                // what an operator hits to dismiss something they have not read.
                KeyCode::Char('q' | 'Q') => app.quit(),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit(),
                KeyCode::Esc => app.cancel_quit(),
                _ => {}
            }
            return;
        }
        Some(Confirm::Kill) => {
            match key.code {
                // Same discipline as Upgrade: only the advertised key kills.
                // `Enter` is what an operator hits to dismiss, not to authorize
                // `kill -9` on a production pid.
                KeyCode::Char('k' | 'K' | 'x' | 'X') => {
                    if let Some(ec) = app.kill_confirm.take() {
                        if ec.kind == crate::app::ExecKind::Kill && ec.panel < app.panels.len() {
                            let gen = app.bump(ec.panel);
                            let server = app.panels[ec.panel].server.clone();
                            let pass = app.panels[ec.panel].sudo_password.clone();
                            let handle = crate::tasks::spawn_kill(
                                ec.panel,
                                gen,
                                server,
                                ec.pid,
                                ec.name,
                                pass,
                                tx.clone(),
                            );
                            tasks.set_aux(ec.panel, handle);
                        } else {
                            // Wrong key for armed action — re-arm.
                            app.kill_confirm = Some(ec);
                        }
                    }
                }
                KeyCode::Char('o' | 'O') => {
                    if let Some(ec) = app.kill_confirm.take() {
                        if ec.kind == crate::app::ExecKind::Journal && ec.panel < app.panels.len() {
                            let gen = app.bump(ec.panel);
                            let server = app.panels[ec.panel].server.clone();
                            let pass = app.panels[ec.panel].sudo_password.clone();
                            let handle = crate::tasks::spawn_journal(
                                ec.panel,
                                gen,
                                server,
                                ec.pid,
                                ec.name,
                                pass,
                                tx.clone(),
                            );
                            tasks.set_aux(ec.panel, handle);
                        } else {
                            app.kill_confirm = Some(ec);
                        }
                    }
                }
                KeyCode::Char('r' | 'R') => {
                    if let Some(ec) = app.kill_confirm.take() {
                        if ec.kind == crate::app::ExecKind::Renice && ec.panel < app.panels.len() {
                            let gen = app.bump(ec.panel);
                            let server = app.panels[ec.panel].server.clone();
                            let pass = app.panels[ec.panel].sudo_password.clone();
                            let handle = crate::tasks::spawn_renice(
                                ec.panel,
                                gen,
                                server,
                                ec.pid,
                                ec.name,
                                pass,
                                tx.clone(),
                            );
                            tasks.set_aux(ec.panel, handle);
                        } else {
                            app.kill_confirm = Some(ec);
                        }
                    }
                }
                KeyCode::Esc | KeyCode::Char('q' | 'Q' | 'n' | 'N') => {
                    app.kill_confirm = None;
                }
                _ => {}
            }
            return;
        }
        Some(Confirm::Upgrade) => {
            match key.code {
                // Only the key the row names. It reads `[U] go  [Esc] cancel`, and
                // `y`, `Y` and `Enter` confirmed as well -- three keys that start
                // `apt upgrade` on every visible host without appearing anywhere on
                // the screen that asked. `Enter` is the worst of them: it is what an
                // operator hits to dismiss a row they have not read, which is the
                // reason the quit confirmation dropped it, and the reason the server
                // removal dropped it in this same pass.
                //
                // Extra *cancel* keys below are not the same thing and stay: a stray
                // key that cancels can only ever be the safe answer.
                KeyCode::Char('u' | 'U') => {
                    if app.any_password_checking() {
                        // A credential-store lookup is still in flight (it can
                        // block on a system dialog). Starting now would run on
                        // passwords the app has not actually read; the header
                        // says `Checking` and this press is deferred until the
                        // last answer lands.
                        return;
                    }
                    let cmds = app.confirm_upgrade();
                    execute_cmds(cmds, app, dims, tx, tasks);
                }
                KeyCode::Esc | KeyCode::Char('q' | 'Q' | 'n' | 'N') => {
                    app.set_show_upgrade_modal(false);
                }
                KeyCode::Char('s' | 'S') => {
                    app.set_show_upgrade_modal(false);
                    let cmds = app.switch_stats();
                    execute_cmds(cmds, app, dims, tx, tasks);
                }
                KeyCode::Char('d' | 'D') => {
                    app.set_show_upgrade_modal(false);
                    let cmds = app.toggle_docker(dims);
                    execute_cmds(cmds, app, dims, tx, tasks);
                }
                KeyCode::Char('f' | 'F') => {
                    app.set_show_upgrade_modal(false);
                    let cmds = app.toggle_fetch(dims);
                    execute_cmds(cmds, app, dims, tx, tasks);
                }
                KeyCode::Char('g' | 'G') => {
                    app.set_show_upgrade_modal(false);
                    let cmds = app.toggle_graphs(dims);
                    execute_cmds(cmds, app, dims, tx, tasks);
                }
                _ => {}
            }
            return;
        }
        None => {}
    }

    if app.vault_creating() {
        // While the creation is in flight the prompt is a progress message, not
        // a field: Argon2id is running and there is nothing to type into. Only
        // Esc, which gives up on waiting, still means anything.
        if app.vault_create_in_flight() {
            if matches!(key.code, KeyCode::Esc) {
                app.cancel_vault_creation();
            }
            return;
        }
        match key.code {
            KeyCode::Enter => {
                let Some(master) = app.begin_vault_create_attempt() else {
                    return;
                };
                let epoch = app.vault_epoch;
                let Some(path) = app.vault_path() else {
                    app.fail_vault_creation("No config directory to create the vault in".into());
                    return;
                };
                let tx2 = tx.clone();
                let vault_config = crate::vault::config_for(path);
                tokio::spawn(async move {
                    let vault = multitop_vault::Vault::new(vault_config);
                    let msg = match vault.initialize(&master).await {
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
        return;
    }

    if app.password_manager.is_some() {
        let action = crate::passwords::handle_key(app, key.code);
        crate::password_actions::apply(action, app, tx, tasks);
        return;
    }

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
            return;
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
        return;
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
        return;
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
            return;
        }
    }

    match key.code {
        // Focus first — `Esc` while zoomed should unzoom, not clear the filter
        // underneath or quit. The focused host is the one the user asked to see
        // alone, and the filter they left behind is still there when they return.
        KeyCode::Esc if app.is_focused() => {
            app.toggle_focus();
            app.rerender_all(dims);
            return;
        }
        // Esc clears an applied filter before it quits. Quitting on the same
        // key that got you here reads as the app dying, and the panels are
        // already hidden, so there is nothing on screen to explain it.
        KeyCode::Esc if !app.filter_query.trim().is_empty() => {
            app.filter_query.clear();
            clamp_selection_to_filter(app);
            app.persist_state();
            return;
        }
        KeyCode::Char('/') => {
            app.set_filtering(true);
            app.filter_query.clear();
            return;
        }
        KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
            app.request_quit();
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.request_quit();
            return;
        }
        KeyCode::Char('e' | 'E') => {
            let load = crate::passwords::open(app, app.selected_panel, false);
            app.dispatch_credential_loads(load, tx);
            return;
        }
        // The number keys count panes on screen, not entries in the config.
        //
        // They used to index the unfiltered list and clamp to its end, so with
        // `/db` showing one pane, `2` selected a host that was not on screen
        // and every view key after it acted on that host instead. Out of range
        // now does nothing, which is the same answer a click on no pane gets:
        // the two ways of choosing a pane agree.
        KeyCode::Char(c @ '1'..='9') => {
            let slot = (c as usize) - ('1' as usize);
            if let Some(&panel) = app.filtered_indices().get(slot) {
                app.selected_panel = panel;
                app.persist_state();
            }
            return;
        }
        KeyCode::Char('c' | 'C') => {
            let old_sort = app.sort;
            app.sort = SortBy::Cpu;
            if old_sort != app.sort {
                app.persist_state();
                super::event_loop::restart_all_agents(app, dims_rx, tx, tasks);
            }
            return;
        }
        KeyCode::Char('m' | 'M') => {
            let old_sort = app.sort;
            app.sort = SortBy::Mem;
            if old_sort != app.sort {
                app.persist_state();
                super::event_loop::restart_all_agents(app, dims_rx, tx, tasks);
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
        KeyCode::Char('+' | '=') => {
            if app.in_graphs() || app.in_alerts() {
                // 4096 samples ≈2.2 h; 80 cols*2*16=2560 ≈1.4 h, 1..16 covers
                // ~10 s to ~1 h at 80 cols, validated against bench.
                app.graph_zoom = (app.graph_zoom + 1).clamp(1, 16);
                app.rerender_all(dims);
            }
            return;
        }
        KeyCode::Char('-' | '_') => {
            if app.in_graphs() || app.in_alerts() {
                app.graph_zoom = app.graph_zoom.saturating_sub(1).max(1);
                app.rerender_all(dims);
            }
            return;
        }
        KeyCode::Char('y' | 'Y') => {
            yank_selected_host(app);
            return;
        }
        KeyCode::Char('H') if !app.is_filtering() => {
            let cmds = app.toggle_alerts(dims);
            for cmd in cmds {
                let _ = cmd;
            }
            app.persist_state();
            return;
        }
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
            return;
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
            return;
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
            return;
        }
        KeyCode::Char('l' | 'L') => {
            // `tail -n 200 -F /var/log/syslog` as framed Exec — Painter+RingLines reuse.
            if app.selected_panel < app.panels.len() {
                let panel = app.selected_panel;
                let gen = app.bump(panel);
                let server = app.panels[panel].server.clone();
                let pass = app.panels[panel].sudo_password.clone();
                let handle = crate::tasks::spawn_tail(panel, gen, server, pass, tx.clone());
                tasks.set_aux(panel, handle);
            }
            return;
        }
        KeyCode::Enter | KeyCode::Char('z' | 'Z') => {
            app.toggle_focus();
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
            app.scroll_to_bottom();
            return;
        }
        _ => {}
    }

    let cmds = match key.code {
        KeyCode::Char('f' | 'F') => app.toggle_fetch(dims),
        KeyCode::Char('d' | 'D') => app.toggle_docker(dims),
        KeyCode::Char('g' | 'G') => app.toggle_graphs(dims),
        KeyCode::Char('h' | 'H') => app.toggle_alerts(dims),
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
            KeyCode::Char('f' | 'F' | 'd' | 'D' | 'g' | 'G' | 'h' | 'H' | 's' | 'S' | 'u' | 'U')
        )
    {
        app.persist_state();
    }

    execute_cmds(cmds, app, dims, tx, tasks);
}

/// Carry out the commands an `App` method produced.
pub fn execute_cmds(
    cmds: Vec<Command>,
    app: &App,
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    for cmd in cmds {
        // A view task supersedes the last view task; an upgrade goes in the
        // upgrade slot and outlives every view switch. Which slot it lands in
        // is decided here, once, rather than by a flag each caller has to keep
        // in step.
        match cmd {
            Command::RunFetch { panel, gen } => tasks.set_aux(
                panel,
                crate::tasks::spawn_fetch(
                    panel,
                    gen,
                    app.panels_epoch,
                    app.panels[panel].server.clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                ),
            ),
            Command::RunDocker { panel, gen } => tasks.set_aux(
                panel,
                crate::tasks::spawn_docker(
                    panel,
                    gen,
                    app.panels_epoch,
                    app.panels[panel].server.clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                ),
            ),
            Command::RunUpgrade { panel, gen } => {
                // Use the panel's stored sudo password (from keychain)
                let password = app.panels[panel].sudo_password.clone();
                tasks.set_upgrade(
                    panel,
                    crate::tasks::spawn_upgrade(
                        panel,
                        gen,
                        app.panels[panel].server.clone(),
                        password,
                        tx.clone(),
                    ),
                );
            }
        }
    }
}

fn execute_palette_command(
    input: &str,
    app: &mut App,
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    let input = input.trim().to_lowercase();
    if let Some(stripped) = input.strip_prefix("filter ") {
        app.filter_query = stripped.to_string();
        clamp_selection_to_filter(app);
        app.persist_state();
    } else if input == "filter" || input == "clear filter" {
        app.filter_query.clear();
        clamp_selection_to_filter(app);
        app.persist_state();
    } else if input.starts_with("upgrade") {
        let loads = app.enter_upgrade_view();
        app.dispatch_credential_loads(loads, tx);
    } else if input == "docker" {
        let cmds = app.toggle_docker(dims);
        for cmd in cmds {
            if let crate::types::Command::RunDocker { panel, gen } = cmd {
                tasks.set_aux(
                    panel,
                    crate::tasks::spawn_docker(
                        panel,
                        gen,
                        app.panels_epoch,
                        app.panels[panel].server.clone(),
                        dims,
                        app.sort,
                        tx.clone(),
                    ),
                );
            }
        }
        app.persist_state();
    } else if input == "fetch" {
        let cmds = app.toggle_fetch(dims);
        for cmd in cmds {
            if let crate::types::Command::RunFetch { panel, gen } = cmd {
                tasks.set_aux(
                    panel,
                    crate::tasks::spawn_fetch(
                        panel,
                        gen,
                        app.panels_epoch,
                        app.panels[panel].server.clone(),
                        dims,
                        app.sort,
                        tx.clone(),
                    ),
                );
            }
        }
        app.persist_state();
    } else if input == "graphs" || input == "graph" {
        let cmds = app.toggle_graphs(dims);
        for cmd in cmds {
            // toggle_graphs returns empty, just rerenders
            let _ = cmd;
        }
        app.persist_state();
    } else if input == "stats" || input == "s" {
        let cmds = app.switch_stats();
        for cmd in cmds {
            let _ = cmd;
        }
        app.persist_state();
    } else if input.starts_with("sort ") {
        let old = app.sort;
        if input.contains("mem") {
            app.sort = SortBy::Mem;
        } else {
            app.sort = SortBy::Cpu;
        }
        if old != app.sort {
            app.persist_state();
        }
    } else if input == "theme" || input.starts_with("theme ") {
        app.cycle_theme();
        if let Some(ref path) = app.config_path {
            crate::config::save_theme(path, app.current_theme().name);
        }
        app.rerender_all(dims);
    } else if input == "add server" || input == "add" {
        let load = crate::passwords::open(app, app.selected_panel, true);
        app.dispatch_credential_loads(load, tx);
    } else if input == "vault unlock" {
        if let Some((vault, epoch)) = app.begin_vault_unlock() {
            drop(crate::run::spawn::spawn_biometric_unlock(
                vault,
                epoch,
                tx.clone(),
            ));
        } else if app.show_vault_password_prompt() {
            // already prompting
        } else {
            app.set_show_upgrade_modal(true);
        }
    } else if input == "yank" || input.starts_with("yank ") || input == "y" || input == "copy" {
        yank_selected_host(app);
    }
}

fn yank_selected_host(app: &App) {
    let Some(panel) = app.panels.get(app.selected_panel) else {
        return;
    };
    let target = panel.server.target();
    let text = target.as_ref().to_string();
    // Try pbcopy (macOS) then xclip/xsel (Linux), best effort.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            });
    }
    #[cfg(target_os = "linux")]
    {
        for prog in ["xclip", "xsel"] {
            let mut cmd = std::process::Command::new(prog);
            if prog == "xclip" {
                cmd.args(["-selection", "clipboard"]);
            }
            if let Ok(mut child) = cmd.stdin(std::process::Stdio::piped()).spawn() {
                use std::io::Write as _;
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                break;
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
    }
}

/// Keep the selection on a panel the user can actually see.
///
/// The selected panel drives the keybar's mode badge and every view-switching
/// key. Left pointing at a filtered-out host, those keys act on a panel that is
/// not on screen.
fn clamp_selection_to_filter(app: &mut App) {
    let shown = app.filtered_indices();
    if !shown.is_empty() && !shown.contains(&app.selected_panel) {
        app.selected_panel = shown[0];
    }
}
