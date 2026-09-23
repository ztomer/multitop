use crate::config::{
    Config, ConfigError, Server, DEFAULT_PORT, DEFAULT_UPGRADE_HISTORY_LINES,
    MIN_UPGRADE_HISTORY_LINES,
};
use std::path::Path;

const MAX_ALERT_PCT: u8 = 100;

/// Read and parse the configuration at `path`.
///
/// # Errors
/// Returns `ConfigError` if the file cannot be read or does not parse.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| ConfigError(format!("Configuration file missing: {}", path.display())))?;
    parse(&text)
}

/// Parse configuration text.
///
/// # Errors
/// Returns `ConfigError` naming the offending key, or the index of the server
/// entry that is wrong — "invalid config" with three hosts in the file is not
/// something an operator can act on.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let value: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => return Err(ConfigError(format!("Could not parse configuration: {e}"))),
    };

    let theme = value
        .get("theme")
        .and_then(|v| v.as_str())
        .map(String::from);

    let servers = match value.get("servers") {
        None => {
            return Err(ConfigError(
                "No 'servers' entries found in configuration".to_string(),
            ))
        }
        Some(toml::Value::Array(a)) => a,
        Some(_) => {
            return Err(ConfigError(
                "'servers' must be a list of tables, got a non-list value".to_string(),
            ))
        }
    };
    if servers.is_empty() {
        return Err(ConfigError(
            "No 'servers' entries found in configuration".to_string(),
        ));
    }

    let (mut out, plaintext) = parse_servers(servers)?;

    // `[[panels]] command="nvidia-smi …"` per roadmap Phase 3 — treated as
    // additional servers whose panel runs the command every 250 ms via Exec
    // and is rendered as a Fetch card (`crates/multitop/src/render_payload.rs:20`).
    if let Some(toml::Value::Array(panels)) = value.get("panels") {
        for (idx, entry) in panels.iter().enumerate() {
            let Some(table) = entry.as_table() else {
                return Err(ConfigError(format!(
                    "Panel entry at index {idx} is not a table"
                )));
            };
            let host = match table.get("host").and_then(|v| v.as_str()) {
                Some(h) if !h.is_empty() => h.to_string(),
                _ => continue,
            };
            let (host, user, port) = endpoint(table, host, "Panel", idx)?;
            let command = table
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty());
            let server = Server {
                host,
                port,
                user,
                upgrade_cmd: None,
                custom_command: command,
                mcp: None,
            };
            out.push(server);
        }
    }

    let requested = value
        .get("upgrade_history_lines")
        .or_else(|| value.get("history_lines"))
        .and_then(toml::Value::as_integer)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(DEFAULT_UPGRADE_HISTORY_LINES);
    let upgrade_history_lines = requested.max(MIN_UPGRADE_HISTORY_LINES);
    let history_lines_raised_from = (upgrade_history_lines != requested).then_some(requested);

    let banner_style = value
        .get("banner_style")
        .and_then(toml::Value::as_str)
        .map_or_else(crate::layout::BannerStyle::default, |s| {
            crate::layout::BannerStyle::parse(s)
        });

    let alert_cpu = value
        .get("alert_cpu")
        .and_then(toml::Value::as_integer)
        .and_then(|v| u8::try_from(v).ok())
        .filter(|&v| v <= MAX_ALERT_PCT);
    let alert_mem = value
        .get("alert_mem")
        .and_then(toml::Value::as_integer)
        .and_then(|v| u8::try_from(v).ok())
        .filter(|&v| v <= MAX_ALERT_PCT);
    let alert_disk = value
        .get("alert_disk")
        .and_then(toml::Value::as_integer)
        .and_then(|v| u8::try_from(v).ok())
        .filter(|&v| v <= MAX_ALERT_PCT);

    let alert_targets = alert_targets(&value);

    Ok(Config {
        servers: out,
        theme,
        upgrade_history_lines,
        history_lines_raised_from,
        banner_style,
        plaintext_passwords: plaintext,
        alert_cpu,
        alert_mem,
        alert_disk,
        alerts: alert_targets,
    })
}

/// # Errors
/// Returns `ConfigError` if the host contains whitespace, which would split
/// one `ssh` argument into two.
pub fn validate_host(host: &str) -> Result<(), ConfigError> {
    if host.chars().any(char::is_whitespace) {
        Err(ConfigError(
            "Host name must not contain whitespace.".to_string(),
        ))
    } else {
        Ok(())
    }
}

