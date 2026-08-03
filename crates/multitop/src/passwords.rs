//! State and input handling for the full-screen configuration panel.

use crossterm::event::KeyCode;
use secrecy::{ExposeSecret, SecretString};

use crate::app::App;
use crate::config::{validate_host, validate_user, Server, DEFAULT_PORT};

#[derive(Clone, Debug)]
pub struct ServerDraft {
    pub original: Option<usize>,
    pub host: String,
    pub user: String,
    pub port: String,
    pub upgrade_cmd: String,
    pub password: String,
    pub field: usize,
}

impl ServerDraft {
    pub fn new(original: Option<usize>, server: Option<&Server>, password: Option<&str>) -> Self {
        Self {
            original,
            host: server.map_or_else(String::new, |s| s.host.clone()),
            user: server.map_or_else(String::new, |s| s.user.clone()),
            port: server.map_or_else(|| DEFAULT_PORT.to_string(), |s| s.port.to_string()),
            upgrade_cmd: server
                .and_then(|s| s.upgrade_cmd.clone())
                .unwrap_or_default(),
            password: password.unwrap_or_default().to_string(),
            field: 0,
        }
    }

    const fn active_field(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.host,
            1 => &mut self.user,
            2 => &mut self.port,
            3 => &mut self.upgrade_cmd,
            _ => &mut self.password,
        }
    }

    fn into_server(self) -> Result<Server, String> {
        validate_host(&self.host).map_err(|error| error.0)?;
        validate_user(&self.user).map_err(|error| error.0)?;
        let port = self
            .port
            .parse::<u16>()
            .map_err(|_| "Port must be between 1 and 65535.".to_string())?;
        if port == 0 {
            return Err("Port must be between 1 and 65535.".to_string());
        }
        Ok(Server {
            host: self.host,
            user: self.user,
            port,
            upgrade_cmd: (!self.upgrade_cmd.trim().is_empty()).then_some(self.upgrade_cmd),
        })
    }
}

#[derive(Clone, Debug)]
pub struct PasswordManager {
    pub selected: usize,
    /// What the current text entry is for, if any.
    ///
    /// This replaced an `editing: bool` plus an `is_sso: bool`. Rotating the
    /// master password needs two prompts in sequence, and the second has to
    /// carry the first's answer -- which a pair of flags cannot express without
    /// a third flag and an untyped side channel.
    pub edit: Option<PasswordEdit>,
    pub input: String,
    pub resume_upgrade: bool,
    pub draft: Option<ServerDraft>,
    pub notice: Option<String>,
    /// Index awaiting confirmation for removal, if any.
    ///
    /// Deleting a server rewrites config.toml and cannot be undone, so it takes
    /// two keys. Running an upgrade already asks first; removing a server was
    /// the more destructive of the two and asked for nothing.
    pub pending_delete: Option<usize>,
}

/// What a text prompt in the Passwords section is collecting.
#[derive(Clone, Debug)]
pub enum PasswordEdit {
    /// The current vault master password, on the way to changing it.
    RotateCurrent,
    /// The replacement, carrying the current one that was just verified by
    /// being typed. Held as a `SecretString` because it lives across two
    /// keystroke rounds rather than being consumed immediately.
    RotateNew { current: SecretString },
}

impl PasswordManager {
    /// Whether a text prompt is open, for rendering.
    #[must_use]
    pub const fn editing(&self) -> bool {
        self.edit.is_some()
    }

    #[must_use]
    pub const fn new(selected: usize, resume_upgrade: bool) -> Self {
        Self {
            selected,
            edit: None,
            input: String::new(),
            resume_upgrade,
            draft: None,
            notice: None,
            pending_delete: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PasswordAction {
    None,
    Save {
        panel: usize,
        password: String,
        resume_upgrade: bool,
    },
    /// Clear the stored password for a host, because its editor was saved with
    /// an empty password field.
    Delete {
        panel: usize,
    },
    ToggleSparklines,
    /// Add hosts from `~/.ssh/config` that are not configured yet.
    ImportSshHosts,
    /// Replace the vault master password. Both are plain `String` because they
    /// are handed straight to the vault and dropped.
    RotateVaultPassword {
        current: String,
        new: String,
    },
    ApplyServers(Vec<Server>),
    /// The result of editing or adding one row: the new server list, and what
    /// to do with that row's password. `None` means the field was left empty,
    /// so any stored password for the host is removed.
    ApplyServerEdit {
        servers: Vec<Server>,
        target_idx: usize,
        password: Option<String>,
    },
}

pub fn open(app: &mut App, selected: usize, resume_upgrade: bool) {
    if !app.panels.is_empty() {
        let idx = selected.min(app.panels.len() - 1);
        app.panels[idx].ensure_sudo_password();
        app.password_manager = Some(PasswordManager::new(idx, resume_upgrade));
    }
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
    let confirmed = matches!(key, KeyCode::Char('y' | 'Y') | KeyCode::Enter);
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
            manager.pending_delete = Some(manager.selected);
            manager.notice = Some(format!(
                "Remove {host} from the configuration? [y] confirm  [Esc] cancel"
            ));
        }
        KeyCode::Char('d' | 'D') => {
            manager.notice = Some("Cannot remove the last remaining server.".to_string());
        }
        KeyCode::Char('s' | 'S') => return PasswordAction::ToggleSparklines,
        // Import from ~/.ssh/config. Additive only: see `config::merge_ssh_hosts`
        // for why nothing already configured is touched.
        KeyCode::Char('i' | 'I') => return PasswordAction::ImportSshHosts,
        // Change the vault master password. Offered only when a vault exists.
        KeyCode::Char('r' | 'R') => {
            if app.vault.is_some() {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::app::App;
    use crate::config::Server;
    use crossterm::event::KeyCode;

    fn test_server(host: &str) -> Server {
        Server {
            host: host.to_string(),
            port: 22,
            user: "admin".to_string(),
            upgrade_cmd: Some("sudo apt update".to_string()),
        }
    }

    #[test]
    fn server_key_draft_field_navigation() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));

        crate::passwords::handle_key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            1
        );

        crate::passwords::handle_key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            2
        );

