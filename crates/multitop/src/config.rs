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
    #[must_use]
    pub fn target(&self) -> std::borrow::Cow<'_, str> {
        if self.user.is_empty() {
            std::borrow::Cow::Borrowed(&self.host)
        } else {
            std::borrow::Cow::Owned(format!("{}@{}", self.user, self.host))
        }
    }
}

#[must_use]
pub fn default_config_path() -> PathBuf {
    config_home().join("multitop/config.toml")
}

/// Location the project used before it was renamed, so we can point at it.
#[must_use]
pub fn legacy_config_path() -> PathBuf {
    config_home().join("monitor/config.toml")
}

fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".config"),
        |home| PathBuf::from(home).join(".config"),
    )
}

/// Validate that a user name contains no whitespace.
///
/// # Errors
///
/// Returns an error if the user name contains whitespace.
pub fn validate_user(user: &str) -> Result<(), ConfigError> {
    if user.chars().any(char::is_whitespace) {
        return err(format!("Invalid user '{user}': contains whitespace"));
    }
    Ok(())
}

/// Validate that a host name contains no whitespace.
///
/// # Errors
///
/// Returns an error if the host name contains whitespace.
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
    pub show_sparklines: bool,
    /// Plaintext `sudo_password` values found in the config file, paired with
    /// the server they belong to.
    ///
    /// Storing a sudo password in `config.toml` is not supported: the file is
    /// world-readable plaintext, and the value was silently ignored, so anyone
    /// who set one had a password sitting on disk doing nothing. They are
    /// collected here so startup can move them into the credential store and
    /// strip them from the file exactly once.
    pub plaintext_passwords: Vec<(Server, String)>,
}

/// Read and validate the server list and config settings.
///
/// Read and validate the server list and config settings.
///
/// # Errors
///
/// Returns an error if the config file cannot be read or parsed.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let legacy = legacy_config_path();
    let Ok(text) = std::fs::read_to_string(path) else {
        let legacy = legacy.exists().then_some(legacy);
        return Err(missing_config_message(path, legacy.as_deref()));
    };
    parse(&text)
}

/// Parse config text. Split from [`load`] so the validation rules are
/// testable without touching the filesystem.
///
/// # Errors
///
/// Returns an error if the text cannot be parsed as TOML.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    // `Value::from_str` parses a bare value; a whole document needs the
    // deserializer entry point.
    let value: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => return err(format!("Could not parse configuration: {e}")),
    };

    let theme = value
        .get("theme")
        .and_then(|v| v.as_str())
        .map(String::from);

    let servers = match value.get("servers") {
        None => return err("No 'servers' entries found in configuration"),
        Some(toml::Value::Array(a)) => a,
        Some(_) => return err("'servers' must be a list of tables, got a non-list value"),
    };
    if servers.is_empty() {
        return err("No 'servers' entries found in configuration");
    }

    let mut out = Vec::with_capacity(servers.len());
    let mut plaintext = Vec::new();
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
            .map_or_else(String::new, String::from);
        validate_user(&user)?;

        let port = match table.get("port") {
            None => DEFAULT_PORT,
            Some(v) => match v.as_integer().and_then(|p| u16::try_from(p).ok()) {
                Some(p) if p > 0 => p,
                _ => return err(format!("Server entry at index {idx} has an invalid 'port'")),
            },
        };

        let upgrade_cmd = table
            .get("upgrade_cmd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());

        let server = Server {
            host,
            port,
            user,
            upgrade_cmd,
        };

        if let Some(secret) = table
            .get("sudo_password")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            plaintext.push((server.clone(), secret.to_string()));
        }

        out.push(server);
    }

    let upgrade_history_lines = value
        .get("upgrade_history_lines")
        .or_else(|| value.get("history_lines"))
        .and_then(toml::Value::as_integer)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(DEFAULT_UPGRADE_HISTORY_LINES);

    let show_sparklines = value
        .get("show_sparklines")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);

    Ok(Config {
        servers: out,
        theme,
        upgrade_history_lines,
        show_sparklines,
        plaintext_passwords: plaintext,
    })
}

/// Remove every `sudo_password` key from the `[[servers]]` tables, preserving
/// everything else in the file.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or written.
pub fn strip_plaintext_passwords(path: &Path) -> Result<usize, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut doc = content
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;

    let mut removed = 0;
    if let Some(toml::Value::Array(servers)) = doc.get_mut("servers") {
        for entry in servers.iter_mut() {
            if let Some(table) = entry.as_table_mut() {
                if table.remove("sudo_password").is_some() {
                    removed += 1;
                }
            }
        }
    }
    if removed > 0 {
        let text = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

/// Save theme selection back to the TOML configuration file.
pub fn save_theme(path: &Path, theme_name: &str) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut doc) = content.parse::<toml::Table>() else {
        return;
    };
    doc.insert(
        "theme".to_string(),
        toml::Value::String(theme_name.to_string()),
    );
    let Ok(new_content) = toml::to_string(&doc) else {
        return;
    };
    let _ = std::fs::write(path, new_content);
}

/// Save `show_sparklines` preference back to the TOML configuration file.
pub fn save_show_sparklines(path: &Path, show: bool) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut doc) = content.parse::<toml::Table>() else {
        return;
    };
    doc.insert("show_sparklines".to_string(), toml::Value::Boolean(show));
    let Ok(new_content) = toml::to_string(&doc) else {
        return;
    };
    let _ = std::fs::write(path, new_content);
}

/// Replace the server list while preserving other top-level configuration.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn save_servers(path: &Path, servers: &[Server]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc = if content.trim().is_empty() {
        toml::Table::new()
    } else {
        content
            .parse::<toml::Table>()
            .map_err(|error| error.to_string())?
    };
    let entries = servers
        .iter()
        .map(|server| {
            let mut table = toml::Table::new();
            table.insert("host".into(), toml::Value::String(server.host.clone()));
            table.insert("port".into(), toml::Value::Integer(server.port.into()));
            table.insert("user".into(), toml::Value::String(server.user.clone()));
            if let Some(command) = &server.upgrade_cmd {
                table.insert("upgrade_cmd".into(), toml::Value::String(command.clone()));
            }
            toml::Value::Table(table)
        })
        .collect();
    doc.insert("servers".into(), toml::Value::Array(entries));
    let output = toml::to_string(&doc).map_err(|error| error.to_string())?;
    std::fs::write(path, output).map_err(|error| error.to_string())
}

/// Parse standard SSH config file (~/.ssh/config) for Host blocks.
#[must_use]
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
        let val = parts.clone().collect::<Vec<_>>().join(" ");

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
                let first_host = parts.next().unwrap_or("");
                if first_host.contains('*') || first_host.contains('?') || first_host.is_empty() {
                    current_host = None;
                } else {
                    current_host = Some(first_host.to_string());
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
