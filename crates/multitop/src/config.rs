//! Config loading and validation.
//!
//! Error text is user-facing and deliberately verbose — a missing or
//! malformed config is the most common first-run failure, and the message is
//! the only help the user gets.

use std::fmt;
use std::path::{Path, PathBuf};

/// Shipped alongside the binary so a fresh install can print a working
/// example without needing the source tree.
pub const EXAMPLE_CONFIG: &str = include_str!("../../../config.example.toml");

pub const DEFAULT_PORT: u16 = 22;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub upgrade_cmd: Option<String>,
}

impl Server {
    /// SSH destination, `user@host` when a user is configured.
    pub fn target(&self) -> std::borrow::Cow<'_, str> {
        if self.user.is_empty() {
            std::borrow::Cow::Borrowed(&self.host)
        } else {
            std::borrow::Cow::Owned(format!("{}@{}", self.user, self.host))
        }
    }
}

pub fn default_config_path() -> PathBuf {
    config_home().join("multitop/config.toml")
}

/// Location the project used before it was renamed, so we can point at it.
pub fn legacy_config_path() -> PathBuf {
    config_home().join("monitor/config.toml")
}

fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(dir);
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".config"),
        None => PathBuf::from(".config"),
    }
}

/// A user name lands in an `ssh` argv slot; whitespace there silently splits
/// into extra arguments, so reject it rather than connecting somewhere odd.
pub fn validate_user(user: &str) -> Result<(), ConfigError> {
    if user.chars().any(char::is_whitespace) {
        return err(format!("Invalid user '{user}': contains whitespace"));
    }
    Ok(())
}

/// A host with whitespace would likewise be split by `ssh`.
pub fn validate_host(host: &str) -> Result<(), ConfigError> {
    if host.chars().any(char::is_whitespace) {
        return err(format!("Invalid host '{host}': contains whitespace"));
    }
    Ok(())
}

fn missing_config_message(path: &Path, legacy: Option<&Path>) -> ConfigError {
    let path = path.display();
    if let Some(old) = legacy {
        let old = old.display();
        return ConfigError(format!(
            "Configuration file missing at {path}\n\n\
             \x20 Your config is still at the old location:\n\
             \x20   {old}\n\n\
             \x20 Migrate it:\n\
             \x20   mkdir -p ~/.config/multitop\n\
             \x20   mv {old} {path}"
        ));
    }
    let hint: String = EXAMPLE_CONFIG
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("  {}", l.trim_end()))
        .collect::<Vec<_>>()
        .join("\n");
    ConfigError(format!(
        "Configuration file missing at {path}\n\n  Create it. Example:\n\n{hint}\n"
    ))
}

pub const DEFAULT_UPGRADE_HISTORY_LINES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub servers: Vec<Server>,
    pub theme: Option<String>,
    pub upgrade_history_lines: usize,
}

/// Read and validate the server list and config settings.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let legacy = legacy_config_path();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            let legacy = legacy.exists().then_some(legacy);
            return Err(missing_config_message(path, legacy.as_deref()));
        }
    };
    parse(&text)
}

/// Parse config text. Split from [`load`] so the validation rules are
/// testable without touching the filesystem.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    // `Value::from_str` parses a bare value; a whole document needs the
    // deserializer entry point.
    let value: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => return err(format!("Could not parse configuration: {e}")),
    };

    let theme = value.get("theme").and_then(|v| v.as_str()).map(String::from);

    let servers = match value.get("servers") {
        None => return err("No 'servers' entries found in configuration"),
        Some(toml::Value::Array(a)) => a,
        Some(_) => return err("'servers' must be a list of tables, got a non-list value"),
    };
    if servers.is_empty() {
        return err("No 'servers' entries found in configuration");
    }

    let mut out = Vec::with_capacity(servers.len());
    for (idx, entry) in servers.iter().enumerate() {
        let Some(table) = entry.as_table() else {
            return err(format!("Server entry at index {idx} is not a table"));
        };
        let host = match table.get("host").and_then(|v| v.as_str()) {
            Some(h) if !h.is_empty() => h.to_string(),
            _ => return err(format!("Server entry at index {idx} missing 'host' field")),
        };
        validate_host(&host)?;

        let user = table
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        validate_user(&user)?;

        let port = match table.get("port") {
            None => DEFAULT_PORT,
            Some(v) => match v.as_integer() {
                Some(p) if (1..=65535).contains(&p) => p as u16,
                _ => return err(format!("Server entry at index {idx} has an invalid 'port'")),
            },
        };

        let upgrade_cmd = table
            .get("upgrade_cmd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());

        out.push(Server {
            host,
            port,
            user,
            upgrade_cmd,
        });
    }

    let upgrade_history_lines = value
        .get("upgrade_history_lines")
        .or_else(|| value.get("history_lines"))
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_UPGRADE_HISTORY_LINES);

    Ok(Config {
        servers: out,
        theme,
        upgrade_history_lines,
    })
}

/// Save theme selection back to the TOML configuration file.
pub fn save_theme(path: &Path, theme_name: &str) {
    let Ok(content) = std::fs::read_to_string(path) else { return; };
    let Ok(mut doc) = content.parse::<toml::Table>() else { return; };
    doc.insert("theme".to_string(), toml::Value::String(theme_name.to_string()));
    let Ok(new_content) = toml::to_string(&doc) else { return; };
    let _ = std::fs::write(path, new_content);
}

/// Parse standard SSH config file (~/.ssh/config) for Host blocks.
pub fn parse_ssh_config(text: &str) -> Vec<Server> {
    let mut servers = Vec::new();
    let mut current_host: Option<String> = None;
    let mut current_user = String::new();
    let mut current_port = DEFAULT_PORT;
    let mut real_hostname: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let val = parts.collect::<Vec<_>>().join(" ");

        match key.to_lowercase().as_str() {
            "host" => {
                if let Some(host) = current_host.take() {
                    servers.push(Server {
                        host: real_hostname.take().unwrap_or(host),
                        port: current_port,
                        user: current_user.clone(),
                        upgrade_cmd: None,
                    });
                }
                if val.contains('*') || val.contains('?') {
                    current_host = None;
                } else {
                    current_host = Some(val);
                    current_user.clear();
                    current_port = DEFAULT_PORT;
                    real_hostname = None;
                }
            }
            "hostname" => {
                if current_host.is_some() {
                    real_hostname = Some(val);
                }
            }
            "user" => {
                if current_host.is_some() {
                    current_user = val;
                }
            }
            "port" if current_host.is_some() => {
                if let Ok(p) = val.parse::<u16>() {
                    current_port = p;
                }
            }
            _ => {}
        }
    }

    if let Some(host) = current_host {
        servers.push(Server {
            host: real_hostname.take().unwrap_or(host),
            port: current_port,
            user: current_user,
            upgrade_cmd: None,
        });
    }

    servers
}


