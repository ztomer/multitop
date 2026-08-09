use crate::config::{Server, DEFAULT_PORT};

#[must_use]
pub fn ssh_config_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("MULTITOP_SSH_CONFIG") {
        return Some(std::path::PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|home| {
        let mut p = std::path::PathBuf::from(home);
        p.push(".ssh");
        p.push("config");
        p
    })
}

#[must_use]
pub fn merge_ssh_hosts(existing: &[Server], imported: Vec<Server>) -> (Vec<Server>, usize) {
    let key = |s: &Server| format!("{}@{}:{}", s.user, s.host, s.port);
    let known: std::collections::HashSet<String> = existing.iter().map(&key).collect();

    let mut merged = existing.to_vec();
    let mut added = 0;
    let mut seen = known;
    for server in imported {
        if seen.insert(key(&server)) {
            merged.push(server);
            added += 1;
        }
    }
    (merged, added)
}

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
