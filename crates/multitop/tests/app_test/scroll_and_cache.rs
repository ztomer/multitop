use super::*;

/// The row on screen and the keys that act must name the same confirmation.
///
/// `ui::keybar_content` chose which confirmation to draw and `run::handle_key`
/// chose which keys were live, each from its own copy of the priority -- in
/// **opposite orders**. The keybar put the armed quit first; the key handler put
/// the upgrade modal first. With both set the screen read
/// `1 upgrade running · [Q] quit anyway · [Esc] stay` while `q` closed an
/// invisible upgrade modal and `u` would have started `apt upgrade` on every
/// visible host -- a confirmation acting on keys it never named, which is the
/// defect the twelfth pass removed by hand.
///
/// Asserted as the pair, not as either order: whatever the priority is, the two
/// must agree.
#[test]

fn the_confirmation_on_screen_is_the_one_whose_keys_are_live() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![Server {
        host: "host1".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("apt upgrade".to_string()),
        custom_command: None,
    }]);

    // An upgrade is running, so Esc arms a quit rather than taking it.
    app.panels[0].upgrade_state = multitop::panel::UpgradeState::STARTED;
    app.request_quit();
    assert!(
        app.quit_armed(),
        "a running upgrade must arm rather than quit"
    );

    // And now the upgrade modal is raised underneath it -- which `VaultUnlocked`
    // does on its own, from a message, with no key involved.
    app.set_show_upgrade_modal(true);
    assert!(app.quit_armed() && app.show_upgrade_modal(), "both are set");

    let theme = app.current_theme();
    let row: String = multitop::ui::keybar_content(&app, theme, 80, multitop::app::Mode::Monitor)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();

    // Whichever the row is, the live keys have to be that one's keys.
    if row.contains("quit anyway") {
        assert_eq!(
            app.active_confirm(),
            Some(multitop::app::Confirm::Quit),
            "the row names the quit confirmation, so its keys must be the live ones; row: {row}"
        );
    } else {
        assert_eq!(
            app.active_confirm(),
            Some(multitop::app::Confirm::Upgrade),
            "the row names the upgrade confirmation, so its keys must be the live ones; row: {row}"
        );
    }

    // And concretely: the screen offers `[Q] quit anyway`, so `q` must quit.
    assert!(
        row.contains("quit anyway"),
        "a quit armed over a running upgrade is the confirmation that must win; row: {row}"
    );
}

// ---------------------------------------------------------------------------
// Fix 2: switch_stats preserves gen + scroll for mid-upgrade panels
// ---------------------------------------------------------------------------

#[test]
fn switch_stats_preserves_gen_for_mid_upgrade_panels() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    // Put panel 0 into upgrade and mark it STARTED.
    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 42;
    let gen_before = a.panels[0].gen;

    a.switch_stats();

    // Mid-upgrade panel: gen preserved so AuxLine stamped 42 still belongs.
    assert_eq!(a.panels[0].gen, gen_before, "gen must not bump mid-upgrade");
    assert_eq!(a.panels[0].mode, Mode::Monitor);
    assert_eq!(a.panels[0].upgrade_state, UpgradeState::STARTED);
}

#[test]
fn switch_stats_resets_scroll_for_idle_panels() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.panels[0].scroll_offset = 25;
    a.switch_stats();

    assert_eq!(a.panels[0].scroll_offset, 0, "idle panel scroll resets");
}

/// Leaving the Upgrade view must not carry its offset into the view being
/// entered: an offset into a scrollback log means nothing in the Monitor pane,
/// which opened scrolled to a position the user never chose.
///
/// These two used to assert the shared-field mechanism — that `scroll_offset`
/// simply was not reset — and that is exactly what leaked.
#[test]
fn leaving_the_upgrade_view_does_not_scroll_the_view_it_leaves_for() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 5;
    a.panels[0].scroll_offset = 20;

    a.switch_stats();

    assert_eq!(
        a.panels[0].scroll_offset, 0,
        "the Monitor pane opened at the upgrade log's offset"
    );
    assert_eq!(
        a.panels[0].upgrade_gen, 5,
        "the in-flight generation must still survive the switch"
    );
}

/// The requirement behind the fix: the user's place in a log they scrolled is
/// still there when they come back to it.
#[test]
fn a_round_trip_returns_to_the_place_in_the_log() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.enter_upgrade_view();
    a.panels[0].scroll_offset = 15;

    a.switch_stats();
    assert_eq!(
        a.panels[0].scroll_offset, 0,
        "the stats pane is not scrolled"
    );

    a.enter_upgrade_view();
    assert_eq!(
        a.panels[0].scroll_offset, 15,
        "coming back to the log lost the place the user had in it"
    );
}

