//! Keys while a confirmation is up: upgrade, quit, and kill/renice/journal.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::Sender;

use crate::app::{App, Confirm, Msg};

use super::commands::execute_cmds;
use super::tasks::Tasks;

/// The confirmation in force (quit, upgrade, ...) owns every key while up.
pub(super) fn confirmation_keys(
    key: KeyEvent,
    app: &mut App,
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) -> bool {
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
            return true;
        }
        Some(Confirm::Kill) => {
            kill_confirm_keys(key, app, tx, tasks);
            return true;
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
                        return true;
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
            return true;
        }
        None => {}
    }

    false
}

/// The keys while a kill is awaiting confirmation: only the advertised key
/// kills, Esc stands down, everything else is swallowed.
pub(super) fn kill_confirm_keys(key: KeyEvent, app: &mut App, tx: &Sender<Msg>, tasks: &mut Tasks) {
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
}
