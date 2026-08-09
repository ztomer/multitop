//! The vault must come into existence on its own, and plaintext passwords in
//! `config.toml` must not survive.
//!
//! Before this, `Vault::initialize` was only ever called from tests: there was
//! no path through the app that created a vault, so "unlock the vault once
//! instead of typing a password per host" was unreachable. And a
//! `sudo_password` key in config.toml was parsed by nothing at all, leaving a
//! plaintext secret on disk that did not even work.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::app::{App, VaultState};
use multitop::config::{self, Server};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// Driving an `App` reaches `password_store` several calls down, and an
/// integration binary is compiled without `cfg(test)`, so the mock is not in
/// force unless it is asked for. Without this these tests query the real OS
/// keychain: every rebuild changes the binary's code signature, so macOS raises
/// an access dialog and the suite stops until a human dismisses it -- and a test
/// can read, overwrite or delete credentials the user depends on.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// `isolate_keychain` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        upgrade_cmd: Some("apt upgrade".into()),
    }
}

// ---------------------------------------------------------------------------
// Plaintext passwords in config.toml
// ---------------------------------------------------------------------------

const WITH_SECRETS: &str = r#"
theme = "Kare"

[[servers]]
host = "192.168.0.33"
port = 22
user = "ztomer"
sudo_password = "hunter2"
upgrade_cmd = "apt upgrade"

[[servers]]
host = "192.168.0.90"
port = 22
user = "ztomer"
upgrade_cmd = "apt upgrade"
"#;

#[test]
fn plaintext_passwords_are_surfaced_not_silently_ignored() {
    let _keychain = isolate_keychain();
    let cfg = config::parse(WITH_SECRETS).unwrap();
    assert_eq!(
        cfg.plaintext_passwords.len(),
        1,
        "the one sudo_password must be reported so it can be moved and deleted"
    );
    let (server, secret) = &cfg.plaintext_passwords[0];
    assert_eq!(server.host, "192.168.0.33");
    assert_eq!(server.user, "ztomer", "must carry the user for the key");
    assert_eq!(secret, "hunter2");

    // The server list itself is unaffected.
    assert_eq!(cfg.servers.len(), 2);
}

#[test]
fn config_without_secrets_reports_none() {
    let _keychain = isolate_keychain();
    let cfg = config::parse(
        r#"
[[servers]]
host = "a"
user = "ztomer"
"#,
    )
    .unwrap();
    assert_eq!(
        cfg.plaintext_passwords,
        [] as [(multitop::config::Server, std::string::String); 0]
    );
}

#[test]
fn stripping_removes_the_secret_and_keeps_everything_else() {
    let _keychain = isolate_keychain();
    let dir = std::env::temp_dir().join(format!("multitop_strip_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, WITH_SECRETS).unwrap();

    let removed = config::strip_plaintext_passwords(&path).unwrap();
    assert_eq!(removed, 1);

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("sudo_password"),
        "the key must be gone: {text}"
    );
    assert!(!text.contains("hunter2"), "the secret must be gone: {text}");

    // Everything else survives the rewrite.
    let cfg = config::parse(&text).unwrap();
    assert_eq!(cfg.servers.len(), 2);
    assert_eq!(cfg.servers[0].host, "192.168.0.33");
    assert_eq!(cfg.servers[0].user, "ztomer");
    assert_eq!(cfg.servers[0].upgrade_cmd.as_deref(), Some("apt upgrade"));
    assert_eq!(cfg.theme.as_deref(), Some("Kare"));
    assert_eq!(
        cfg.plaintext_passwords,
        [] as [(multitop::config::Server, std::string::String); 0]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stripping_is_idempotent() {
    let _keychain = isolate_keychain();
    let dir = std::env::temp_dir().join(format!("multitop_strip2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, WITH_SECRETS).unwrap();

    assert_eq!(config::strip_plaintext_passwords(&path).unwrap(), 1);
    assert_eq!(
        config::strip_plaintext_passwords(&path).unwrap(),
        0,
        "a second pass must find nothing and rewrite nothing"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Vault creation
// ---------------------------------------------------------------------------

#[test]
fn saving_a_password_with_no_vault_starts_vault_creation() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![server("web-01")]);
    app.config_path = Some(std::path::PathBuf::from("/tmp/multitop-x/config.toml"));
    assert!(app.vault.is_none());

    assert!(app.begin_vault_creation(), "creation must be offered");
    assert!(app.vault_creating());
    assert!(matches!(app.vault_state, VaultState::Creating { .. }));
}

#[test]
fn vault_creation_is_not_offered_when_one_exists() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![server("web-01")]);
    app.config_path = Some(std::path::PathBuf::from("/tmp/multitop-x/config.toml"));
    // Simulate an existing vault by handing the app a vault handle.
    let dir = std::env::temp_dir().join(format!("multitop_vault_exists_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let vp = dir.join("vault.bin");
    std::fs::write(&vp, b"placeholder").unwrap();
    app.config_path = Some(dir.join("config.toml"));
    app.vault =
        multitop::vault::create_vault(&app.config_path.clone().unwrap()).map(std::sync::Arc::new);
    assert!(app.vault.is_some(), "precondition: a vault file is present");

    assert!(
        !app.begin_vault_creation(),
        "must not offer to create a second vault"
    );
    assert!(!app.vault_creating());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vault_creation_needs_somewhere_to_put_the_file() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![server("web-01")]);
    app.config_path = None;
    assert!(
        !app.begin_vault_creation(),
        "no config path means nowhere to create a vault"
    );
}

#[test]
fn vault_path_sits_beside_the_config() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![server("web-01")]);
    app.config_path = Some(std::path::PathBuf::from(
        "/home/x/.config/multitop/config.toml",
    ));
    assert_eq!(
        app.vault_path().unwrap(),
        std::path::PathBuf::from("/home/x/.config/multitop/vault.bin")
    );
}

#[tokio::test]
async fn a_created_vault_can_be_unlocked_and_holds_passwords() {
    let _keychain = isolate_keychain_async().await;
    // End-to-end on the real vault: create, store, lock, reopen, read back.
    let dir = std::env::temp_dir().join(format!("multitop_vault_new_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let vault_path = dir.join("vault.bin");

    let vault = multitop_vault::Vault::new(multitop_vault::VaultConfig {
        vault_path: vault_path.clone(),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    });
    vault.initialize("master-pw").await.unwrap();
    assert!(vault_path.exists(), "creation must produce the vault file");

    let mut unlocked = vault.unlock_with_password("master-pw").unwrap();
    unlocked
        .set_password(
            "ztomer@192.168.0.33:22".to_string(),
            &secrecy::SecretString::from("sudo-pw".to_string()),
        )
        .unwrap();
    unlocked.lock();

    let reopened = vault.unlock_with_password("master-pw").unwrap();
    let got = reopened.get_password("ztomer@192.168.0.33:22").unwrap();
    assert_eq!(secrecy::ExposeSecret::expose_secret(&got), "sudo-pw");

    let _ = std::fs::remove_dir_all(&dir);
}