/// And the same round trip through the other two views.
#[test]
fn the_place_in_the_log_survives_the_docker_and_fetch_views_too() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.enter_upgrade_view();
    a.panels[0].scroll_offset = 9;

    a.toggle_docker((80, 24));
    assert_eq!(a.panels[0].scroll_offset, 0);
    a.toggle_fetch((80, 24));
    assert_eq!(a.panels[0].scroll_offset, 0);

    a.enter_upgrade_view();
    assert_eq!(a.panels[0].scroll_offset, 9);
}

/// Pressing the view's own key again is documented as doing nothing. It used
/// to throw every pane's scroll position away first.
#[test]
fn asking_for_the_view_already_showing_keeps_the_scroll_position() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.toggle_docker((80, 24));
    a.panels[0].scroll_offset = 12;
    assert!(
        a.toggle_docker((80, 24)).is_empty(),
        "a no-op spawns nothing"
    );
    assert_eq!(
        a.panels[0].scroll_offset, 12,
        "a no-op discarded the scroll"
    );

    a.toggle_fetch((80, 24));
    a.panels[0].scroll_offset = 7;
    assert_eq!(a.toggle_fetch((80, 24)), []);
    assert_eq!(a.panels[0].scroll_offset, 7);
}

// ---------------------------------------------------------------------------
// Fix 3: cached payload on view entry
// ---------------------------------------------------------------------------

#[test]
fn toggle_docker_shows_cached_payload_immediately() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    // Simulate a previously-fetched docker payload.
    let payload = multitop_agent::proto::Payload::Docker {
        host: "test-host".into(),
        rows: vec![multitop_agent::docker::Row {
            name: "web".into(),
            status: "Up 2 hours".into(),
            image: "nginx:latest".into(),
            cpu: "0.5%".into(),
            cpu_pct: 0.5,
            mem: "64M".into(),
            mem_bytes: 67_108_864,
        }],
    };
    a.panels[0].last_docker = Some(payload);

    let cmds = a.toggle_docker((80, 24));

    // Commands spawned to refresh, and view shows cached data.
    assert!(!cmds.is_empty(), "refresh task spawned");
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("web")),
        "cached container name visible"
    );
}

#[test]
fn toggle_fetch_shows_cached_payload_immediately() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.panels[0].last_fetch = Some(FetchSnapshot {
        user_host: "test-host".into(),
        ..FetchSnapshot::default()
    });

    let cmds = a.toggle_fetch((80, 24));

    assert!(!cmds.is_empty(), "refresh task spawned");
    // Cached fetch rendered into view (non-empty, has content beyond the loading row).
    assert!(
        a.panels[0].view.len() > 1,
        "cached fetch rendered multiple lines"
    );
    // The host name is rendered as fullwidth glyphs by center_header; what matters
    // is that the cached snapshot produced real output (not a loading placeholder).
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("OS")),
        "cached fetch rendered detail rows, not a loading placeholder"
    );
}

// ---------------------------------------------------------------------------
// Fix 5: task-death watchdog via mark_upgrade_interrupted
// ---------------------------------------------------------------------------

#[test]
fn mark_upgrade_interrupted_flips_started_to_done() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].server = Server {
        host: "test.example".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: Some("sudo apt upgrade".into()),
        custom_command: None,
    };

    a.mark_upgrade_interrupted(0);

    assert_eq!(a.panels[0].upgrade_state, UpgradeState::DONE);
}

#[test]
fn mark_upgrade_interrupted_persists_finished_at() {
    let _guard = isolate_keychain();
    let mut a = app(2);
    a.config_path = Some(std::env::temp_dir().join("multitop_test_state.toml"));

    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].server = Server {
        host: "persist.example".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: Some("sudo apt upgrade".into()),
        custom_command: None,
    };

    a.mark_upgrade_interrupted(0);

    let key = multitop::password_store::account(&a.panels[0].server);
    let entry = a.host_updates.get(&key).expect("state persisted");
    assert!(entry.finished_at.is_some(), "finished_at recorded");
    assert!(!entry.success, "interrupted = not success");
}

#[test]
fn mark_upgrade_interrupted_noop_for_done_panel() {
    let _guard = isolate_keychain();
    let mut a = app(2);

    a.panels[0].upgrade_state = UpgradeState::DONE;
    a.mark_upgrade_interrupted(0);

    assert_eq!(a.panels[0].upgrade_state, UpgradeState::DONE);
    assert!(a.host_updates.is_empty(), "no state written for DONE panel");
}
