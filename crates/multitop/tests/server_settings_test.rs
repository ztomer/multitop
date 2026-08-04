//! Comprehensive integration tests for Server Settings Manager,
//! keybar visual flare, hotkeys ('e'), and upgrade flow.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crossterm::event::KeyCode;
use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::password_store;
use multitop::passwords::{self, PasswordAction};
use std::sync::atomic::{AtomicU16, Ordering};

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
}

/// `e` opens the panel from the main view and edits a row inside it; Esc and
/// `q` are what leave. `e` used to both open and close it, which cost the panel
/// the obvious key for its main action.
#[test]
fn test_open_and_close_settings_manager() {
    let _keychain = isolate_keychain();
    for leave in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('Q')] {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        assert!(app.password_manager.is_none());

        passwords::open(&mut app, 0, false);
        assert!(app.password_manager.is_some());

        let action = passwords::handle_key(&mut app, leave);
        assert_eq!(action, PasswordAction::None);
        assert!(app.password_manager.is_none(), "{leave:?} must leave");
    }
}

/// Inside the panel, `e` opens the row editor rather than closing the panel.
#[test]
fn test_e_edits_the_selected_row() {
    let _keychain = isolate_keychain();
    for edit in [KeyCode::Enter, KeyCode::Char('e'), KeyCode::Char('E')] {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        passwords::open(&mut app, 0, false);

        let action = passwords::handle_key(&mut app, edit);
        assert_eq!(action, PasswordAction::None);
        let manager = app
            .password_manager
            .as_ref()
            .unwrap_or_else(|| panic!("{edit:?} must not close the panel"));
        assert!(manager.draft.is_some(), "{edit:?} must open the row editor");
    }
}

#[test]
fn test_apply_servers_updates_panels_dynamically() {
    let _keychain = isolate_keychain();
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
    let _keychain = isolate_keychain();
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
        PasswordAction::ApplyServerEdit {
            servers: vec![test_server("host1"), test_server("host2")],
            target_idx: 1,
            password: Some("new_password".to_string()),
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

/// Saving a password must not kill an upgrade already running on that host.
///
/// Found by the audit item 3 asks for. `mode == Upgrade` holds for the whole
/// session once `u` has been pressed, so any password save while the upgrade
/// view was showing took the "resume the upgrade" branch -- which replaces the
/// panel's task and aborts what was there. Every child is spawned with
/// `kill_on_drop`, so that killed the SSH session of a running `apt upgrade`,
/// interrupting a package transaction on the real machine and leaving the
/// remote lock file behind. `execute_cmds` refuses to abort a running upgrade
/// for exactly this reason; this path disagreed with it.
#[tokio::test]
async fn test_saving_a_password_does_not_restart_a_running_upgrade() {
    let _store_guard = setup_mock_store_async().await;

    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;
    app.panels[0].upgrade_state = multitop::panel::UpgradeState::STARTED;
    app.panels[0].upgrade_gen = app.panels[0].gen;
    let gen_before = app.panels[0].gen;

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

    assert_eq!(
        app.panels[0].gen, gen_before,
        "no new generation -- a running upgrade must not be superseded"
    );
    assert!(
        tasks.aux[0].is_none(),
        "and no replacement task may be spawned over it"
    );
    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("mypassword"),
        "the password is still saved; only the restart is refused"
    );
}

/// The resume itself must keep working: an upgrade that stopped for want of a
/// password is exactly what it is for.
#[tokio::test]
async fn test_saving_a_password_resumes_a_finished_upgrade() {
    let _store_guard = setup_mock_store_async().await;

    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;
    app.panels[0].upgrade_state = multitop::panel::UpgradeState::DONE;

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

    assert_eq!(
        app.panels[0].upgrade_state,
        multitop::panel::UpgradeState::STARTED,
        "a run that already finished must resume with the new password"
    );
    assert!(tasks.aux[0].is_some(), "and a task must be spawned for it");
}
