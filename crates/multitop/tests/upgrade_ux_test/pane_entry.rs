use super::*;

#[tokio::test]
async fn first_press_switches_to_the_upgrade_pane() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);

    h.press('u');

    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);
    assert!(
        !h.app.show_upgrade_modal(),
        "the first press must not open the confirm modal"
    );
    assert!(
        !h.app.upgrades_in_flight(),
        "the first press must not start an upgrade"
    );
    assert!(
        h.emitted().is_empty(),
        "the first press must not queue any work"
    );
}

/// The regression that motivated this work: the first press used to behave
/// differently depending on whether an upgrade had run before.
#[tokio::test]
async fn first_press_is_the_same_before_and_after_an_upgrade_has_run() {
    let _keychain = isolate_keychain_async().await;
    let mut fresh = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    fresh.press('u');
    let fresh_mode = fresh.app.panels[0].mode;
    let fresh_modal = fresh.app.show_upgrade_modal();

    let mut used = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    used.app.panels[0].upgrade_state = multitop::panel::UpgradeState::DONE;
    used.app.panels[0].last_upgrade = vec!["previous output".to_string()].into();
    used.press('u');

    assert_eq!(
        fresh_mode, used.app.panels[0].mode,
        "first press must land in the same view either way"
    );
    assert_eq!(
        fresh_modal,
        used.app.show_upgrade_modal(),
        "first press must never open the modal, upgraded before or not"
    );
}

#[tokio::test]
async fn first_press_reaches_the_pane_from_every_other_view() {
    let _keychain = isolate_keychain_async().await;
    for entry in ['d', 'f', 's'] {
        let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
        h.press(entry);
        h.press('u');
        assert_eq!(
            h.app.panels[0].mode,
            Mode::Upgrade,
            "u from '{entry}' must reach the Upgrade pane"
        );
        assert!(!h.app.show_upgrade_modal(), "from '{entry}'");
    }
}

// ---------------------------------------------------------------------------
// 2. The pane tells the user what they need to decide.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pane_shows_the_command_and_history_for_each_host() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt update && apt upgrade -y")),
        server("db-02", Some("dnf upgrade -y")),
    ]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    h.app.host_updates.insert(
        multitop::password_store::account(&h.servers[0]),
        HostUpdate {
            started_at: Some(now - 86_400 * 3 - 30),
            finished_at: Some(now - 86_400 * 3),
            success: true,
        },
    );

    h.press('u');

    // The host name itself comes from the panel banner that `ui::draw` writes
    // over view[0], not from the pane body — see `upgrade_view::header`. What
    // the body must get right is the per-host detail.
    let web = h.pane_text(0);
    assert!(web.contains("apt update && apt upgrade -y"), "{web}");
    assert!(web.contains("3 days ago"), "{web}");
    assert!(web.contains("ok"), "{web}");

    // Each pane shows its OWN host, not a shared summary.
    let db = h.pane_text(1);
    assert!(db.contains("dnf upgrade -y"), "{db}");
    assert!(db.contains("never"), "db-02 has no history: {db}");
    assert!(
        !db.contains("apt update"),
        "panes must not leak each other's commands: {db}"
    );
}

#[tokio::test]
async fn pane_explains_a_host_with_no_upgrade_cmd() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", None)]);
    h.press('u');

    let text = h.pane_text(0);
    assert!(text.contains("not configured"), "{text}");
    assert!(text.contains("host is skipped"), "{text}");
    assert!(
        text.contains("set upgrade_cmd in config.toml"),
        "must show how to fix it: {text}"
    );
}

#[tokio::test]
async fn pane_warns_about_an_interrupted_previous_run() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    h.app.host_updates.insert(
        multitop::password_store::account(&h.servers[0]),
        HostUpdate {
            started_at: Some(now - 3600),
            finished_at: None,
            success: false,
        },
    );

    h.press('u');

    let text = h.pane_text(0);
    assert!(text.contains("interrupted"), "{text}");
    assert!(text.contains("never finished"), "{text}");
}

// ---------------------------------------------------------------------------
// 3. Second press starts the run.
// ---------------------------------------------------------------------------
