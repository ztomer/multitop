use super::*;

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
        PasswordAction::ApplyServers(new_servers),
        &mut app,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels[0].server.host, "host1");
    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("secret1"));
    assert_eq!(app.panels[2].server.host, "host3");

    let _ = std::fs::remove_file(tmp_path);
}
