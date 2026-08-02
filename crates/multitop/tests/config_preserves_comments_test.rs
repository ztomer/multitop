//! Writing config.toml must not destroy what the user wrote around the values.
//!
//! Every writer used to round-trip through `toml::Table`, which rebuilds the
//! file from its parsed values. Comments and blank lines are not values, so they
//! vanished. Adding a server, deleting one, stripping a plaintext password, or
//! merely pressing the sparklines toggle was enough to strip a hand-written
//! config -- silently, and to a file the user maintains by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::config::{save_servers, save_show_sparklines, strip_plaintext_passwords, Server};

const ANNOTATED: &str = r#"# multitop configuration -- keep these notes.
theme = "Kare"
show_sparklines = false

# The media server. Reboots on Sundays.
[[servers]]
host = "192.168.0.33"
port = 22
user = "ztomer"
upgrade_cmd = "us;ud"

# Spare box; upgrade_cmd deliberately omitted.
[[servers]]
host = "192.168.0.90"
port = 22
user = "ztomer"
"#;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mt_cfg_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.toml")
}

fn server(host: &str, cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        upgrade_cmd: cmd.map(ToString::to_string),
    }
}

#[test]
fn saving_servers_keeps_comments_and_other_settings() {
    let path = scratch("save");
    std::fs::write(&path, ANNOTATED).unwrap();

    save_servers(
        &path,
        &[
            server("192.168.0.33", Some("us;ud")),
            server("192.168.0.90", None),
            server("192.168.0.158", Some("./update_sys.sh")),
        ],
    )
    .unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("keep these notes"),
        "header comment lost:\n{after}"
    );
    assert!(
        after.contains("Reboots on Sundays"),
        "per-server comment lost:\n{after}"
    );
    assert!(
        after.contains("deliberately omitted"),
        "comment lost:\n{after}"
    );
    assert!(after.contains("theme = \"Kare\""), "theme lost:\n{after}");

    // And the edit itself took effect.
    let cfg = multitop::config::parse(&after).unwrap();
    assert_eq!(cfg.servers.len(), 3);
    assert_eq!(cfg.servers[2].host, "192.168.0.158");
    assert_eq!(cfg.servers[1].upgrade_cmd, None);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn removing_a_server_keeps_the_remaining_comments() {
    let path = scratch("remove");
    std::fs::write(&path, ANNOTATED).unwrap();

    save_servers(&path, &[server("192.168.0.90", None)]).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("keep these notes"),
        "header comment lost:\n{after}"
    );
    let cfg = multitop::config::parse(&after).unwrap();
    assert_eq!(cfg.servers.len(), 1);
    assert_eq!(cfg.servers[0].host, "192.168.0.90");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn toggling_sparklines_keeps_comments() {
    let path = scratch("spark");
    std::fs::write(&path, ANNOTATED).unwrap();

    save_show_sparklines(&path, true);

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("keep these notes"),
        "a keystroke toggle destroyed the config comments:\n{after}"
    );
    assert!(
        after.contains("Reboots on Sundays"),
        "comment lost:\n{after}"
    );
    let cfg = multitop::config::parse(&after).unwrap();
    assert!(cfg.show_sparklines, "the toggle did not take effect");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn stripping_a_plaintext_password_keeps_comments() {
    let path = scratch("strip");
    let with_secret = ANNOTATED.replace(
        "upgrade_cmd = \"us;ud\"",
        "upgrade_cmd = \"us;ud\"\nsudo_password = \"hunter2\"",
    );
    std::fs::write(&path, &with_secret).unwrap();

    assert_eq!(strip_plaintext_passwords(&path).unwrap(), 1);

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        !after.contains("hunter2"),
        "the secret must be gone:\n{after}"
    );
    assert!(
        !after.contains("sudo_password"),
        "the key must be gone:\n{after}"
    );
    assert!(after.contains("keep these notes"), "comment lost:\n{after}");
    assert!(
        after.contains("Reboots on Sundays"),
        "comment lost:\n{after}"
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
