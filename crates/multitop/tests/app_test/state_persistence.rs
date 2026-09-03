use super::*;

#[test]

fn quit_sets_the_flag() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    assert!(!a.should_quit());
    a.quit();
    assert!(a.should_quit());
}

#[test]
fn helpers_wrap_in_ansi() {
    let _keychain = isolate_keychain();
    assert!(error_line("boom").contains("boom"));
    assert!(status_line("wait").contains("wait"));
    assert!(header_line("hi").contains("hi"));
}

#[test]
fn local_server_deduplication() {
    use multitop::config::Server;
    use multitop::ssh::is_local;
    let _keychain = isolate_keychain();

    let s1 = Server {
        host: "127.0.0.1".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let s2 = Server {
        host: "localhost".into(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let s3 = Server {
        host: "192.168.0.33".into(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };

    let mut servers = vec![s1, s2, s3];
    let mut seen_local = false;
    servers.retain(|s| {
        if is_local(s) {
            if seen_local {
                false
            } else {
                seen_local = true;
                true
            }
        } else {
            true
        }
    });

    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].host, "127.0.0.1");
    assert_eq!(servers[1].host, "192.168.0.33");
}

#[test]
fn server_configuration_persists_without_password_fields() {
    let _keychain = isolate_keychain();
    let toml = r#"
    [[servers]]
    host = "192.168.0.33"
    upgrade_cmd = "us;ud"
    "#;

    let dir = std::env::temp_dir().join(format!("multitop_test_{}", std::process::id()));
    let file = dir.join("config.toml");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(&file, toml).unwrap();

    let servers = multitop::config::parse(toml).unwrap().servers;
    multitop::config::save_servers(&file, &servers).unwrap();
    let updated = std::fs::read_to_string(&file).unwrap();
    let cfg2 = multitop::config::parse(&updated).unwrap();
    assert_eq!(cfg2.servers[0].host, "192.168.0.33");
    assert!(!updated.contains("sudo_password"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Editing the server list rebuilds every panel. The rebuilt panels used to
/// come back at the compiled-in default scrollback, because the configured
/// `upgrade_history_lines` is applied once at startup and `Panel::new` knows
/// nothing about it -- so adding one host silently reset everyone's log depth.
#[test]
fn rebuilt_panels_keep_the_configured_upgrade_history() {
    let _k = isolate_keychain();
    let mut a = App::new(servers(1));
    a.upgrade_history_lines = 3;

    a.replace_panels(servers(2));

    for p in &mut a.panels {
        for i in 0..10 {
            p.last_upgrade.push(format!("line {i}"));
        }
        assert_eq!(
            p.last_upgrade.len(),
            3,
            "a rebuilt panel must inherit the configured history depth"
        );
    }
}

/// A state write that failed must be said out loud.
///
/// `save_state` goes to some trouble to be atomic precisely because
/// `upgrade_started_at` and each host's `started_at` are what make an
/// interrupted run detectable afterwards. A write that never happened defeats
/// that just as completely as a torn one -- and the error was discarded, so a
/// read-only or full disk cost the user their upgrade history with nothing on
/// screen to say so.
#[test]
fn a_state_write_that_failed_is_reported() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![Server {
        host: "host1".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("apt upgrade".to_string()),
        custom_command: None,
    }]);

    // A config path whose parent is a regular file: `save_state` cannot create
    // its temporary there. The shape of a read-only config directory, without
    // needing one.
    let blocker =
        std::env::temp_dir().join(format!("multitop_state_blocker_{}", std::process::id()));
    let _ = std::fs::remove_file(&blocker);
    std::fs::write(&blocker, b"not a directory").unwrap();
    app.config_path = Some(blocker.join("config.toml"));

    app.mark_upgrades_started(&[0]);

    // Through `pane_lines`, which is what the pane draws: a notice lives in
    // `notes` and is wrapped in at render time, not stored in `view`.
    let said = multitop::ui::pane_lines(&app, 0, 20, 60, 0)
        .0
        .iter()
        .any(|l| l.contains("could not save upgrade state"));

    let _ = std::fs::remove_file(&blocker);

    assert!(
        said,
        "a failed state write must reach the panel; the pane said: {:?}",
        multitop::ui::pane_lines(&app, 0, 20, 60, 0).0
    );
}

/// The failed-state-write notice must survive the path that actually runs.
///
/// `confirm_upgrade` marks each host started -- which is where the write
/// happens, and where the notice is pushed -- and *then* calls `run_upgrade`,
/// which clears each panel's `last_upgrade` ring before streaming into it. The
/// panels are already in Upgrade mode by then, so `Panel::note` had put the
/// notice in exactly the buffer the next line clears. `note` writes only to
/// `notes` now, which nothing on the upgrade path clears, so the ordering can no
/// longer bite -- but the ordering is still the thing under test.
///
/// Found reviewing this session's own work. Testing `mark_upgrades_started`
/// directly passed, because nothing clears the ring afterwards; the defect
/// lives in the order of the real path, which is the only thing worth pinning.
#[test]
fn the_failed_state_write_notice_survives_confirm_upgrade() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![Server {
        host: "host1".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("apt upgrade".to_string()),
        custom_command: None,
    }]);

    let blocker =
        std::env::temp_dir().join(format!("multitop_confirm_blocker_{}", std::process::id()));
    let _ = std::fs::remove_file(&blocker);
    std::fs::write(&blocker, b"not a directory").unwrap();
    app.config_path = Some(blocker.join("config.toml"));

    // The real sequence: `u` to enter the view, then confirm.
    app.enter_upgrade_view();
    let _ = app.confirm_upgrade();

    // Asked of the pane, not of a buffer. This used to read `last_upgrade`
    // directly, and it broke the day `Panel::note` stopped writing there --
    // on a change that made the notice *more* durable, not less. A test that
    // names the buffer passes or fails on the mechanism; `ui::pane_lines` is
    // the single entry point to what is actually in that pane.
    let (pane, _) = multitop::ui::pane_lines(&app, 0, 20, 60, 0);
    let said = pane
        .iter()
        .any(|l| l.contains("could not save upgrade state"));

    let _ = std::fs::remove_file(&blocker);

    assert!(
        said,
        "the notice must still be in the pane the operator is looking at; \
         the pane holds: {pane:?}"
    );
}

/// A notice must survive the next agent frame.
///
/// `Panel::note`'s own doc says it exists so a message is never "built, stored,
/// and never drawn" -- and in every non-Upgrade mode it wrote into `view`,
/// which is *derived state*: `show_last_frame` and the Monitor packet arm both
/// rebuild `view` from `last_frame`. The first frame arrives about a second
/// after startup, which is exactly when every startup notice is written: the
/// plaintext-password migration, the clamped `upgrade_history_lines`, an
/// unreadable `state.toml`. All of them appeared for one second and were gone.
#[test]
fn a_notice_survives_the_next_frame() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.panels[0].note("moved 2 plaintext passwords out of config.toml".to_string());
    let pane = |a: &App| multitop::ui::pane_lines(a, 0, 20, 60, 0).0.join("\n");
    assert!(
        pane(&a).contains("plaintext passwords"),
        "it must show immediately"
    );

    // The first monitor frame lands.
    a.apply(Msg::Frame {
        panel: 0,
        epoch: 0,
        lines: vec!["cpu 4%".into(), "mem 1.2G".into()],
    });

    let shown = pane(&a);
    assert!(
        shown.contains("cpu 4%"),
        "the frame must still be drawn: {shown}"
    );
    assert!(
        shown.contains("plaintext passwords"),
        "and the notice must still be there -- a message the next frame erases \
         is a message nobody reads: {shown}"
    );
}

/// The same, through a rendered packet rather than a status frame: both paths
/// rebuild `view`, and a notice that survives one and not the other is the
/// two-places-one-quantity shape this round keeps finding.
#[test]
fn a_notice_survives_a_rendered_monitor_packet() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.panels[0].note("upgrade_history_lines = 0 would leave nothing to show".to_string());

    a.apply(Msg::Packet {
        panel: 0,
        gen: a.panels[0].gen,
        epoch: 0,
        payload: multitop_agent::proto::Payload::Monitor(
            multitop_agent::render::Snapshot::default(),
        ),
        dims: (80, 24),
    });

    let shown = multitop::ui::pane_lines(&a, 0, 20, 60, 0).0.join("\n");
    assert!(
        shown.contains("would leave nothing to show"),
        "the pane says: {shown}"
    );
}
