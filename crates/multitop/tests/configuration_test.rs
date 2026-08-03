//! Regression tests for the full-screen configuration and password workflow.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crossterm::event::KeyCode;
use multitop::app::App;
use multitop::config::Server;
use multitop::password_store;
use multitop::passwords::{self, PasswordAction};

/// Divert credentials to the in-memory store, and hold the process-global
/// guard for the test body.
///
/// Without this these tests read the developer's real OS keychain:
/// `passwords::open` calls `Panel::ensure_sudo_password`, which calls
/// `password_store::load`, which falls back to the SSO entry. An integration
/// test binary is compiled without `cfg(test)`, so `is_mock_enabled()` is false
/// unless something says otherwise -- and every rebuild changes the binary's
/// code signature, so macOS puts up a keychain-access prompt and the suite
/// stops dead waiting for a human. Tests must never touch real credentials.
fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "root".to_string(),
        upgrade_cmd: Some("sudo apt update".to_string()),
    }
}

#[test]
fn password_entry_accepts_numeric_characters() {
    let _keychain = isolate();
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, false);
    // Enter opens the row editor; the password is its fifth field.
    let _ = passwords::handle_key(&mut app, KeyCode::Enter);
    for _ in 0..4 {
        let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    }
    for character in "pa55w0rd9".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let manager = app.password_manager.as_ref().expect("configuration open");
    let draft = manager.draft.as_ref().expect("the row editor is open");
    assert_eq!(draft.password, "pa55w0rd9");
}

#[test]
fn server_settings_manager_opens_and_sets_resume_upgrade() {
    let _keychain = isolate();
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, true);
    let manager = app.password_manager.as_ref().expect("settings open");
    assert!(manager.resume_upgrade);
}

#[test]
fn server_editor_creates_a_configured_server() {
    let _keychain = isolate();
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, false);
    let _ = passwords::handle_key(&mut app, KeyCode::Char('a'));
    for character in "new-host".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    for character in "deploy".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    let _ = passwords::handle_key(&mut app, KeyCode::Backspace);
    let _ = passwords::handle_key(&mut app, KeyCode::Backspace);
    for character in "2222".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let action = passwords::handle_key(&mut app, KeyCode::Enter);
    let PasswordAction::ApplyServerEdit {
        servers, password, ..
    } = action
    else {
        panic!("expected server update")
    };
    assert_eq!(password, None, "no password was typed for the new host");
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[1].host, "new-host");
    assert_eq!(servers[1].user, "deploy");
    assert_eq!(servers[1].port, 2222);
}

#[test]
fn test_parse_ssh_config_multi_alias() {
    let _keychain = isolate();
    let ssh_config = r"
Host web db app
    HostName 192.168.1.100
    User admin
    Port 2222
";
    let servers = multitop::config::parse_ssh_config(ssh_config);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].host, "192.168.1.100");
    assert_eq!(servers[0].user, "admin");
    assert_eq!(servers[0].port, 2222);
}

#[test]
fn test_save_new_server_with_password() {
    let _keychain = isolate();
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, false);
    let _ = passwords::handle_key(&mut app, KeyCode::Char('a'));
    for character in "new-server".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    // Tab to user
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    for character in "root".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    // Tab to port
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    // Tab to upgrade_cmd
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    // Tab to password
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    for character in "secret123".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let action = passwords::handle_key(&mut app, KeyCode::Enter);
    let PasswordAction::ApplyServerEdit {
        servers,
        target_idx,
        password,
    } = action
    else {
        panic!("expected a server edit action");
    };
    assert_eq!(servers.len(), 2);
    assert_eq!(target_idx, 1);
    assert_eq!(
        password.as_deref(),
        Some("secret123"),
        "a password typed in the row editor is that host's own"
    );
}
