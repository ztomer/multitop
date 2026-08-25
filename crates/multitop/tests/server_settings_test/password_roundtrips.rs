use super::*;

#[tokio::test]
async fn test_save_password_in_upgrade_mode_triggers_upgrade_resume() {
    let _store_guard = setup_mock_store_async().await;

    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "mypassword".to_string(),
            resume_upgrade: false,
        },
        &mut app,
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

    // Panel in Monitor mode, save with resume_upgrade=false
    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "pass".to_string(),
            resume_upgrade: false,
        },
        &mut app,
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
        PasswordAction::ApplyServers(vec![s1, s2, s3]),
        &mut app,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("secret1"));
    assert_eq!(app.panels[1].sudo_password.as_deref(), Some("secret2"));
    assert_eq!(app.panels[2].sudo_password, None); // new server has no password

    let _ = std::fs::remove_file(tmp_path);
}
