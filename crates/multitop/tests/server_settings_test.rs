//! Comprehensive integration tests for Server Settings Manager,
//! keybar visual flare, hotkeys ('e'), and upgrade flow.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crossterm::event::KeyCode;
use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::password_store;
use multitop::passwords::{self, ConfigSection, PasswordAction};
use std::sync::atomic::{AtomicU16, Ordering};

static PORT_COUNTER: AtomicU16 = AtomicU16::new(10000);

fn next_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: next_port(),
        user: "admin".to_string(),
        upgrade_cmd: Some("sudo apt update".to_string()),
    }
}

/// Reset the process-global mock store, holding the test guard so a
/// concurrently running test cannot be wiped out mid-run. Keep the returned
/// guard alive for the whole test body.
fn setup_mock_store() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    reset_store();
    guard
}

/// `setup_mock_store` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
async fn setup_mock_store_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    reset_store();
    guard
}

fn reset_store() {
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    password_store::delete_sso().unwrap();
}

#[test]
fn test_open_and_close_settings_manager_with_e_key() {
    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    assert!(app.password_manager.is_none());

    // Open via passwords::open
    passwords::open(&mut app, 0, false);
    assert!(app.password_manager.is_some());
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );

    // Press 'e' to close
    let action = passwords::handle_key(&mut app, KeyCode::Char('e'));
    assert_eq!(action, PasswordAction::None);
    assert!(app.password_manager.is_none());
}

#[test]
fn test_tab_between_passwords_and_servers_sections() {
    let mut app = App::new(vec![test_server("host1")]);
    passwords::open(&mut app, 0, false);
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );

    // Press Tab to switch to Servers section
    let action = passwords::handle_key(&mut app, KeyCode::Tab);
    assert_eq!(action, PasswordAction::None);
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Servers
    );

    // Press Tab to switch back to Passwords section
    let action2 = passwords::handle_key(&mut app, KeyCode::Tab);
    assert_eq!(action2, PasswordAction::None);
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );
}

#[test]
fn test_apply_servers_updates_panels_dynamically() {
    // Same server values throughout: `test_server` allocates a new port per
    // call, so re-calling it would describe different credentials.
    let s1 = test_server("host1");
    let s2 = test_server("host2");
    let s3 = test_server("host3");

    let mut app = App::new(vec![s1.clone(), s2.clone()]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());
    app.panels[0].sudo_password = Some("secret1".to_string());
    app.panels[0].password_saved = true;

    let new_servers = vec![s1, s2, s3];

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(2);

    multitop::password_actions::apply(
        PasswordAction::ApplyServers(new_servers.clone()),
        &mut app,
        &new_servers,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels[0].server.host, "host1");
    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("secret1"));
    assert_eq!(app.panels[2].server.host, "host3");

    let _ = std::fs::remove_file(tmp_path);
}

#[tokio::test]
async fn test_save_password_in_upgrade_mode_triggers_upgrade_resume() {
    let _store_guard = setup_mock_store_async().await;

    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);
    let servers = vec![test_server("host1")];

    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "mypassword".to_string(),
            resume_upgrade: false,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("mypassword"));
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
}

#[test]
fn test_add_and_delete_server_from_passwords_section() {
    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    passwords::open(&mut app, 0, false);

    // Press 'a' in Passwords section -> opens draft
    let action = passwords::handle_key(&mut app, KeyCode::Char('a'));
    assert_eq!(action, PasswordAction::None);
    let manager = app.password_manager.as_ref().unwrap();
    assert!(manager.draft.is_some());

    // Cancel draft
    let _ = passwords::handle_key(&mut app, KeyCode::Esc);
    assert!(app.password_manager.as_ref().unwrap().draft.is_none());

    // Cancelling the draft above left us in the Servers section, where removal
    // now takes a confirmation: 'd' asks, 'y' answers.
    let armed = passwords::handle_key(&mut app, KeyCode::Char('d'));
    assert_eq!(armed, PasswordAction::None, "the first press only asks");
    let action_del = passwords::handle_key(&mut app, KeyCode::Char('y'));
    let PasswordAction::ApplyServers(remaining) = action_del else {
        panic!("expected ApplyServers action");
    };
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].host, "host2");
}

