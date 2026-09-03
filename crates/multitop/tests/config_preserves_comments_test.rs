//! Writing config.toml must not destroy what the user wrote around the values.
//!
//! Every writer used to round-trip through `toml::Table`, which rebuilds the
//! file from its parsed values. Comments and blank lines are not values, so they
//! vanished. Adding a server, deleting one, stripping a plaintext password, or
//! merely pressing a settings toggle was enough to strip a hand-written
//! config -- silently, and to a file the user maintains by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::config::{save_servers, strip_plaintext_passwords, Server};

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
        custom_command: None,
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

/// Cycling the theme must not strip the config file.
///
/// `save_banner_style`, the function immediately below `save_theme`, carries a
/// doc comment saying exactly this: "Round-tripping through `toml::Table`
/// rebuilds the file from its values, so every comment and blank line the user
/// wrote disappears -- and this runs on a keystroke, which means one press of a
/// display toggle was enough to strip a hand-written config."
///
/// It was fixed there and not here. `save_theme` does the identical job on the
/// identical file, reached by a different keystroke (`t`), and still parsed to
/// `toml::Table` and re-serialised.
#[test]
fn cycling_the_theme_keeps_the_users_comments() {
    let dir = std::env::temp_dir().join(format!("multitop_theme_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.toml");
    let original = r#"# multitop configuration
# Keep this file readable: it is edited by hand.

theme = "kare"

# The production box. Do not remove.
[[servers]]
host = "web-01"
port = 22
user = "deploy"
"#;
    std::fs::write(&path, original).unwrap();

    multitop::config::save_theme(&path, "dracula");

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("dracula"),
        "the theme must actually be saved: {after}"
    );
    assert!(
        after.contains("# multitop configuration"),
        "the header comment must survive a keystroke: {after}"
    );
    assert!(
        after.contains("# The production box. Do not remove."),
        "and so must the comment the user attached to a server: {after}"
    );
    assert!(
        after.contains("edited by hand"),
        "every comment, not just the first: {after}"
    );
}

/// Writing the config must never be able to truncate it.
///
/// `state.rs` grew `write_atomic` because a truncating write does not merely
/// fail to record the new value, it destroys the old one -- and then the config
/// writers, which run on a *keystroke* and target the file the user maintains by
/// hand, kept using `std::fs::write`. That is strictly the worse of the two
/// files to lose.
///
/// Checked by the observable consequence: a failed write leaves the previous
/// contents intact, and leaves no scratch file behind.
#[test]
fn a_config_write_that_fails_leaves_the_old_file_intact() {
    let dir = std::env::temp_dir().join(format!("multitop_atomic_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.toml");
    let original = "# hand written\ntheme = \"kare\"\n\n[[servers]]\nhost = \"web-01\"\nport = 22\nuser = \"deploy\"\n";
    std::fs::write(&path, original).unwrap();

    // Make the directory read-only so the temporary file cannot be created.
    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        perms.set_mode(0o500);
    }
    std::fs::set_permissions(&dir, perms).unwrap();

    multitop::config::save_theme(&path, "dracula");

    // Put it back before asserting, so a failure cannot leave an unwritable dir.
    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        perms.set_mode(0o700);
    }
    std::fs::set_permissions(&dir, perms).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after, original,
        "a write that could not happen must leave the config exactly as it was"
    );
    let strays: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "config.toml")
        .collect();
    assert!(
        strays.is_empty(),
        "no scratch file may be left behind: {strays:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
