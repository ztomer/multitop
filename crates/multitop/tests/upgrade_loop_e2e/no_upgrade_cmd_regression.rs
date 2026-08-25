use super::*;

// ============================================================================
// Bug a regressions: "No output on <host> on updater"
// A server without `upgrade_cmd` was silently skipped: the panel got a single
// dim line, `upgrade_state` stayed NIL, `last_upgrade` stayed empty, and the
// confirm modal never disclosed the skip. Now the skip is an explicit
// terminal state that persists and is surfaced before running.
// ============================================================================

/// A server with `upgrade_cmd: None` (as created by `-r <host>` or a config
/// with `upgrade_cmd` commented out).
fn no_upgrade_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
    }
}

/// Bug a: skipped server must reach a terminal DONE state with a visible,
/// host-specific message — not a dead NIL state that reads as "no output".
#[test]
fn test_upgrade_skip_server_reaches_terminal_state() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![no_upgrade_server("192.168.0.90")]);

    let cmds = app.run_upgrade();

    assert!(
        cmds.is_empty(),
        "no RunUpgrade command for a skipped server"
    );
    let p = &app.panels[0];
    assert_eq!(p.upgrade_state, UpgradeState::DONE, "skip must be terminal");
    assert!(
        p.upgrade_gen > 0,
        "skip must record the generation it was decided at"
    );
    assert!(!p.last_upgrade.is_empty(), "skip message must be persisted");
    let view = multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
        .0
        .join("\n");
    assert!(view.contains("192.168.0.90"), "message must name the host");
    assert!(view.contains("skipped"), "message must say it was skipped");
    assert!(
        view.contains("upgrade_cmd"),
        "message must point at the missing upgrade_cmd"
    );
}

/// Bug a: with a mix of configured and skipped servers, only the configured
/// one gets a `RunUpgrade` command and the skipped one reaches DONE.
#[test]
fn test_upgrade_mixed_servers_only_configured_run() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![
        local_server("apt update && apt upgrade -y"),
        no_upgrade_server("192.168.0.90"),
    ]);

    let cmds = app.run_upgrade();
    assert_eq!(cmds.len(), 1, "only the configured server runs");
    let Command::RunUpgrade { panel, .. } = cmds[0] else {
        panic!("Expected RunUpgrade for configured server");
    };
    assert_eq!(panel, 0);
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(app.panels[1].upgrade_state, UpgradeState::DONE);
    assert!(
        multitop::ui::pane_lines(&app, 1, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("192.168.0.90"),
        "skipped panel must still show why it was skipped"
    );
}

/// Bug a: the skip message must survive a switch to monitor and back — the
/// user must never land on a blank panel after the updater.
#[test]
fn test_upgrade_skip_message_persists_across_views() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![no_upgrade_server("192.168.0.90")]);
    app.run_upgrade();
    let msg: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();

    app.switch_stats();
    assert_eq!(app.panels[0].mode, Mode::Monitor);

    app.enter_upgrade_view();
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    // The Upgrade pane now always opens with a status header, so the previous
    // output follows it rather than being the whole view. The message itself
    // must still survive intact.
    let header = app.upgrade_pane_header(0);
    assert!(
        !header.is_empty(),
        "expected a status header above the previous output"
    );
    let ring: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert_eq!(ring, msg, "skip message must persist across view switches");
}

/// Bug a: the confirm modal data helper must list exactly the hosts that will
/// be skipped, so the user learns about them before running.
#[test]
fn test_upgrade_skip_hosts_helper_lists_unconfigured() {
    let _keychain = isolate_keychain();
    let app = App::new(vec![
        local_server("apt update"),
        no_upgrade_server("192.168.0.90"),
        no_upgrade_server("192.168.0.158"),
    ]);

    assert_eq!(
        app.upgrade_skip_hosts(),
        vec!["192.168.0.90".to_string(), "192.168.0.158".to_string()]
    );
}

/// Bug a: skip + completed upgrade round-trip. After a run where a server was
/// skipped, pressing `u` again (after switching to stats) shows the persisted
/// skip message instead of re-showing the confirm modal or a blank panel.
#[test]
fn test_upgrade_skip_then_u_shows_message_not_modal() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![no_upgrade_server("192.168.0.90")]);
    app.run_upgrade();
    app.switch_stats();
    assert!(!app.in_upgrade());

    assert!(
        !app.upgrades_in_flight(),
        "no upgrade in flight for a skipped server"
    );
    press(&mut app, crossterm::event::KeyCode::Char('u'));

    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("skipped"),
        "must show the skip message, not the modal"
    );
    assert!(!app.show_upgrade_modal());
}