        crate::passwords::handle_key(&mut app, KeyCode::Up);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            1
        );

        crate::passwords::handle_key(&mut app, KeyCode::Down);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            2
        );
    }

    #[test]
    fn server_key_draft_char_input() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));

        crate::passwords::handle_key(&mut app, KeyCode::Char('t'));
        crate::passwords::handle_key(&mut app, KeyCode::Char('e'));
        crate::passwords::handle_key(&mut app, KeyCode::Char('s'));
        crate::passwords::handle_key(&mut app, KeyCode::Char('t'));

        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .host,
            "test"
        );
    }

    #[test]
    fn server_key_draft_backspace() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "test".to_string();

        crate::passwords::handle_key(&mut app, KeyCode::Backspace);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .host,
            "tes"
        );
    }

    #[test]
    fn server_key_draft_enter_valid() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "newhost".to_string();
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .user = "user".to_string();
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .port = "22".to_string();
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .upgrade_cmd = "cmd".to_string();

        let action = crate::passwords::handle_key(&mut app, KeyCode::Enter);
        assert!(matches!(
            action,
            PasswordAction::ApplyServers(_) | PasswordAction::ApplyServerEdit { .. }
        ));
    }

    #[test]
    fn server_key_draft_enter_invalid() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "host with spaces".to_string();

        let action = crate::passwords::handle_key(&mut app, KeyCode::Enter);
        assert_eq!(action, PasswordAction::None);
        assert!(app.password_manager.as_ref().unwrap().notice.is_some());
        assert!(app.password_manager.as_ref().unwrap().draft.is_some());
    }

    #[test]
    fn server_key_draft_esc_cancels() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "test".to_string();

        let action = crate::passwords::handle_key(&mut app, KeyCode::Esc);
        assert_eq!(action, PasswordAction::None);
        assert!(app.password_manager.as_ref().unwrap().draft.is_none());
    }

    /// `s` toggles sparklines. It used to set a shared sudo password, which is
    /// a thing that no longer exists.
    #[test]
    fn s_toggles_sparklines() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('s'));
        assert_eq!(action, PasswordAction::ToggleSparklines);
    }

    /// `d` removes the server, and is the only meaning `d` has now. It used to
    /// mean "delete the password" in one section and "delete the server" in the
    /// other, on the same key, over the same list of hosts.
    #[test]
    fn d_removes_a_server_after_confirmation() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);

        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('y'));
        let PasswordAction::ApplyServers(remaining) = action else {
            panic!("expected the server list to change");
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].host, "host2");
    }

    /// Removing a server rewrites config.toml and cannot be undone, so it takes
    /// two keys. It used to happen on the first press, with no question asked --
    /// while running an upgrade, the less destructive of the two, had a confirm
    /// modal.
    #[test]
    fn server_delete_asks_before_removing() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        assert_eq!(action, PasswordAction::None, "the first press must not act");
        let manager = app.password_manager.as_ref().unwrap();
        assert_eq!(manager.pending_delete, Some(0));
        assert!(
            manager.notice.as_ref().unwrap().contains("host1"),
            "the question must name what is about to be removed"
        );
    }

    #[test]
    fn server_delete_goes_ahead_on_confirmation() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);
        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('y'));
        let PasswordAction::ApplyServers(remaining) = action else {
            panic!("expected the removal to be applied");
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].host, "host2");
        assert_eq!(app.password_manager.as_ref().unwrap().pending_delete, None);
    }

    #[test]
    fn server_delete_can_be_cancelled() {
        for cancel in [KeyCode::Esc, KeyCode::Char('n')] {
            let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
            crate::passwords::open(&mut app, 0, false);
            crate::passwords::handle_key(&mut app, KeyCode::Char('d'));

            let action = crate::passwords::handle_key(&mut app, cancel);
            assert_eq!(action, PasswordAction::None, "{cancel:?} must not remove");
            let manager = app.password_manager.as_ref().unwrap();
            assert_eq!(manager.pending_delete, None, "{cancel:?} disarms");
            assert!(manager.notice.as_ref().unwrap().contains("cancelled"));
        }
    }

    /// A stray second press must not become the confirmation for a question the
    /// user never saw answered.
    #[test]
    fn a_cancelled_removal_stays_cancelled() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);
        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        crate::passwords::handle_key(&mut app, KeyCode::Esc);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('y'));
        assert_eq!(
            action,
            PasswordAction::None,
            "y after a cancel must not remove anything"
        );
        assert_eq!(app.panels.len(), 2);
    }

    #[test]
    fn the_last_server_still_cannot_be_removed() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        assert_eq!(action, PasswordAction::None);
        let manager = app.password_manager.as_ref().unwrap();
        assert_eq!(manager.pending_delete, None, "nothing to confirm");
        assert!(manager.notice.as_ref().unwrap().contains("Cannot remove"));
    }
}
