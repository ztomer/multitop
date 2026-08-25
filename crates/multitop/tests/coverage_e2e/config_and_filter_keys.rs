use super::*;

// ===========================================================================
// config.rs — load/save servers
// ===========================================================================

#[test]
fn config_save_and_load_servers_roundtrip() {
    let _g = isolate_keychain();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let servers = vec![
        Server {
            host: "web-01".into(),
            port: 22,
            user: "admin".into(),
            upgrade_cmd: Some("true".into()),
        },
        Server {
            host: "db-01".into(),
            port: 22,
            user: "admin".into(),
            upgrade_cmd: Some("true".into()),
        },
    ];
    multitop::config::save_servers(&path, &servers).expect("save ok");

    // Servers are loaded as part of Config.
    let loaded = multitop::config::load(&path).expect("load ok");
    assert_eq!(loaded.servers.len(), 2);
    assert_eq!(loaded.servers[0].host, "web-01");
}

// ===========================================================================
// passwords.rs — handle_key in various modes
// ===========================================================================

#[test]
fn passwords_handle_key_edit_opens_draft() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    open(&mut a, 0, false);

    let action = passwords_handle_key(&mut a, KeyCode::Char('e'));
    assert!(matches!(action, PasswordAction::None));
    assert!(a.password_manager.as_ref().unwrap().draft.is_some());
}

#[test]
fn passwords_handle_key_quit_closes() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    open(&mut a, 0, false);

    passwords_handle_key(&mut a, KeyCode::Esc);
    assert!(a.password_manager.is_none(), "Esc closes settings");
}

// ===========================================================================
// run.rs — more handle_key paths (upgrade confirm, vault unlock, filter enter)
// ===========================================================================

#[test]
fn handle_key_upgrade_confirm_u() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_upgrade_confirm.toml"));
    a.panels[0].mode = Mode::Upgrade;

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // First u enters upgrade view (already there), second u starts vault unlock
    // or shows modal.
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('u'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    // No vault → shows upgrade modal.
    assert!(a.show_upgrade_modal(), "second u shows upgrade modal");
}

#[tokio::test]
async fn handle_key_upgrade_modal_confirms() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_modal_confirm.toml"));
    a.set_show_upgrade_modal(true);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // Press u to confirm from modal.
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('u'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    // Modal dismissed, upgrade started.
    assert!(!a.show_upgrade_modal());
}

#[test]
fn handle_key_filter_enter_keeps_query() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.set_filtering(true);
    a.filter_query = "web".into();

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    // Enter keeps the query but exits editing mode.
    assert!(!a.is_filtering());
    assert_eq!(a.filter_query, "web");
}

#[test]
fn handle_key_esc_clears_filter() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.filter_query = "stale".into();

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // Esc with a non-empty filter clears it (doesn't quit).
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert!(a.filter_query.is_empty(), "Esc clears filter first");
    assert!(!a.should_quit(), "Esc with filter doesn't quit");
}

#[test]
fn handle_key_esc_with_filter_clears_filter() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.filter_query = "active".into();

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // Esc with a non-empty filter clears it instead of quitting.
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert!(a.filter_query.is_empty(), "Esc clears filter first");
    assert!(!a.should_quit());
}

#[test]
fn handle_key_q_quits_when_no_filter() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // q with no filter quits directly.
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert!(a.should_quit(), "q with no filter quits");
}
