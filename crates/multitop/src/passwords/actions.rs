use crate::app::App;
use crate::config::Server;
use crate::passwords::types::{PasswordAction, PasswordEdit, PasswordManager, ServerDraft};
use crossterm::event::KeyCode;
use secrecy::{ExposeSecret, SecretString};

pub fn open(app: &mut App, selected: usize, resume_upgrade: bool) -> Vec<(usize, Server)> {
    if !app.panels.is_empty() {
        let idx = selected.min(app.panels.len() - 1);
        app.password_manager = Some(PasswordManager::new(idx, resume_upgrade));
        // The selected host's credential-state row reads from the store; the
        // lookup is dispatched off the loop thread like every other store read,
        // and the manager paints whatever the panel knows until it lands.
        return app
            .panels
            .get(idx)
            .filter(|p| p.needs_credential_load())
            .map(|p| (idx, p.server.clone()))
            .into_iter()
            .collect();
    }
    Vec::new()
}

/// One list, one set of keys.
///
/// This was two sections with `Tab` between them: a Passwords list and a
/// Servers list, showing the same hosts with different columns and different
/// meanings for the same key -- `d` deleted a password on one side and a server
/// on the other. The password for a host is a property of that host, so it is
/// edited where the host is edited, and there is nothing left to switch between.
pub fn handle_key(app: &mut App, key: KeyCode) -> PasswordAction {
    if app.password_manager.is_none() {
        return PasswordAction::None;
    }
    row_key(app, key)
}

/// Resolve a removal the user has been asked to confirm.
///
/// Anything other than an explicit yes cancels, so a stray keystroke can only
/// ever be the safe answer.
#[allow(clippy::expect_used)]
fn answer_pending_delete(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    let Some(idx) = manager.pending_delete.take() else {
        return PasswordAction::None;
    };
    // Only the key the question names. `Enter` used to confirm as well, and it
    // is exactly the wrong key to accept here for two reasons.
    //
    // It is not offered: the prompt reads `[y] confirm  [Esc] cancel`, so a
    // confirmation on `Enter` is a key that acts without being advertised.
    //
    // And it is *this panel's own key for opening a row to edit it*. `d` then
    // `Enter` -- press `d` to see what the question says, then the key you use
    // to work on a row -- removed the host and, through `write_servers`,
    // aborted any upgrade running on it: a `dpkg` transaction interrupted on a
    // real machine by two keystrokes that never meant to.
    //
    // The quit confirmation dropped `Enter` for the same reason earlier in this
    // round -- "`Enter` is what an operator hits to dismiss something they have
    // not read" -- and this is that instance's surviving sibling.
    let confirmed = matches!(key, KeyCode::Char('y' | 'Y'));
    if !confirmed || idx >= app.panels.len() {
        if let Some(m) = app.password_manager.as_mut() {
            m.notice = Some("Removal cancelled.".to_string());
        }
        return PasswordAction::None;
    }

    let host = app.panels[idx].server.host.clone();
    let servers: Vec<Server> = app
        .panels
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, panel)| panel.server.clone())
        .collect();
    if let Some(m) = app.password_manager.as_mut() {
        m.notice = Some(format!("Removed {host}."));
    }
    PasswordAction::ApplyServers(servers)
}

/// Keys while a text prompt is open. The prompt owns every printable key.
#[allow(clippy::expect_used)]
fn prompt_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    match key {
        KeyCode::Esc => {
            manager.edit = None;
            manager.input.clear();
            manager.notice = None;
        }
        KeyCode::Enter => {
            let typed = std::mem::take(&mut manager.input);
            let stage = manager.edit.take();
            if typed.is_empty() {
                manager.notice = Some("Password was not changed.".to_string());
                return PasswordAction::None;
            }
            match stage {
                Some(PasswordEdit::RotateCurrent) => {
                    // Not checked here: verifying it means running the KDF,
                    // which belongs off the event loop. The rotation attempt
                    // does it once and reports back, rather than paying for it
                    // twice and freezing the UI to say something the user
                    // learns a moment later anyway.
                    manager.edit = Some(PasswordEdit::RotateNew {
                        current: SecretString::from(typed),
                    });
                    manager.notice = Some("Enter the NEW master password:".to_string());
                }
                Some(PasswordEdit::RotateNew { current }) => {
                    return PasswordAction::RotateVaultPassword {
                        current: current.expose_secret().to_string(),
                        new: typed,
                    };
                }
                None => {}
            }
        }
        KeyCode::Backspace => {
            manager.input.pop();
        }
        KeyCode::Char(character) => manager.input.push(character),
        _ => {}
    }
    PasswordAction::None
}

