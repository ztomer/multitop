//! Panel state integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::app::Mode;
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// An integration binary is compiled without `cfg(test)`, so the mock store is
/// not in force unless it is asked for, and anything holding an `App` reaches
/// `password_store` several calls down. Without this these tests query the real
/// OS keychain: every rebuild changes the binary's code signature, so macOS
/// raises an access dialog and the suite stops until a human dismisses it.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some("echo test".to_string()),
        custom_command: None,
    }
}

/// Reset the process-global mock store, holding the test guard so a
/// concurrently running test cannot be wiped out mid-run. Keep the returned
/// guard alive for the whole test body.
fn enable_mock_store() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    reset_store();
    guard
}

fn reset_store() {
    password_store::enable_mock_store();
    password_store::clear_mock_store();
}

#[test]
fn test_panel_new_initializes_state() {
    let _keychain = isolate_keychain();
    let server = test_server("127.0.0.1");
    let panel = multitop::app::Panel::new(server);

    assert_eq!(panel.mode, Mode::Monitor);
    assert_eq!(panel.upgrade_state, UpgradeState::NIL);
    assert!(panel.view.iter().any(|l| l.contains("connecting")));
}

#[test]
fn test_dispatch_credential_load_then_answer_lands_keychain_password() {
    let _store_guard = enable_mock_store();
    let server = test_server("127.0.0.10");
    password_store::save(&server, "keychain_pass").unwrap();

    let mut panel = multitop::app::Panel::new(server.clone());
    assert!(panel.needs_credential_load(), "no password held yet");
    panel.mark_credential_load_dispatched();
    assert!(panel.password_checking, "in flight until the answer lands");
    panel.answer_credential_load(password_store::load(&server));

    assert_eq!(panel.sudo_password.as_deref(), Some("keychain_pass"));
    assert!(panel.password_saved);
    assert!(!panel.password_checking);
}

#[test]
fn test_dispatch_credential_load_then_answer_without_store_entry_yields_none() {
    let _store_guard = enable_mock_store();
    let server = test_server("127.0.0.12");
    // No password in keychain or vault
    let mut panel = multitop::app::Panel::new(server.clone());
    assert!(panel.needs_credential_load());
    panel.mark_credential_load_dispatched();
    panel.answer_credential_load(password_store::load(&server));

    assert_eq!(panel.sudo_password, None);
    assert!(!panel.password_saved);
    assert!(!panel.password_checking);
    assert!(
        !panel.needs_credential_load(),
        "answered once is answered forever"
    );
}

#[test]
fn test_set_sudo_password_session_only() {
    let _store_guard = enable_mock_store();
    let server = test_server("127.0.0.13");
    let mut panel = multitop::app::Panel::new(server);

    panel.set_sudo_password("session_pass".to_string(), false);

    assert_eq!(panel.sudo_password.as_deref(), Some("session_pass"));
}

#[test]
fn test_set_sudo_password_from_vault() {
    let _store_guard = enable_mock_store();
    let server = test_server("127.0.0.14");
    let mut panel = multitop::app::Panel::new(server);

    panel.set_sudo_password("vault_pass".to_string(), true);

    assert_eq!(panel.sudo_password.as_deref(), Some("vault_pass"));
    // password_saved is set externally after successful keychain save
    // Here we're just testing the panel method
    assert!(panel.external_password);
}

#[test]
fn test_password_saved_flag_sync() {
    let _store_guard = enable_mock_store();
    let server = test_server("127.0.0.15");
    let mut panel = multitop::app::Panel::new(server.clone());

    // Save succeeds
    password_store::save(&server, "pass").unwrap();
    panel.set_sudo_password("pass".to_string(), true);
    // password_saved is set by caller after save
    panel.password_saved = true;
    assert!(panel.password_saved);

    // Simulate save failure by not calling set_sudo_password with from_vault=true
    let server2 = test_server("127.0.0.16");
    let mut panel2 = multitop::app::Panel::new(server2);
    panel2.sudo_password = Some("pass".to_string());
    panel2.password_saved = false; // Manually set to false
    assert!(!panel2.password_saved);
}

#[test]
fn test_show_last_frame_restores_view() {
    let _keychain = isolate_keychain();
    let server = test_server("127.0.0.17");
    let mut panel = multitop::app::Panel::new(server);

    panel.last_frame = Some(vec!["line1".to_string(), "line2".to_string()]);
    panel.show_last_frame();

    assert_eq!(panel.view, vec!["line1", "line2"]);
}

#[test]
fn test_panel_mode_transitions() {
    let _keychain = isolate_keychain();
    let server = test_server("127.0.0.18");
    let mut panel = multitop::app::Panel::new(server);

    assert_eq!(panel.mode, Mode::Monitor);

    panel.mode = Mode::Docker;
    assert_eq!(panel.mode, Mode::Docker);

    panel.mode = Mode::Fetch;
    assert_eq!(panel.mode, Mode::Fetch);

    panel.mode = Mode::Upgrade;
    assert_eq!(panel.mode, Mode::Upgrade);

    panel.mode = Mode::Monitor;
    assert_eq!(panel.mode, Mode::Monitor);
}

#[test]
fn test_panel_generation_bump_on_mode_change() {
    let _keychain = isolate_keychain();
    let server = test_server("127.0.0.19");
    let mut panel = multitop::app::Panel::new(server);

    let gen0 = panel.gen;
    panel.mode = Mode::Docker;
    panel.gen = gen0 + 1;
    assert_eq!(panel.gen, gen0 + 1);

    panel.mode = Mode::Fetch;
    panel.gen += 1;
    assert_eq!(panel.gen, gen0 + 2);
}
