use super::*;

#[tokio::test]
async fn a_history_limit_below_the_floor_is_raised_and_said_out_loud() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    // One line would leave the Upgrade pane swallowing everything, including
    // the warnings that name a lock file to remove. Obeying it silently is the
    // failure; raising it silently is only half a fix.
    std::fs::write(
        dir.path().join("config.toml"),
        "upgrade_history_lines = 1\n\n[[servers]]\nhost = \"alpha.example\"\nport = 22\nuser = \"admin\"\n",
    )
    .unwrap();

    let (drawn, _) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![],
    )
    .await;
    assert!(
        drawn.contains("upgrade_history_lines"),
        "the raised limit was applied without telling anyone:\n{drawn}"
    );
}

#[tokio::test]
async fn a_state_file_that_cannot_be_parsed_says_so_and_is_kept() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.toml");
    std::fs::write(&state, "this is not = = toml").unwrap();

    let (drawn, _) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![],
    )
    .await;
    assert!(
        drawn.contains("could not be parsed"),
        "an unreadable state file must not look like a first run:\n{drawn}"
    );
    // The evidence is preserved rather than overwritten by the next save.
    assert!(state.with_extension("toml.unreadable").exists());
}

#[tokio::test]
async fn a_plaintext_password_in_the_config_is_moved_out_of_it() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "[[servers]]\nhost = \"alpha.example\"\nport = 22\nuser = \"admin\"\nsudo_password = \"hunter2\"\n",
    )
    .unwrap();

    // Porting a password earns a vault the same way an interactive save does,
    // so the offer is up and has to be answered before the app will quit.
    let (drawn, _) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![key(KeyCode::Esc)],
    )
    .await;

    let after = std::fs::read_to_string(&config).unwrap();
    assert!(
        !after.contains("hunter2"),
        "a plaintext password was left on disk:\n{after}"
    );
    assert!(
        drawn.contains("plaintext password"),
        "the move happened silently:\n{drawn}"
    );
}

#[tokio::test]
async fn a_named_theme_is_selected_at_startup() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let name = multitop_agent::color::THEMES[1].name;

    // Asking for a theme by name must select it; asking for one that does not
    // exist must leave the default alone rather than failing to start.
    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        Some(name.to_ascii_uppercase()),
        vec![],
    )
    .await;
    assert!(outcome.error.is_none());

    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        Some("no-such-theme".into()),
        vec![],
    )
    .await;
    assert!(
        outcome.error.is_none(),
        "an unknown theme must not stop startup"
    );
}

// ---------------------------------------------------------------------- mouse

#[tokio::test]
async fn a_click_selects_the_pane_it_landed_on() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
        test_server("delta.example"),
    ];
    let size = (120u16, 40u16);
    let shown: Vec<usize> = (0..servers.len()).collect();
    let (areas, _) = multitop::ui::regions(Rect::new(0, 0, size.0, size.1), servers.len());
    // A point inside the last pane, which is not the one selected at startup.
    let last = areas[areas.len() - 1];
    let (x, y) = (last.x + 1, last.y + 1);
    assert_eq!(
        panel_at_pos(x, y, Rect::new(0, 0, size.0, size.1), &shown),
        Some(3)
    );

    let (drawn, outcome) = run_to_quit(
        &dir,
        servers,
        size,
        None,
        vec![
            mouse(MouseEventKind::Down(MouseButton::Left), x, y),
            // Scrolls on the same pane; the loop must act on them rather than
            // discarding them with the motion floods.
            mouse(MouseEventKind::ScrollUp, x, y),
            mouse(MouseEventKind::ScrollDown, x, y),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
    assert!(
        drawn.contains("delta"),
        "the clicked pane should be drawn:\n{drawn}"
    );
}

#[tokio::test]
async fn mouse_traffic_that_lands_on_nothing_is_ignored() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    // Motion floods in whenever any-event tracking is on; the keybar row and
    // any point past the screen match no pane. None of it may stop the loop.
    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![
            mouse(MouseEventKind::Moved, 5, 5),
            mouse(MouseEventKind::Up(MouseButton::Left), 5, 5),
            mouse(MouseEventKind::Down(MouseButton::Right), 5, 5),
            mouse(MouseEventKind::Down(MouseButton::Left), 5000, 5000),
            mouse(MouseEventKind::ScrollUp, 5000, 5000),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
}

#[test]
fn a_click_with_no_panes_on_screen_selects_nothing() {
    // Answering "panel 0" here moved the selection to the first host whenever
    // the user clicked the keys row, which is the row that invites clicking.
    assert_eq!(panel_at_pos(1, 1, Rect::new(0, 0, 80, 24), &[]), None);
    // A filtered list answers with the index into the *unfiltered* list, which
    // is what the panes on screen actually are.
    assert_eq!(panel_at_pos(1, 1, Rect::new(0, 0, 80, 24), &[7]), Some(7));
}

// ------------------------------------------------------------ ignored events

#[tokio::test]
async fn events_the_loop_has_no_use_for_are_stepped_over() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![
            Event::FocusGained,
            Event::FocusLost,
            Event::Paste("pasted".into()),
            Event::Resize(100, 30),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
}

#[tokio::test]
async fn a_terminal_that_goes_away_leaves_through_the_normal_exit() {
    // Returning early would strand the SSH children, so a dead event source
    // has to quit the same way `q` does.
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    let mut stream = tokio_stream::iter(vec![Err(std::io::Error::other("terminal went away"))]);
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("a dead event source must end the loop");

    // A read error is not a terminal failure: the loop quit cleanly, so there
    // is nothing to report but the upgrades it killed.
    assert!(outcome.error.is_none());
    assert_eq!(outcome.killed, [] as [std::string::String; 0]);
}

#[tokio::test]
async fn an_exhausted_event_source_ends_the_loop() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    let mut stream = tokio_stream::iter(Vec::<std::io::Result<Event>>::new());
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("an exhausted event source must end the loop");
    assert!(outcome.error.is_none());
}

// -------------------------------------------------------------------- signals