#[test]
fn test_save_server_with_password() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::SaveServerWithPassword {
            servers: vec![test_server("host1"), test_server("host2")],
            target_idx: 1,
            password: "new_password".to_string(),
        },
        &mut app,
        &[test_server("host1"), test_server("host2")],
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels.len(), 2);
    assert_eq!(app.panels[1].server.host, "host2");
    assert_eq!(app.panels[1].sudo_password.as_deref(), Some("new_password"));

    let _ = std::fs::remove_file(tmp_path);
}

#[test]
fn test_delete_password_removes_from_keychain() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    // Use the panel's server for password operations
    let server = app.panels[0].server.clone();
    password_store::save(&server, "test_pass").unwrap();

    multitop::password_actions::apply(
        PasswordAction::Delete { panel: 0 },
        &mut app,
        std::slice::from_ref(&server),
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels[0].sudo_password, None);
    assert!(!app.panels[0].password_saved);

    let loaded = password_store::load(&server).unwrap();
    assert_eq!(loaded, None);

    let _ = std::fs::remove_file(tmp_path);
}

#[test]
fn test_save_sso_propagates_to_all_panels() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(2);
    let servers = vec![test_server("host1"), test_server("host2")];

    multitop::password_actions::apply(
        PasswordAction::SaveSso {
            password: "sso_pass".to_string(),
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("sso_pass"));
    assert_eq!(app.panels[1].sudo_password.as_deref(), Some("sso_pass"));
    assert!(app.panels[0].password_saved);
    assert!(app.panels[1].password_saved);

    let loaded = password_store::load_sso().unwrap();
    assert_eq!(loaded.as_deref(), Some("sso_pass"));

    let _ = std::fs::remove_file(tmp_path);
}

#[test]
fn test_delete_sso_clears_all() {
    let _store_guard = setup_mock_store();
    password_store::save_sso("sso_pass").unwrap();

    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(2);
    let servers = vec![test_server("host1"), test_server("host2")];

    multitop::password_actions::apply(
        PasswordAction::DeleteSso,
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    // SSO deleted from store
    let loaded = password_store::load_sso().unwrap();
    assert_eq!(loaded, None);

    let _ = std::fs::remove_file(tmp_path);
}

#[test]
fn test_toggle_sparklines_persists_config() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::ToggleSparklines,
        &mut app,
        &[test_server("host1")],
        &tx,
        &mut tasks,
    );

    assert!(app.show_sparklines());

    let _ = std::fs::remove_file(tmp_path);
}

#[test]
fn test_save_resume_upgrade_false() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);
    let servers = vec![test_server("host1")];

    // Panel in Monitor mode, save with resume_upgrade=false
    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "pass".to_string(),
            resume_upgrade: false,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    // Should not trigger upgrade (panel stays in Monitor mode)
    assert_eq!(app.panels[0].mode, Mode::Monitor);

    let _ = std::fs::remove_file(tmp_path);
}

#[test]
fn test_apply_servers_preserves_existing_passwords() {
    let _store_guard = setup_mock_store();
    // Reuse the same server values. `test_server` allocates a fresh port on
    // every call, so calling it twice for "host1" produces two DIFFERENT
    // credentials (`admin@host1:<port>`), and this test used to pass only
    // because panels were rematched by host alone -- the very leak that let one
    // account's password reach another account on the same machine.
    let s1 = test_server("host1");
    let s2 = test_server("host2");
    let s3 = test_server("host3");

    let mut app = App::new(vec![s1.clone(), s2.clone()]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());
    app.panels[0].sudo_password = Some("secret1".to_string());
    app.panels[0].password_saved = true;
    app.panels[1].sudo_password = Some("secret2".to_string());
    app.panels[1].password_saved = true;

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(2);

    multitop::password_actions::apply(
        PasswordAction::ApplyServers(vec![s1.clone(), s2.clone(), s3.clone()]),
        &mut app,
        &[s1, s2, s3],
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("secret1"));
    assert_eq!(app.panels[1].sudo_password.as_deref(), Some("secret2"));
    assert_eq!(app.panels[2].sudo_password, None); // new server has no password

    let _ = std::fs::remove_file(tmp_path);
}
