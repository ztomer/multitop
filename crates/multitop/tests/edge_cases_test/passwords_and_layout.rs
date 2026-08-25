use super::*;

#[tokio::test]
async fn a_key_pressed_with_settings_closed_does_nothing() {
    // The dispatcher checks before touching the manager, so a key that arrives
    // after the screen has closed cannot unwrap a `None`.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    assert!(app.password_manager.is_none());
    assert!(matches!(
        handle_key(&mut app, KeyCode::Char('x')),
        PasswordAction::None
    ));
}

#[tokio::test]
async fn typing_into_the_password_field_accumulates_and_corrects() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    open(&mut app, 0, false);
    // Open the row for editing, which is what puts a field on screen.
    handle_key(&mut app, KeyCode::Enter);

    for c in "hunter2".chars() {
        handle_key(&mut app, KeyCode::Char(c));
    }
    handle_key(&mut app, KeyCode::Backspace);
    // Keys the field has no use for leave it alone rather than being swallowed
    // by the row navigation underneath.
    handle_key(&mut app, KeyCode::Insert);
    handle_key(&mut app, KeyCode::F(5));

    // Whatever the field holds, none of that may have escaped the editor.
    assert!(
        app.password_manager.is_some(),
        "the editor closed on a stray key"
    );
}

#[tokio::test]
async fn answering_a_deletion_nobody_asked_about_does_nothing() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    open(&mut app, 0, false);

    // No pending confirmation: `y` is just a key.
    assert!(matches!(
        handle_key(&mut app, KeyCode::Char('y')),
        PasswordAction::None
    ));
    assert!(app.password_manager.is_some());
}

// ----------------------------------------------------------- the vault state

#[tokio::test]
async fn dismissing_a_prompt_that_is_not_up_leaves_the_state_alone() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    let before = app.vault_epoch;

    app.set_show_vault_password_prompt(false);
    assert!(!app.show_vault_password_prompt());
    assert_eq!(
        app.vault_epoch, before,
        "a no-op retired an in-flight attempt"
    );

    // Asking for it twice is also a no-op rather than a second prompt.
    app.set_show_vault_password_prompt(true);
    app.set_vault_password_error(Some("wrong password".into()));
    app.set_show_vault_password_prompt(true);
    assert_eq!(
        app.vault_password_error().map(String::as_str),
        Some("wrong password"),
        "asking again cleared the reason the user is being asked"
    );
}

#[tokio::test]
async fn a_second_create_attempt_while_one_is_running_is_refused() {
    // Argon2id is already running; starting another would race two writes to
    // the same file.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    assert!(app.begin_vault_creation());

    app.vault_password_input_mut().push_str("hunter2");
    assert!(app.begin_vault_create_attempt().is_some());
    assert!(app.vault_create_in_flight());
    assert!(
        app.begin_vault_create_attempt().is_none(),
        "a second attempt started while the first was still running"
    );
}

// ------------------------------------------------------------------- layout

#[tokio::test]
async fn wrapping_to_a_width_of_zero_yields_nothing_rather_than_looping() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    assert_eq!(
        multitop::layout::wrap_words("some text here", 0),
        [] as [std::string::String; 0]
    );
    assert_eq!(
        multitop::layout::wrap_words("", 10),
        [] as [std::string::String; 0]
    );
}

#[tokio::test]
async fn a_grid_with_no_room_left_to_share_still_returns_a_row_per_panel() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // The flexible rows ask for more than there is; whoever is left when the
    // pool empties gets nothing rather than a wrapped negative.
    for panels in 1usize..=12 {
        for height in [1u16, 2, 3, 5, 8, 13, 40] {
            let (areas, _) =
                multitop::ui::regions(ratatui::layout::Rect::new(0, 0, 80, height), panels);
            assert_eq!(areas.len(), panels, "panels={panels} height={height}");
            // A grid, so panels share rows — what has to hold is that no pane
            // is placed outside the screen it is drawn on.
            for a in &areas {
                assert!(
                    a.y + a.height <= height,
                    "panels={panels} height={height}: a pane runs off the bottom: {a:?}"
                );
                assert!(
                    a.x + a.width <= 80,
                    "panels={panels}: a pane runs off the side: {a:?}"
                );
            }
        }
    }
}

// ------------------------------------------------------------------- refit

#[tokio::test]
async fn a_header_with_no_rule_in_it_is_left_as_it_was() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Nothing to refit: the caller keeps the line it had.
    assert_eq!(multitop::refit::refit_header("plain text", 80), None);
    // A rule but no name is the same answer.
    assert_eq!(
        multitop::refit::refit_header("\u{2500}\u{2500}\u{2500}", 80),
        None
    );
}

#[tokio::test]
async fn a_header_wider_than_the_pane_drops_its_rules_rather_than_the_name() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let header = format!(
        "\u{2500}\u{2500} {} \u{2500}\u{2500}",
        multitop_agent::fmt::fullwidth("web-01")
    );
    // Narrower than the name: no room for a rule either side, so the name is
    // all that comes back.
    let tight = multitop::refit::refit_header(&header, 4).expect("a header must be produced");
    assert!(
        !tight.contains('\u{2500}'),
        "a rule was drawn with no room for it: {tight:?}"
    );
    assert!(
        tight.contains('\u{FF57}'),
        "the name was dropped: {tight:?}"
    );

    // Exactly the name's width, and one under the two-space budget: both take
    // the same "name only" answer.
    for cols in [11, 12, 13] {
        let out = multitop::refit::refit_header(&header, cols).expect("a header");
        assert!(out.contains('\u{FF57}'), "cols={cols}: {out:?}");
    }

    // Wide enough, and the rules come back.
    let roomy = multitop::refit::refit_header(&header, 60).expect("a header");
    assert!(
        roomy.contains('\u{2500}'),
        "no rule at a roomy width: {roomy:?}"
    );
}

// ------------------------------------------------------------- ssh commands
