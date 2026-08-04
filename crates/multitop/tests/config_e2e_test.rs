//! Config & SSH config parsing integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::config::{load, parse, parse_ssh_config, save_servers, Server};
use multitop::ssh::is_local;
use std::fs;
use tempfile::TempDir;

fn test_server(host: &str, user: &str, port: u16, upgrade_cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        user: user.to_string(),
        port,
        upgrade_cmd: upgrade_cmd.map(String::from),
    }
}

#[test]
fn test_config_load_valid_toml() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let toml = r#"
theme = "kare"
show_sparklines = true
upgrade_history_lines = 1000

[[servers]]
host = "server1"
port = 22
user = "admin"
upgrade_cmd = "apt update"

[[servers]]
host = "server2"
port = 2222
user = ""
upgrade_cmd = "yum update"
"#;
    fs::write(&config_path, toml).unwrap();

    let config = load(&config_path).unwrap();
    assert_eq!(config.theme, Some("kare".to_string()));
    assert_eq!(config.upgrade_history_lines, 1000);
    assert_eq!(config.servers.len(), 2);
    assert_eq!(config.servers[0].host, "server1");
    assert_eq!(config.servers[0].port, 22);
    assert_eq!(config.servers[0].user, "admin");
    assert_eq!(config.servers[1].port, 2222);
}

#[test]
fn test_config_load_missing_file() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("nonexistent.toml");

    let result = load(&config_path);
    // load() returns error when file doesn't exist
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Configuration file missing"));
}

#[test]
fn test_config_load_malformed_toml() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("bad.toml");

    fs::write(&config_path, "not valid toml [[[").unwrap();

    let result = load(&config_path);
    assert!(result.is_err());
}

#[test]
fn test_config_save_servers_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let servers = vec![
        test_server("host1", "user1", 22, Some("cmd1")),
        test_server("host2", "user2", 2222, None),
    ];

    save_servers(&config_path, &servers).unwrap();

    let content = fs::read_to_string(&config_path).unwrap();
    let config = parse(&content).unwrap();

    assert_eq!(config.servers.len(), 2);
    assert_eq!(config.servers[0].host, "host1");
    assert_eq!(config.servers[0].user, "user1");
    assert_eq!(config.servers[0].port, 22);
    assert_eq!(config.servers[0].upgrade_cmd, Some("cmd1".to_string()));
    assert_eq!(config.servers[1].host, "host2");
    assert_eq!(config.servers[1].upgrade_cmd, None);
}

#[test]
fn test_ssh_config_parse_multiple_hosts() {
    let ssh_config = r"
Host server1
    HostName 192.168.1.10
    User admin
    Port 22

Host server2
    HostName 192.168.1.20
    User root
    Port 2222

Host *
    User default
";
    let servers = parse_ssh_config(ssh_config);
    eprintln!("Parsed servers: {servers:?}");
    // Wildcard hosts are skipped, so only 2 concrete hosts
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].host, "192.168.1.10");
    assert_eq!(servers[0].user, "admin");
    assert_eq!(servers[0].port, 22);
    assert_eq!(servers[1].host, "192.168.1.20");
    assert_eq!(servers[1].user, "root");
    assert_eq!(servers[1].port, 2222);
}

#[test]
fn test_ssh_config_parse_wildcards() {
    let ssh_config = r"
Host *.example.com
    User wildcard_user
    Port 22

Host *
    User default_user
";
    let servers = parse_ssh_config(ssh_config);
    // Wildcard hosts without HostName are skipped
    assert_eq!(servers.len(), 0);
}

#[test]
fn test_ssh_config_parse_real_file() {
    let ssh_config = r"
# Comment
Host github.com
    HostName github.com
    User git
    Port 22
    IdentityFile ~/.ssh/id_ed25519

Host myserver
    HostName 10.0.0.5
    User deploy
    Port 22
";
    let servers = parse_ssh_config(ssh_config);
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].host, "github.com");
    assert_eq!(servers[0].user, "git");
    assert_eq!(servers[1].host, "10.0.0.5");
    assert_eq!(servers[1].user, "deploy");
}

#[test]
fn test_config_path_precedence() {
    // This tests the precedence logic in config.rs
    // --config flag > MULTITOP_CONFIG env > default > legacy
    use multitop::config::{default_config_path, legacy_config_path};

    let default = default_config_path();
    let legacy = legacy_config_path();

    eprintln!("default: {default:?}");
    eprintln!("legacy: {legacy:?}");

    assert!(default.to_string_lossy().contains("multitop"));
    assert!(legacy.to_string_lossy().contains("monitor")); // old name
    assert_ne!(default, legacy);
}

#[test]
fn test_server_deduplication() {
    let s1 = test_server("127.0.0.1", "user1", 0, None);
    let s2 = test_server("localhost", "user2", 22, None);
    let s3 = test_server("192.168.1.1", "user3", 22, None);

    let mut servers = vec![s1, s2, s3];
    let mut seen_local = false;
    servers.retain(|s| {
        if is_local(s) {
            if seen_local {
                false
            } else {
                seen_local = true;
                true
            }
        } else {
            true
        }
    });

    assert_eq!(servers.len(), 2);
    assert!(is_local(&servers[0]));
    assert!(!is_local(&servers[1]));
}

