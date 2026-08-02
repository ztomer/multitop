//! State and input handling for the full-screen configuration panel.

use crossterm::event::KeyCode;

use crate::app::App;
use crate::config::{validate_host, validate_user, Server, DEFAULT_PORT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigSection {
    Passwords,
    Servers,
}

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
    pub section: ConfigSection,
    pub selected: usize,
    pub editing: bool,
    pub is_sso: bool,
    pub input: String,
    pub resume_upgrade: bool,
    pub draft: Option<ServerDraft>,
    pub notice: Option<String>,
}

impl PasswordManager {
    #[must_use]
    pub const fn new(selected: usize, resume_upgrade: bool) -> Self {
        Self {
            section: ConfigSection::Passwords,
            selected,
            editing: false,
            is_sso: false,
            input: String::new(),
            resume_upgrade,
            draft: None,
            notice: None,
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
    Delete {
        panel: usize,
    },
    SaveSso {
        password: String,
    },
    DeleteSso,
    ToggleSparklines,
    ApplyServers(Vec<Server>),
    SaveServerWithPassword {
        servers: Vec<Server>,
        target_idx: usize,
        password: String,
    },
}

pub fn open(app: &mut App, selected: usize, resume_upgrade: bool) {
    if !app.panels.is_empty() {
        let idx = selected.min(app.panels.len() - 1);
        app.panels[idx].ensure_sudo_password();
        app.password_manager = Some(PasswordManager::new(idx, resume_upgrade));
    }
}

pub fn handle_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let Some(manager) = app.password_manager.as_mut() else {
        return PasswordAction::None;
    };
    if manager.draft.is_some() {
        return server_key(app, key);
    }
    if key == KeyCode::Tab && !manager.editing {
        manager.section = match manager.section {
            ConfigSection::Passwords => ConfigSection::Servers,
            ConfigSection::Servers => ConfigSection::Passwords,
        };
        manager.notice = None;
        return PasswordAction::None;
    }
    match manager.section {
        ConfigSection::Passwords => password_key(app, key),
        ConfigSection::Servers => server_key(app, key),
    }
}

#[allow(clippy::expect_used)]
fn password_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    if manager.editing {
        match key {
            KeyCode::Esc => {
                manager.editing = false;
                manager.is_sso = false;
                manager.input.clear();
            }
            KeyCode::Enter => {
                let password = std::mem::take(&mut manager.input);
                let is_sso = manager.is_sso;
                manager.editing = false;
                manager.is_sso = false;
                if password.is_empty() {
                    manager.notice = Some("Password was not changed.".to_string());
                } else if is_sso {
                    return PasswordAction::SaveSso { password };
                } else {
                    return PasswordAction::Save {
                        panel: manager.selected,
                        password,
                        resume_upgrade: manager.resume_upgrade,
                    };
                }
            }
            KeyCode::Backspace => {
                manager.input.pop();
            }
            KeyCode::Char(character) => manager.input.push(character),
            _ => {}
        }
        return PasswordAction::None;
    }
    match key {
        KeyCode::Esc | KeyCode::Char('e' | 'E') => app.password_manager = None,
        KeyCode::Up | KeyCode::Char('k' | 'K') => {
            manager.selected = manager.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j' | 'J') => {
            manager.selected = (manager.selected + 1).min(app.panels.len().saturating_sub(1));
        }
        KeyCode::Char('s' | 'S') | KeyCode::Enter => {
            manager.editing = true;
            manager.is_sso = true;
            manager.input.clear();
            manager.notice =
                Some("Enter Single Sign-On (SSO) password for all servers:".to_string());
        }
        KeyCode::Char('o' | 'O') => {
            manager.editing = true;
            manager.is_sso = false;
            manager.input.clear();
            manager.notice = Some(format!(
                "Enter server password override for {}:",
                app.panels[manager.selected].server.target()
            ));
        }
        KeyCode::Char('p' | 'P') => return PasswordAction::ToggleSparklines,
        KeyCode::Char('a' | 'A') => {
            manager.draft = Some(ServerDraft::new(None, None, None));
            manager.section = ConfigSection::Servers;
        }
        // In the Passwords section, delete the PASSWORD. It used to remove the
        // whole server from config.toml -- unconfirmed, one keystroke, under a
        // hint that just read "[D] Delete" while the user was looking at a list
        // of passwords. Server removal lives in the Servers section, where the
        // hint says so. This also gives `PasswordAction::Delete` a caller: it
        // was implemented and tested but no key produced it, so a saved
        // password could not be removed from the UI at all.
        KeyCode::Char('d' | 'D') => {
            return PasswordAction::Delete {
                panel: manager.selected,
            };
        }
        _ => {}
    }
    PasswordAction::None
}

