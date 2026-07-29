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
    pub field: usize,
}

impl ServerDraft {
    fn new(original: Option<usize>, server: Option<&Server>) -> Self {
        Self {
            original,
            host: server.map_or_else(String::new, |s| s.host.clone()),
            user: server.map_or_else(String::new, |s| s.user.clone()),
            port: server.map_or_else(|| DEFAULT_PORT.to_string(), |s| s.port.to_string()),
            upgrade_cmd: server
                .and_then(|s| s.upgrade_cmd.clone())
                .unwrap_or_default(),
            field: 0,
        }
    }

    fn active_field(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.host,
            1 => &mut self.user,
            2 => &mut self.port,
            _ => &mut self.upgrade_cmd,
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
    pub input: String,
    pub store_on_save: bool,
    pub resume_upgrade: bool,
    pub draft: Option<ServerDraft>,
    pub notice: Option<String>,
}

impl PasswordManager {
    pub fn new(selected: usize, resume_upgrade: bool) -> Self {
        Self {
            section: ConfigSection::Passwords,
            selected,
            editing: false,
            input: String::new(),
            store_on_save: false,
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
        store: bool,
        resume_upgrade: bool,
    },
    Delete {
        panel: usize,
    },
    ApplyServers(Vec<Server>),
}

pub fn open(app: &mut App, selected: usize, resume_upgrade: bool) {
    if !app.panels.is_empty() {
        app.password_manager = Some(PasswordManager::new(
            selected.min(app.panels.len() - 1),
            resume_upgrade,
        ));
    }
}

pub fn handle_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let Some(manager) = app.password_manager.as_mut() else {
        return PasswordAction::None;
    };
    if key == KeyCode::Tab && !manager.editing && manager.draft.is_none() {
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

fn password_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    if manager.editing {
        match key {
            KeyCode::Esc => {
                manager.editing = false;
                manager.input.clear();
            }
            KeyCode::Enter => {
                let password = std::mem::take(&mut manager.input);
                manager.editing = false;
                if password.is_empty() {
                    manager.notice = Some("Password was not changed.".to_string());
                } else {
                    return PasswordAction::Save {
                        panel: manager.selected,
                        password,
                        store: manager.store_on_save,
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
            manager.selected = manager.selected.saturating_sub(1)
        }
        KeyCode::Down | KeyCode::Char('j' | 'J') => {
            manager.selected = (manager.selected + 1).min(app.panels.len().saturating_sub(1))
        }
        KeyCode::Enter => {
            manager.editing = true;
            manager.input.clear();
            manager.store_on_save = app.panels[manager.selected].password_saved;
            manager.notice = None;
        }
        KeyCode::Char('s' | 'S') => manager.store_on_save = !manager.store_on_save,
        KeyCode::Char('d' | 'D') => {
            return PasswordAction::Delete {
                panel: manager.selected,
            }
        }
        _ => {}
    }
    PasswordAction::None
}

fn server_key(app: &mut App, key: KeyCode) -> PasswordAction {
    let manager = app.password_manager.as_mut().expect("manager exists");
    if let Some(draft) = manager.draft.as_mut() {
        match key {
            KeyCode::Esc => manager.draft = None,
            KeyCode::Tab | KeyCode::Down => draft.field = (draft.field + 1) % 4,
            KeyCode::Up => draft.field = draft.field.checked_sub(1).unwrap_or(3),
            KeyCode::Backspace => {
                draft.active_field().pop();
            }
            KeyCode::Char(character) => draft.active_field().push(character),
            KeyCode::Enter => {
                let draft = manager.draft.take().expect("draft exists");
                match draft.clone().into_server() {
                    Ok(server) => {
                        let mut servers: Vec<Server> = app
                            .panels
                            .iter()
                            .map(|panel| panel.server.clone())
                            .collect();
                        if let Some(index) = draft.original {
                            servers[index] = server;
                        } else {
                            servers.push(server);
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
            manager.selected = manager.selected.saturating_sub(1)
        }
        KeyCode::Down | KeyCode::Char('j' | 'J') => {
            manager.selected = (manager.selected + 1).min(app.panels.len().saturating_sub(1))
        }
        KeyCode::Char('a' | 'A') => manager.draft = Some(ServerDraft::new(None, None)),
        KeyCode::Enter => {
            manager.draft = app
                .panels
                .get(manager.selected)
                .map(|panel| ServerDraft::new(Some(manager.selected), Some(&panel.server)))
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