#[test]
fn test_validate_host_valid() {
    use multitop::config::validate_host;
    assert!(validate_host("example.com").is_ok());
    assert!(validate_host("192.168.1.1").is_ok());
    assert!(validate_host("localhost").is_ok());
    assert!(validate_host("server-1").is_ok());
    assert!(validate_host("server_1").is_ok());
    assert!(validate_host("xn--bcher-kva.ch").is_ok()); // Unicode domain
}

#[test]
fn test_validate_host_rejects_invalid() {
    use multitop::config::validate_host;
    // Empty is allowed (validates as ok)
    assert!(validate_host("").is_ok());
    assert!(validate_host(&"a".repeat(254)).is_ok()); // length not checked
    assert!(validate_host("host with spaces").is_err());
    assert!(validate_host("host\nnewline").is_err());
}

#[test]
fn test_validate_user_valid() {
    use multitop::config::validate_user;
    assert!(validate_user("user").is_ok());
    assert!(validate_user("user_name").is_ok());
    assert!(validate_user("user-name").is_ok());
    assert!(validate_user("").is_ok()); // empty is valid (means default)
}

#[test]
fn test_validate_user_rejects_invalid() {
    use multitop::config::validate_user;
    // Length not checked, only whitespace
    assert!(validate_user(&"a".repeat(33)).is_ok());
    assert!(validate_user("user@domain").is_ok()); // @ not rejected
    assert!(validate_user("user\ntab").is_err());
}

#[test]
fn test_server_target_with_user_port() {
    let s = test_server("example.com", "user", 2222, None);
    // target() only includes user@host, not port
    assert_eq!(s.target(), "user@example.com");
}

#[test]
fn test_server_target_without_user() {
    let s = test_server("example.com", "", 2222, None);
    assert_eq!(s.target(), "example.com");
}

#[test]
fn test_server_target_default_port() {
    let s = test_server("example.com", "user", 22, None);
    assert_eq!(s.target(), "user@example.com");
}

#[test]
fn test_parse_valid_toml_structure() {
    let toml = r#"
theme = "kare"
show_sparklines = false
upgrade_history_lines = 5000

[[servers]]
host = "test"
port = 22
user = "admin"
upgrade_cmd = "sudo apt upgrade"
"#;
    let config = multitop::config::parse(toml).unwrap();
    assert_eq!(config.theme, Some("kare".to_string()));
    assert_eq!(config.upgrade_history_lines, 5000);
    assert_eq!(config.servers.len(), 1);
    assert_eq!(config.servers[0].host, "test");
}

/// A config file written by an older version still loads.
///
/// `show_sparklines` was a real key until sparklines were deleted. Every user
/// who ever toggled it has it written into their `config.toml`, and the parser
/// must ignore it rather than refuse the file -- a removed feature that takes
/// the user's whole configuration with it is a worse defect than the feature
/// was. The same holds for any key a future version retires.
#[test]
fn a_retired_config_key_does_not_break_the_file_that_still_has_it() {
    let toml = r#"
theme = "kare"
show_sparklines = true
some_key_no_version_ever_had = 42
upgrade_history_lines = 1234

[[servers]]
host = "web-01"
port = 22
user = "admin"
upgrade_cmd = "sudo apt upgrade"
"#;
    let config = multitop::config::parse(toml).expect("a retired key must not fail the parse");
    assert_eq!(config.theme, Some("kare".to_string()));
    assert_eq!(config.upgrade_history_lines, 1234);
    assert_eq!(config.servers.len(), 1);
}

/// `upgrade_history_lines = 0` must not be obeyed.
///
/// The Upgrade pane is composed from the same ring as the history, and
/// `RingLines::push` on a zero-capacity ring is a silent no-op -- so obeying it
/// meant the pane showed nothing for the whole of an upgrade, and the
/// completion note, the sudo-refused warning and the held-lock warning naming
/// the file to remove were all dropped with nothing saying why.
///
/// `RingLines::from` was fixed for this exact state in an earlier round, when
/// an empty fixture set a capacity of zero. The config file was the other door
/// into it, and it was still open.
#[test]
fn a_zero_history_setting_is_raised_rather_than_silently_swallowing_the_log() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        "upgrade_history_lines = 0\n\n\
         [[servers]]\nhost = \"server1\"\nport = 22\nuser = \"admin\"\n",
    )
    .unwrap();

    let config = load(&config_path).unwrap();

    assert!(
        config.upgrade_history_lines >= multitop::config::MIN_UPGRADE_HISTORY_LINES,
        "a ring that swallows every line is not a setting: got {}",
        config.upgrade_history_lines
    );
    assert_eq!(
        config.history_lines_raised_from,
        Some(0),
        "and the substitution must be reported, not made silently"
    );

    // The point of the floor, proven where it matters: a line pushed into a
    // ring built at this capacity survives.
    let mut ring = multitop::panel::RingLines::new(config.upgrade_history_lines);
    ring.push("upgrade finished".to_string());
    assert_eq!(
        ring.last().map(String::as_str),
        Some("upgrade finished"),
        "the pane must be able to show what an upgrade said"
    );
}

/// A value the user actually chose is left alone, and reports no substitution.
#[test]
fn a_usable_history_setting_is_used_as_written() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        "upgrade_history_lines = 200\n\n\
         [[servers]]\nhost = \"server1\"\nport = 22\nuser = \"admin\"\n",
    )
    .unwrap();

    let config = load(&config_path).unwrap();
    assert_eq!(config.upgrade_history_lines, 200);
    assert_eq!(config.history_lines_raised_from, None);
}
