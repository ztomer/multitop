use super::*;

// ===========================================================================
// handle_key paths (run.rs)
// ===========================================================================

#[test]
fn handle_key_filter_mode() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.set_filtering(true);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // Type a filter query.
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('w'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.filter_query, "w");
}

#[test]
fn handle_key_filter_esc_clears() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.set_filtering(true);
    a.filter_query = "web".into();

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.filter_query, "");
}

#[test]
fn handle_key_number_selects_panel() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![
        test_server("h1"),
        test_server("h2"),
        test_server("h3"),
    ]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(3);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('2'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.selected_panel, 1);
}

#[tokio::test]
async fn handle_key_sort_toggles() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('m'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.sort, multitop_agent::SortBy::Mem);
}

#[test]
fn handle_key_theme_cycle() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_theme.toml"));

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    let theme_before = a.theme_idx;
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('t'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_ne!(a.theme_idx, theme_before);
}

#[test]
fn handle_key_scroll_up_down() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Up, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx.clone()),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.panels[0].scroll_offset, 1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Down, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.panels[0].scroll_offset, 0);
}

#[test]
fn handle_key_page_up_down() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::PageUp, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.panels[0].scroll_offset, 15);
}

#[test]
fn handle_key_home_end() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // Home scrolls to top (max offset).
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Home, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx.clone()),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.panels[0].scroll_offset, usize::MAX);

    // End returns to bottom.
    handle_key(
        KeyEvent::new_with_kind(KeyCode::End, KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert_eq!(a.panels[0].scroll_offset, 0);
}

#[test]
fn handle_key_ctrl_c_quits() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    // Ctrl-C quits directly (no upgrades in flight).
    handle_key(
        KeyEvent::new_with_kind(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        ),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert!(a.should_quit());
}

#[test]
fn handle_key_settings_opens() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('e'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert!(a.password_manager.is_some());
}

#[test]
fn handle_key_slash_starts_filter() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char('/'), KeyModifiers::NONE, KeyEventKind::Press),
        &mut a,
        (80, 24),
        Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
    assert!(a.is_filtering());
}

// ===========================================================================
// run.rs — panel_at_pos, size_change, rerender_all, replace_panels
// ===========================================================================

#[test]
fn panel_at_pos_selects_correct_panel() {
    let _g = isolate_keychain();

    let area = Rect::new(0, 0, 80, 24);
    let shown = [0, 1, 2, 3];
    // Click on panel 1 (top-right quadrant).
    assert_eq!(panel_at_pos(75, 2, area, &shown), Some(1));
    // Click on panel 2 (bottom-left).
    assert_eq!(panel_at_pos(5, 20, area, &shown), Some(2));
}

#[test]
fn panel_at_pos_returns_none_for_gap() {
    let _g = isolate_keychain();

    let area = Rect::new(0, 0, 80, 24);
    let shown = [0, 1, 2]; // odd count → gap at bottom-right
    assert_eq!(panel_at_pos(75, 20, area, &shown), None);
}

#[test]
fn rerender_all_renders_at_new_dims() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    a.panels[0].last_monitor = Some(multitop_agent::proto::Payload::Monitor(Snapshot {
        host: "h1".into(),
        ..Snapshot::default()
    }));

    a.rerender_all((120, 40));
    // After rerender, view is updated (render_payload produces output).
    assert_ne!(a.panels[0].view, [] as [std::string::String; 0]);
}

#[test]
fn replace_panels_carries_credentials() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].sudo_password = Some("secret".into());
    a.panels[0].password_saved = true;

    // Same account (user@host:port) → credential carried to new panel.
    let new_servers = vec![test_server("h1")];
    a.replace_panels(new_servers);
    assert_eq!(a.panels[0].sudo_password.as_deref(), Some("secret"));
    assert!(a.panels[0].password_saved);
}
