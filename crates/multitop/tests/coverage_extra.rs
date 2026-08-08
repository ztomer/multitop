//! Additional coverage tests (2026-08-07)
//!
//! These tests exercise uncovered code paths. Kept separate from
//! `coverage_pure.rs` because they call `App::new` which triggers the
//! keychain-isolation gate for the whole file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::items_after_statements
)]

use multitop::config::Server;
use multitop::password_store;

#[test]
fn port_plaintext_passwords_moves_and_strips() {
    use multitop::app::App;
    use multitop::password_actions::port_plaintext_passwords;

    let _guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();

    let server = Server {
        host: "port-test".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    };
    let mut app = App::new(vec![server.clone()]);
    app.config_path = Some(std::path::PathBuf::from("/tmp/test_config_port.toml"));

    let entries = vec![(server, "testpass".to_string())];
    port_plaintext_passwords(
        &mut app,
        std::path::Path::new("/tmp/test_config_port.toml"),
        &entries,
    );

    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("testpass"));
    assert!(app.panels[0].password_saved);
}

#[test]
fn apply_cycle_banner_style_no_config() {
    use multitop::app::App;
    use multitop::password_actions::apply;
    use multitop::passwords::PasswordAction;
    use tokio::sync::mpsc;

    let _guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();

    let server = Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    };
    let mut app = App::new(vec![server]);

    let (tx, _rx) = mpsc::channel(16);
    apply(
        PasswordAction::CycleBannerStyle,
        &mut app,
        &tx,
        &mut multitop::run::Tasks::new(1),
    );
}

#[test]
fn apply_import_ssh_hosts_no_file() {
    use multitop::app::App;
    use multitop::password_actions::apply;
    use multitop::passwords::PasswordAction;
    use tokio::sync::mpsc;

    let _guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();

    let server = Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    };
    let mut app = App::new(vec![server]);
    app.config_path = Some(std::path::PathBuf::from("/tmp/test_config_import.toml"));

    let (tx, _rx) = mpsc::channel(16);
    apply(
        PasswordAction::ImportSshHosts,
        &mut app,
        &tx,
        &mut multitop::run::Tasks::new(1),
    );
}