/// # Errors
/// Returns `ConfigError` if the user contains whitespace, for the same reason
/// as [`validate_host`].
pub fn validate_user(user: &str) -> Result<(), ConfigError> {
    if user.chars().any(char::is_whitespace) {
        Err(ConfigError(
            "User name must not contain whitespace.".to_string(),
        ))
    } else {
        Ok(())
    }
}

/// A plaintext `sudo_password` found beside its server in the file, kept
/// aside so it can be moved into the vault.
type PlaintextSecret = (Server, String);

/// What `[[servers]]` yields: the servers, and the plaintext passwords any
/// of them carried.
type ParsedServers = (Vec<Server>, Vec<PlaintextSecret>);

/// The `[[servers]]` entries.
fn parse_servers(servers: &[toml::Value]) -> Result<ParsedServers, ConfigError> {
    let mut out = Vec::with_capacity(servers.len());
    let mut plaintext = Vec::new();
    for (idx, entry) in servers.iter().enumerate() {
        let Some(table) = entry.as_table() else {
            return Err(ConfigError(format!(
                "Server entry at index {idx} is not a table"
            )));
        };
        let host = match table.get("host").and_then(|v| v.as_str()) {
            Some(h) if !h.is_empty() => h.to_string(),
            _ => {
                return Err(ConfigError(format!(
                    "Server entry at index {idx} missing 'host' field"
                )))
            }
        };
        let (host, user, port) = endpoint(table, host, "Server", idx)?;

        let upgrade_cmd = table
            .get("upgrade_cmd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());

        let custom_command = table
            .get("command")
            .or_else(|| table.get("custom_command"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());

        let mcp = table
            .get("mcp")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());

        let server = Server {
            host,
            port,
            user,
            upgrade_cmd,
            custom_command,
            mcp,
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
    Ok((out, plaintext))
}

/// The validated `host`, `user` and `port` of a `[[servers]]` or `[[panels]]`
/// table; `what` names the table kind in errors, with its index.
fn endpoint(
    table: &toml::Table,
    host: String,
    what: &str,
    idx: usize,
) -> Result<(String, String, u16), ConfigError> {
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
            _ => {
                return Err(ConfigError(format!(
                    "{what} entry at index {idx} has an invalid 'port'"
                )))
            }
        },
    };
    Ok((host, user, port))
}

/// One alert target from its table: a webhook, a desktop flag, or nothing
/// when it names neither.
fn alert_target(tbl: &toml::Table) -> Option<crate::config::AlertTarget> {
    let webhook = tbl
        .get("webhook")
        .and_then(toml::Value::as_str)
        .map(String::from);
    let desktop = tbl
        .get("desktop")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    (webhook.is_some() || desktop).then_some(crate::config::AlertTarget { webhook, desktop })
}

/// `[[alerts]]` (an array of targets) or `[alerts]` (one target).
fn alert_targets(value: &toml::Value) -> Vec<crate::config::AlertTarget> {
    match value.get("alerts") {
        Some(toml::Value::Array(arr)) => arr
            .iter()
            .filter_map(|entry| entry.as_table().and_then(alert_target))
            .collect(),
        Some(toml::Value::Table(tbl)) => alert_target(tbl).into_iter().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alerts_array_of_tables() {
        let toml_str = r#"
[[servers]]
host = "prod-01"

[[alerts]]
webhook = "https://ntfy.sh/my-topic"
desktop = true

[[alerts]]
webhook = "https://hooks.slack.com/services/xyz"
"#;
        let res = parse(toml_str);
        assert!(res.is_ok());
        let Ok(cfg) = res else { return };
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.alerts.len(), 2);
        assert_eq!(
            cfg.alerts[0].webhook.as_deref(),
            Some("https://ntfy.sh/my-topic")
        );
        assert!(cfg.alerts[0].desktop);
        assert_eq!(
            cfg.alerts[1].webhook.as_deref(),
            Some("https://hooks.slack.com/services/xyz")
        );
        assert!(!cfg.alerts[1].desktop);
    }

    #[test]
    fn parse_alerts_single_table() {
        let toml_str = r#"
[[servers]]
host = "prod-02"

[alerts]
desktop = true
"#;
        let res = parse(toml_str);
        assert!(res.is_ok());
        let Ok(cfg) = res else { return };
        assert_eq!(cfg.alerts.len(), 1);
        assert!(cfg.alerts[0].webhook.is_none());
        assert!(cfg.alerts[0].desktop);
    }
}