#[allow(clippy::expect_used)]
fn server_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    if let Some(draft) = manager.draft.as_mut() {
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
                let pass_input = draft.password.clone();
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
                        if !pass_input.trim().is_empty() {
                            return PasswordAction::SaveServerWithPassword {
                                servers,
                                target_idx,
                                password: pass_input,
                            };
                        }
                        return PasswordAction::ApplyServers(servers);
                    }
                    Err(error) => {
                        manager.notice = Some(error);
                        manager.draft = Some(draft);
                    }
                }
            }
            _ => {}
        }
        return PasswordAction::None;
    }
    match key {
        KeyCode::Esc | KeyCode::Char('e' | 'E') => app.password_manager = None,
        KeyCode::Up | KeyCode::Char('k' | 'K') => {
            manager.selected = manager.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j' | 'J') => {
            manager.selected = (manager.selected + 1).min(app.panels.len().saturating_sub(1));
        }
        KeyCode::Char('a' | 'A') => manager.draft = Some(ServerDraft::new(None, None, None)),
        KeyCode::Enter => {
            manager.draft = app.panels.get(manager.selected).map(|panel| {
                ServerDraft::new(
                    Some(manager.selected),
                    Some(&panel.server),
                    panel.sudo_password.as_deref(),
                )
            });
        }
        KeyCode::Char('d' | 'D') if app.panels.len() > 1 => {
            let mut servers: Vec<Server> = app
                .panels
                .iter()
                .map(|panel| panel.server.clone())
                .collect();
            servers.remove(manager.selected);
            return PasswordAction::ApplyServers(servers);
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
            PasswordAction::ApplyServers(_) | PasswordAction::SaveServerWithPassword { .. }
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

    #[test]
    fn password_key_sparkline_toggle() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('p'));
        assert_eq!(action, PasswordAction::ToggleSparklines);
    }

    /// In the Passwords section, `d` deletes the password, not the server.
    ///
    /// It used to remove the whole server from config.toml -- unconfirmed, one
    /// keystroke, under a hint that read only "[D] Delete" while the user was
    /// looking at a list of passwords. It also meant `PasswordAction::Delete`
    /// had no caller at all: removing a saved password was implemented and
    /// tested but unreachable from the keyboard.
    #[test]
    fn password_key_d_deletes_the_password_not_the_server() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        assert_eq!(action, PasswordAction::Delete { panel: 0 });
    }

    /// Even with one server left, `d` here is about the password, so there is
    /// nothing to refuse.
    #[test]
    fn password_key_d_works_with_a_single_server() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        assert_eq!(
            action,
            PasswordAction::Delete { panel: 0 },
            "deleting a password must not be blocked by the server count"
        );
    }

    /// Removing a server still works, in the section that says so.
    #[test]
    fn server_section_d_still_removes_a_server() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);
        crate::passwords::handle_key(&mut app, KeyCode::Tab);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        let PasswordAction::ApplyServers(remaining) = action else {
            panic!("expected the server list to change");
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].host, "host2");
    }
}
