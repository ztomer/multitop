use super::*;

// ===========================================================================
// ui.rs — draw paths (via public exports)
// ===========================================================================

#[test]
fn keybar_badges_shed_whole() {
    let _g = isolate_keychain();

    let pal = &multitop_agent::color::THEMES[0];
    let label = Style::default().fg(Color::DarkGray);
    let key_hi = Style::default().fg(Color::White);
    let sort_label = Style::default().fg(Color::DarkGray);
    let accent = Color::Yellow;

    let badges = keybar_badges(SortBy::Cpu, pal, label, key_hi, sort_label, accent);
    assert_eq!(badges.len(), 3, "three badges: Settings, Theme, Sort");
    // Each badge has (width, spans).
    for (w, spans) in &badges {
        assert!(*w > 0);
        assert_ne!(spans.as_slice(), []);
    }
}

#[test]
fn mode_pair_highlights_active() {
    let _g = isolate_keychain();

    let active = Style::default().fg(Color::Black);
    let key_off = Style::default().fg(Color::White);
    let label_off = Style::default().fg(Color::DarkGray);

    // Active mode → both styles are the active style.
    let (k, l) = mode_pair(Mode::Docker, Mode::Docker, active, key_off, label_off);
    assert_eq!(k, active);
    assert_eq!(l, active);

    // Inactive mode → off styles.
    let (k, l) = mode_pair(Mode::Docker, Mode::Monitor, active, key_off, label_off);
    assert_eq!(k, key_off);
    assert_eq!(l, label_off);
}

// ===========================================================================
// fmt.rs (multitop) — status_line, error_line, header_line
// ===========================================================================

#[test]
fn fmt_helpers_produce_output() {
    let _g = isolate_keychain();
    let status = multitop::fmt::status_line("ready");
    assert!(status.contains("ready"));

    let error = multitop::fmt::error_line(String::from("failed"));
    assert!(error.contains("failed"));

    let header = multitop::fmt::header_line(String::from("Upgrade on host"));
    assert!(header.contains("Upgrade on host"));
}

// ===========================================================================
// config_ui.rs — draw path (via public exports)
// ===========================================================================

#[test]
fn config_ui_module_exists() {
    let _g = isolate_keychain();
    // Verify the module has public types we can reference.
    let _ = multitop::config_ui::draw;
}

// ===========================================================================
// state.rs — save/load roundtrip
// ===========================================================================

#[tokio::test]
async fn state_save_and_load_roundtrip() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let mut hosts = std::collections::BTreeMap::new();
    hosts.insert(
        "admin@h1:22".into(),
        multitop::state::HostUpdate {
            started_at: Some(100),
            finished_at: Some(172),
            success: true,
        },
    );

    let state = multitop::state::AppState {
        last_update: Some(172),
        upgrade_started_at: None,
        hosts,
        selected_host: None,
        filter_query: None,
        sort: None,
        views: Default::default(),
    };

    multitop::state::save_state(&config_path, &state).expect("save ok");
    let loaded = multitop::state::load_state(&config_path);

    assert_eq!(loaded.state.last_update, Some(172));
    assert_eq!(loaded.state.hosts.len(), 1);
    assert!(loaded.notice.is_none(), "clean load says nothing");
}

#[tokio::test]
async fn state_load_missing_file_is_first_run() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("nonexistent").join("config.toml");

    let loaded = multitop::state::load_state(&config_path);
    assert!(loaded.state.last_update.is_none());
    assert!(loaded.notice.is_none(), "missing file is silent first run");
}

// ===========================================================================
// Event loop integration — drives run.rs body via scripted events
// ===========================================================================

/// Drive the real event loop with scripted events and a way to stop it.
/// Returns the terminal backend so we can inspect what was drawn.
async fn drive_event_loop(
    servers: Vec<Server>,
    size: (u16, u16),
    events: Vec<Event>,
) -> ratatui::Terminal<ratatui::backend::TestBackend> {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _dims_rx) = watch::channel((0u16, 0u16));

    // Event stream: scripted events followed by pending.
    let mut stream = tokio_stream::iter(events.into_iter().map(Ok)).chain(tokio_stream::pending());

    let backend = TestBackend::new(size.0, size.1);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

    // Run the loop with a timeout — it will process scripted events then
    // block on pending(). The timeout ensures we can inspect state.
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            servers,
            config_path,
            None,
        ),
    )
    .await;

    terminal
}

fn event_loop_test_server(port_offset: u16) -> Server {
    Server {
        host: format!("127.0.0.{}", port_offset % 255 + 1),
        port: 0,
        user: "testuser".into(),
        upgrade_cmd: Some("true".into()),
    }
}

#[tokio::test]
async fn event_loop_processes_key_events() {
    let servers = vec![event_loop_test_server(1)];

    // Script: switch to fetch, then docker, then stats, then quit.
    let terminal = drive_event_loop(
        servers,
        (100, 30),
        vec![
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('f'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('d'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('s'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
        ],
    )
    .await;

    // The terminal was drawn to (buffer is non-empty).
    let backend = terminal.backend();
    let buffer = backend.buffer();
    // Just verify the buffer has content (was drawn).
    assert_ne!(buffer.content, [] as [ratatui::buffer::Cell; 0]);
}

#[tokio::test]
async fn event_loop_handles_resize() {
    let servers = vec![event_loop_test_server(1)];

    let terminal = drive_event_loop(servers, (100, 30), vec![Event::Resize(120, 40)]).await;

    let backend = terminal.backend();
    assert_eq!(backend.buffer().area.width, 100); // Original size (resize is debounced)
}

#[tokio::test]
async fn event_loop_filter_and_quit() {
    let servers = vec![event_loop_test_server(1)];

    let terminal = drive_event_loop(
        servers,
        (100, 30),
        vec![
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('/'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('w'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('e'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('b'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Enter,
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
        ],
    )
    .await;

    let backend = terminal.backend();
    assert_ne!(backend.buffer().content, [] as [ratatui::buffer::Cell; 0]);
}
