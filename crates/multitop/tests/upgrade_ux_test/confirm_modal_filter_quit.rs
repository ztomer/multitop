use super::*;

#[tokio::test]
async fn second_press_opens_the_confirm_modal() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    assert!(!h.app.show_upgrade_modal());

    h.press('u');
    assert!(
        h.app.show_upgrade_modal(),
        "the second press must ask for confirmation"
    );
    assert!(
        !h.app.upgrades_in_flight(),
        "still nothing running until the modal is confirmed"
    );
}

#[tokio::test]
async fn confirming_after_two_presses_starts_the_upgrade() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    h.press('u');
    h.press('u');

    assert!(!h.app.show_upgrade_modal(), "modal closes on confirm");
    assert!(
        h.app.upgrades_in_flight(),
        "confirming must actually start the upgrade"
    );
    assert!(
        h.app.upgrade_started_at.is_some(),
        "the start time is recorded so an interrupted run can be detected"
    );
}

#[tokio::test]
async fn second_press_with_nothing_configured_does_not_open_a_modal() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", None), server("db-02", None)]);
    h.press('u');
    h.press('u');

    assert!(
        !h.app.show_upgrade_modal(),
        "a modal whose only outcome is skipping every host is not worth showing"
    );
    let text = h.pane_text(0);
    assert!(text.contains("nothing to run"), "{text}");
}

#[tokio::test]
async fn presses_are_ignored_while_an_upgrade_is_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    h.press('u');
    h.press('u');
    assert!(h.app.upgrades_in_flight());

    h.press('u');
    assert!(
        !h.app.show_upgrade_modal(),
        "u must not re-arm an upgrade that is already running"
    );
}

// ---------------------------------------------------------------------------
// 3b. The filter scopes the run (class F). A filter that narrowed the grid to
//     one host used to still run `apt upgrade` on every host in config.toml,
//     while the hidden hosts' output and failures never rendered. What you see
//     is what you run.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_active_filter_scopes_the_upgrade_run() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt upgrade")),
        server("db-02", Some("apt upgrade")),
        server("cache-03", Some("apt upgrade")),
    ]);
    // Narrow the grid to db-02 and keep the filter.
    h.press('/');
    for c in "db-02".chars() {
        h.press(c);
    }
    h.press_key(KeyCode::Enter);
    assert_eq!(h.app.filtered_indices(), vec![1]);

    h.press('u');
    h.press('u');
    assert!(
        h.app.show_upgrade_modal(),
        "the second press must still ask for confirmation"
    );

    let cmds = h.app.confirm_upgrade();
    let panels: Vec<usize> = cmds
        .iter()
        .filter_map(|c| match c {
            multitop::types::Command::RunUpgrade { panel, .. } => Some(*panel),
            _ => None,
        })
        .collect();
    assert_eq!(
        panels,
        vec![1],
        "the run must be scoped to the filtered host, got {panels:?}"
    );
}

#[tokio::test]
async fn the_confirm_row_counts_only_the_filtered_scope() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt upgrade")),
        server("db-02", Some("apt upgrade")),
        server("cache-03", None),
    ]);
    h.press('/');
    for c in "web".chars() {
        h.press(c);
    }
    h.press_key(KeyCode::Enter);
    h.press('u');
    h.press('u');

    // The scoped set is web-01 only: one host, nothing to skip.
    let cmds = h.app.confirm_upgrade();
    let panels: Vec<usize> = cmds
        .iter()
        .filter_map(|c| match c {
            multitop::types::Command::RunUpgrade { panel, .. } => Some(*panel),
            _ => None,
        })
        .collect();
    assert_eq!(panels, vec![0], "only the visible host runs: {panels:?}");
}

#[tokio::test]
async fn a_filter_matching_only_unconfigured_hosts_has_nothing_to_run() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt upgrade")),
        server("db-02", None),
    ]);
    h.press('/');
    for c in "db".chars() {
        h.press(c);
    }
    h.press_key(KeyCode::Enter);

    h.press('u');
    h.press('u');

    assert!(
        !h.app.show_upgrade_modal(),
        "a filter showing only unconfigured hosts has nothing to confirm"
    );
    assert!(
        !h.app.upgrades_in_flight(),
        "and must not have started anything"
    );
    let text = h.pane_text(1);
    assert!(text.contains("nothing to run"), "{text}");
}

// ---------------------------------------------------------------------------
// 3c. Quitting while upgrades are in flight (class F). `Esc` is the key an
//     operator presses to back out of a screen, and it used to kill a live
//     `apt upgrade` on every host with no question asked. The first press
//     now arms a confirmation that names the hosts; `q` confirms, `Esc`
//     stands down.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn first_quit_press_arms_confirmation_when_upgrades_are_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    assert!(h.app.upgrades_in_flight());

    h.press('q');
    assert!(
        !h.app.should_quit(),
        "the first press must not kill a running upgrade"
    );
    assert!(h.app.quit_armed(), "it must arm the confirmation instead");
    assert_eq!(
        h.app.running_upgrade_hosts(),
        vec!["web-01"],
        "the confirm row must be able to name the host it would kill"
    );
}

#[tokio::test]
async fn second_quit_press_quits_and_esc_stands_down() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);

    h.press('q');
    assert!(h.app.quit_armed());

    h.press_key(KeyCode::Esc);
    h.press('q');
    assert!(
        h.app.quit_armed(),
        "Esc stands the armed quit down, so the next q must arm again"
    );
    assert!(!h.app.should_quit());

    h.press('q');
    h.press('q');
    assert!(h.app.should_quit(), "q while armed confirms the quit");
}

#[tokio::test]
async fn quit_is_immediate_when_nothing_is_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('q');
    assert!(
        h.app.should_quit(),
        "with nothing in flight, q must still quit in one press"
    );
    assert!(
        !h.app.quit_armed(),
        "and must not have armed a confirmation"
    );
}

#[tokio::test]
async fn ctrl_c_arms_the_same_confirmation_while_upgrades_are_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);

    let ctrl_c = KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    };
    handle_key(
        ctrl_c,
        &mut h.app,
        (80, 24),
        Arc::clone(&h.dims_rx),
        &h.tx,
        &mut h.tasks,
    );

    assert!(
        !h.app.should_quit(),
        "Ctrl-C must not kill a running upgrade either"
    );
    assert!(h.app.quit_armed());
}

// ---------------------------------------------------------------------------
// 4. The flow is reversible and repeatable.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn s_leaves_the_pane_and_u_returns_to_it() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);

    h.press('s');
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);

    // Back in, and still only arming on the second press.
    h.press('u');
    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);
    assert!(
        !h.app.show_upgrade_modal(),
        "re-entering the pane must not skip straight to the modal"
    );
}

// ---------------------------------------------------------------------------
// 5. Switching views mid-run. Reported from live use: after switching to stats
//    during an upgrade on one host, `u` would not come back, and the host that
//    finished while away lost its completion marker.
// ---------------------------------------------------------------------------