/// Keys while a server row is open for editing.
#[allow(clippy::expect_used)]
fn draft_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    let Some(draft) = manager.draft.as_mut() else {
        return PasswordAction::None;
    };
    match key {
        KeyCode::Esc => manager.draft = None,
        KeyCode::Tab | KeyCode::Down => draft.field = (draft.field + 1) % 5,
        KeyCode::Up => draft.field = draft.field.checked_sub(1).unwrap_or(4),
        KeyCode::Backspace => {
            draft.active_field().pop();
        }
        KeyCode::Char(character) => draft.active_field().push(character),
        KeyCode::Enter => {
            let draft = manager.draft.take().expect("draft exists");
            let typed = draft.password.clone();
            let original_idx = draft.original;
            match draft.clone().into_server() {
                Ok(server) => {
                    let mut servers: Vec<Server> = app
                        .panels
                        .iter()
                        .map(|panel| panel.server.clone())
                        .collect();
                    let target_idx = if let Some(index) = original_idx {
                        servers[index] = server;
                        index
                    } else {
                        servers.push(server);
                        servers.len() - 1
                    };
                    return PasswordAction::ApplyServerEdit {
                        servers,
                        target_idx,
                        // An emptied password field means "this host has no
                        // password of its own". Keeping the old one would
                        // contradict what the editor showed when it was saved,
                        // and there is no other way to take one back now that
                        // the separate Passwords list is gone.
                        password: (!typed.trim().is_empty()).then_some(typed),
                    };
                }
                Err(error) => {
                    manager.notice = Some(error);
                    manager.draft = Some(draft);
                }
            }
        }
        _ => {}
    }
    PasswordAction::None
}

#[allow(clippy::expect_used)]
fn row_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    if manager.edit.is_some() {
        return prompt_key(app, key);
    }
    if manager.draft.is_some() {
        return draft_key(app, key);
    }
    // A pending removal owns the next keystroke, so no other binding can be hit
    // by accident while the question is on screen.
    if manager.pending_delete.is_some() {
        return answer_pending_delete(app, key);
    }

    match key {
        // Leaving is Esc or `q`, the two keys that mean "back" everywhere else
        // in the app. `e` used to close it -- the key that opens the panel is
        // not the key that closes it, and `e` is needed for Edit, which is the
        // one thing this list is for.
        KeyCode::Esc | KeyCode::Char('q' | 'Q') => app.password_manager = None,
        KeyCode::Up | KeyCode::Char('k' | 'K') => {
            manager.selected = manager.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j' | 'J') => {
            manager.selected = (manager.selected + 1).min(app.panels.len().saturating_sub(1));
        }
        // Edit this host: name, user, port, upgrade command and password, all
        // in one place. `e` as well as Enter, because the panel is a list of
        // rows and "edit the row" is the action it exists for.
        KeyCode::Enter | KeyCode::Char('e' | 'E') => {
            manager.draft = app.panels.get(manager.selected).map(|panel| {
                ServerDraft::new(
                    Some(manager.selected),
                    Some(&panel.server),
                    panel.sudo_password.as_deref(),
                )
            });
        }
        KeyCode::Char('a' | 'A') => manager.draft = Some(ServerDraft::new(None, None, None)),
        KeyCode::Char('d' | 'D') if app.panels.len() > 1 => {
            let host = app
                .panels
                .get(manager.selected)
                .map_or_else(String::new, |p| p.server.host.clone());
            let selected = manager.selected;
            // Applying the edit stops every run in flight -- the generations all
            // move, so nothing could report on them afterwards anyway. That is
            // an interrupted package transaction, and the operator is told
            // before the key that does it, not after. Asked through the same
            // method the quit path names its hosts with, so the two answers
            // cannot drift apart.
            let running = app.running_upgrade_hosts();
            let notice = if running.is_empty() {
                format!("Remove {host} from the configuration? [y] confirm  [Esc] cancel")
            } else {
                format!(
                    "Remove {host}? This interrupts the upgrade running on {}. \
                     [y] confirm  [Esc] cancel",
                    running.join(", ")
                )
            };
            if let Some(manager) = app.password_manager.as_mut() {
                manager.pending_delete = Some(selected);
                manager.notice = Some(notice);
            }
        }
        KeyCode::Char('d' | 'D') => {
            manager.notice = Some("Cannot remove the last remaining server.".to_string());
        }
        // Import from ~/.ssh/config. Additive only: see `config::merge_ssh_hosts`
        // for why nothing already configured is touched.
        KeyCode::Char('b' | 'B') => return PasswordAction::CycleBannerStyle,
        KeyCode::Char('i' | 'I') => return PasswordAction::ImportSshHosts,
        // Change the vault master password. Offered only when a vault exists,
        // and only when one is not already being changed.
        KeyCode::Char('r' | 'R') => {
            if manager.rotating {
                manager.notice =
                    Some("The master password is already being changed; one moment.".to_string());
            } else if app.vault.is_some() {
                manager.edit = Some(PasswordEdit::RotateCurrent);
                manager.input.clear();
                manager.notice = Some("Enter the CURRENT master password:".to_string());
            } else {
                manager.notice =
                    Some("No vault to rotate; save a password to create one.".to_string());
            }
        }
        _ => {}
    }
    PasswordAction::None
}
