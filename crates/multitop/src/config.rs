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
    pub fn target(&self) -> String {
        if self.user.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user, self.host)
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

/// Read and validate the server list.
pub fn load(path: &Path) -> Result<Vec<Server>, ConfigError> {
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
pub fn parse(text: &str) -> Result<Vec<Server>, ConfigError> {
    // `Value::from_str` parses a bare value; a whole document needs the
    // deserializer entry point.
    let value: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => return err(format!("Could not parse configuration: {e}")),
    };

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
    Ok(out)
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
                if let Some(host_alias) = current_host.take() {
                    if host_alias != "*" && !host_alias.contains('*') && !host_alias.contains('?') {
                        servers.push(Server {
                            host: real_hostname.unwrap_or(host_alias),
                            port: current_port,
                            user: current_user.clone(),
                            upgrade_cmd: None,
                        });
                    }
                }
                current_host = Some(val.clone());
                current_user.clear();
                current_port = DEFAULT_PORT;
                real_hostname = None;
            }
            "hostname" => real_hostname = Some(val),
            "user" => current_user = val,
            "port" => {
                if let Ok(p) = val.parse::<u16>() {
                    current_port = p;
                }
            }
            _ => {}
        }
    }

    if let Some(host_alias) = current_host {
        if host_alias != "*" && !host_alias.contains('*') && !host_alias.contains('?') {
            servers.push(Server {
                host: real_hostname.unwrap_or(host_alias),
                port: current_port,
                user: current_user,
                upgrade_cmd: None,
            });
        }
    }

    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_err(text: &str) -> String {
        parse(text).unwrap_err().0
    }

    #[test]
    fn valid_single_server() {
        let s = parse("[[servers]]\nhost = \"192.168.0.33\"\nport = 22\nuser = \"\"\n").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].host, "192.168.0.33");
        assert_eq!(s[0].port, 22);
        assert_eq!(s[0].user, "");
        assert_eq!(s[0].upgrade_cmd, None);
    }

    #[test]
    fn multiple_servers() {
        let s = parse(
            "[[servers]]\nhost = \"192.168.0.33\"\n\n[[servers]]\nhost = \"192.168.0.90\"\nuser = \"admin\"\n",
        )
        .unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].user, "admin");
    }

    #[test]
    fn port_defaults_to_22() {
        assert_eq!(
            parse("[[servers]]\nhost = \"a\"\n").unwrap()[0].port,
            DEFAULT_PORT
        );
    }

    #[test]
    fn upgrade_cmd_read() {
        let s = parse("[[servers]]\nhost = \"a\"\nupgrade_cmd = \"apt upgrade -y\"\n").unwrap();
        assert_eq!(s[0].upgrade_cmd.as_deref(), Some("apt upgrade -y"));
    }

    #[test]
    fn blank_upgrade_cmd_is_none() {
        let s = parse("[[servers]]\nhost = \"a\"\nupgrade_cmd = \"   \"\n").unwrap();
        assert_eq!(s[0].upgrade_cmd, None);
    }

    #[test]
    fn servers_must_be_a_list() {
        assert!(parse_err("servers = {}").contains("non-list"));
    }

    #[test]
    fn empty_servers_rejected() {
        assert!(parse_err("servers = []").contains("No 'servers' entries"));
    }

    #[test]
    fn absent_servers_rejected() {
        assert!(parse_err("other = 1").contains("No 'servers' entries"));
    }

    #[test]
    fn missing_host_rejected() {
        assert!(parse_err("[[servers]]\nport = 22\n").contains("missing 'host'"));
    }

    #[test]
    fn empty_host_rejected() {
        assert!(parse_err("[[servers]]\nhost = \"\"\n").contains("missing 'host'"));
    }

    #[test]
    fn non_table_entry_rejected() {
        assert!(parse_err("servers = ['not-a-table']").contains("not a table"));
    }

    #[test]
    fn second_entry_non_table_rejected() {
        let e = parse_err("servers = [{host = 'a'}, 'bad-string']");
        assert!(e.contains("not a table"));
        assert!(
            e.contains("index 1"),
            "the message names the offending entry: {e}"
        );
    }

    #[test]
    fn whitespace_user_rejected() {
        assert!(
            parse_err("[[servers]]\nhost = \"a\"\nuser = \"bad user\"\n").contains("whitespace")
        );
    }

    #[test]
    fn whitespace_host_rejected() {
        assert!(parse_err("[[servers]]\nhost = \"bad host\"\n").contains("whitespace"));
    }

    #[test]
    fn invalid_port_rejected() {
        assert!(parse_err("[[servers]]\nhost = \"a\"\nport = 0\n").contains("invalid 'port'"));
        assert!(parse_err("[[servers]]\nhost = \"a\"\nport = 99999\n").contains("invalid 'port'"));
        assert!(parse_err("[[servers]]\nhost = \"a\"\nport = \"22\"\n").contains("invalid 'port'"));
    }

    #[test]
    fn malformed_toml_reported() {
        assert!(parse_err("[[servers").contains("Could not parse"));
    }

    #[test]
    fn user_validation() {
        assert!(validate_user("").is_ok());
        assert!(validate_user("admin").is_ok());
        assert!(validate_user("monitor-user").is_ok());
        for bad in ["bad user", " admin", "admin ", "a\tb", "a\nb"] {
            assert!(validate_user(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn target_formatting() {
        let mut s = Server {
            host: "10.0.0.1".into(),
            port: 22,
            user: String::new(),
            upgrade_cmd: None,
        };
        assert_eq!(s.target(), "10.0.0.1");
        s.user = "admin".into();
        assert_eq!(s.target(), "admin@10.0.0.1");
    }

    #[test]
    fn missing_config_points_at_legacy_location() {
        let e = missing_config_message(
            Path::new("/new/config.toml"),
            Some(Path::new("/old/config.toml")),
        );
        assert!(e.0.contains("old location"));
        assert!(e.0.contains("/old/config.toml"));
        assert!(e.0.contains("mv /old/config.toml /new/config.toml"));
    }

    #[test]
    fn missing_config_shows_example() {
        let e = missing_config_message(Path::new("/new/config.toml"), None);
        assert!(e.0.contains("Create it. Example:"));
        assert!(e.0.contains("[[servers]]"));
        // Example lines are indented, and blanks dropped.
        assert!(e.0.contains("  [[servers]]"));
        assert!(!e.0.contains("\n\n\n"));
    }

    /// The bundled example must itself be a config the parser accepts.
    #[test]
    fn bundled_example_is_valid() {
        let servers = parse(EXAMPLE_CONFIG).expect("example config must parse");
        assert!(!servers.is_empty());
    }

    #[test]
    fn load_missing_file_reports_path() {
        let e = load(Path::new("/nonexistent/multitop/config.toml")).unwrap_err();
        assert!(e
            .0
            .contains("Configuration file missing at /nonexistent/multitop/config.toml"));
    }

    #[test]
    fn parse_ssh_config_extracts_hosts() {
        let ssh_cfg = "Host prod-db\n  HostName 192.168.0.33\n  User ztomer\n  Port 2222\n\nHost *\n  User root\n";
        let servers = parse_ssh_config(ssh_cfg);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].host, "192.168.0.33");
        assert_eq!(servers[0].user, "ztomer");
        assert_eq!(servers[0].port, 2222);
    }
}
