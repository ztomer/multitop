#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::app::*;
use multitop::config::Server;
use multitop::fmt::{error_line, header_line, status_line};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// Driving an `App` reaches `password_store` several calls down, and an
/// integration binary is compiled without `cfg(test)`, so the mock is not in
/// force unless it is asked for. Without this these tests query the real OS
/// keychain: every rebuild changes the binary's code signature, so macOS raises
/// an access dialog and the suite stops until a human dismisses it -- and a test
/// can read, overwrite or delete credentials the user depends on.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// `isolate_keychain` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn servers(n: usize) -> Vec<Server> {
    (0..n)
        .map(|i| Server {
            host: format!("s{i}"),
            port: 22,
            user: String::new(),
            upgrade_cmd: None,
        })
        .collect()
}

fn app(n: usize) -> App {
    App::new(servers(n))
}

fn text(p: &Panel) -> String {
    p.view.join("\n")
}

#[test]
fn starts_in_monitor_mode_showing_connecting() {
    let _keychain = isolate_keychain();
    let a = app(2);
    assert_eq!(a.panels.len(), 2);
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Monitor);
        assert!(text(p).contains("connecting..."));
    }
}

#[test]
fn empty_server_list_is_allowed() {
    let _keychain = isolate_keychain();
    let mut a = app(0);
    assert!(a.panels.is_empty());
    assert!(a.toggle_docker().is_empty());
    assert!(a.switch_stats().is_empty());
}

#[test]
fn frame_is_shown_in_monitor_mode() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.apply(Msg::Frame {
        panel: 0,
        epoch: 0,
        lines: vec!["line1".into(), "line2".into()],
    });
    assert_eq!(text(&a.panels[0]), "line1\nline2");
}

#[test]
fn frame_is_stored_but_hidden_in_docker_mode() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.toggle_docker();
    a.apply(Msg::Frame {
        panel: 0,
        epoch: 0,
        lines: vec!["fresh".into()],
    });
    assert!(
        !text(&a.panels[0]).contains("fresh"),
        "docker view must not be overwritten"
    );
    assert_eq!(
        a.panels[0].last_frame.as_deref(),
        Some(&["fresh".to_string()][..])
    );
}

#[test]
fn frame_for_unknown_panel_is_ignored() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.apply(Msg::Frame {
        panel: 9,
        epoch: 0,
        lines: vec!["x".into()],
    });
    assert!(text(&a.panels[0]).contains("connecting"));
}

#[test]
fn toggle_docker_switches_every_panel_at_once() {
    let _keychain = isolate_keychain();
    let mut a = app(3);
    let cmds = a.toggle_docker();
    assert_eq!(cmds.len(), 3);
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Docker);
        assert!(text(p).contains("Docker loading"));
    }
}

#[test]
fn toggle_docker_twice_stays_in_docker_mode() {
    let _keychain = isolate_keychain();
    let mut a = app(3);
    a.toggle_docker();
    let cmds = a.toggle_docker();
    assert!(cmds.is_empty());
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Docker);
    }
}

#[test]
fn switch_stats_restores_the_last_frame() {
    let _keychain = isolate_keychain();
    let mut a = app(3);
    for i in 0..3 {
        a.apply(Msg::Frame {
            panel: i,
            epoch: 0,
            lines: vec![format!("data{i}")],
        });
    }
    a.toggle_docker();
    a.switch_stats();
    for (i, p) in a.panels.iter().enumerate() {
        assert_eq!(text(p), format!("data{i}"), "panel {i}");
    }
}

#[test]
fn switch_stats_without_data_says_waiting() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.toggle_docker();
    a.switch_stats();
    assert!(text(&a.panels[0]).contains("waiting for data"));
}

#[test]
fn switch_stats_from_docker() {
    let _keychain = isolate_keychain();
    let mut a = app(3);
    a.toggle_docker();
    a.switch_stats();
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Monitor);
    }
}

#[test]
fn every_transition_bumps_the_generation() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let g0 = a.panels[0].gen;
    a.toggle_docker();
    let g1 = a.panels[0].gen;
    a.switch_stats();
    let g2 = a.panels[0].gen;
    assert!(g1 > g0 && g2 > g1);
}

#[test]
fn stale_results_are_dropped() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let cmds = a.toggle_docker();
    let Command::RunDocker { gen, .. } = cmds[0] else {
        panic!()
    };
    a.switch_stats();

    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: None,
    });
    a.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "late docker output".into(),
    });
    assert!(!text(&a.panels[0]).contains("late docker output"));
}

#[test]
fn current_results_are_shown() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let cmds = a.toggle_docker();
    let Command::RunDocker { gen, .. } = cmds[0] else {
        panic!()
    };
    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: None,
    });
    a.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "container list".into(),
    });
    assert_eq!(text(&a.panels[0]), "container list");
}

#[test]
fn aux_output_streams_line_by_line() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let gen = a.panels[0].gen;
    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on s0".into()),
    });
    for i in 0..3 {
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: format!("step {i}"),
        });
    }
    let t = text(&a.panels[0]);
    assert!(t.starts_with("Upgrade on s0"));
    assert!(t.contains("step 0") && t.contains("step 2"));
}

#[test]
fn aux_output_is_capped() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let cap = a.upgrade_history_lines;
    let gen = a.panels[0].gen;
    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: None,
    });
    // Push well past the cap plus the whole amortisation band, so a trim must
    // have happened at least once.
    for i in 0..cap + 4 * multitop::app::LOG_AMORTIZE {
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: format!("l{i}"),
        });
    }
    let view = &a.panels[0].view;
    // This panel never started an upgrade (`upgrade_cmd` is None), so its lines
    // went to the visible pane only — the one remaining capped `Vec` path. The
    // trim is amortised: it never exceeds cap + LOG_AMORTIZE, and the tail (the
    // newest lines) is always intact.
    assert!(view.len() <= cap + multitop::app::LOG_AMORTIZE);
    let last_idx = cap + 4 * multitop::app::LOG_AMORTIZE - 1;
    assert_eq!(view.last().unwrap(), &format!("l{last_idx}"));
}

/// The upgrade log itself is a `RingLines`: exactly `cap` slots, overwriting
/// the oldest in place, with the newest line always surviving.
#[test]
fn upgrade_ring_is_exactly_capped() {
    let _keychain = isolate_keychain();
    let mut servers = servers(1);
    servers[0].upgrade_cmd = Some("apt upgrade -y".into());
    let mut a = App::new(servers);
    let cmds = a.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!("Expected RunUpgrade")
    };
    let cap = a.upgrade_history_lines;
    let over = cap + 1024;
    for i in 0..over {
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: format!("l{i}"),
        });
    }
    let ring = &a.panels[0].last_upgrade;
    assert_eq!(
        ring.len(),
        cap,
        "the ring must stay exactly at cap, not a bounded band above it"
    );
    let last_idx = over - 1;
    assert_eq!(
        ring.last().unwrap(),
        &format!("l{last_idx}"),
        "the newest line must always survive"
    );
    assert_eq!(
        ring.iter().next().unwrap(),
        &format!("l{}", over - cap),
        "the oldest surviving line must be the first one past the cap"
    );
}

#[test]
fn upgrade_without_command_explains_itself() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let cmds = a.run_upgrade();
    assert!(cmds.is_empty(), "nothing to run");
    assert!(multitop::ui::pane_lines(&a, 0, usize::MAX, 0, 0)
        .0
        .join("\n")
        .contains("No upgrade_cmd"));
}

#[test]
fn upgrade_command_large_output_and_carriage_return_cleanliness() {
    let _keychain = isolate_keychain();
    let raw_lines = vec![
        "ls -l /usr/bin".to_string(),
        "Downloading 10%\rDownloading 50%\rDownloading 100%".to_string(),
        "total 12345".to_string(),
        "drwxr-xr-x 1 root root \x1b]0;title\x074096 Jan 1\x08 00:00 bin".to_string(),
    ];
    let mut clean = Vec::new();
    for line in raw_lines {
        for part in line.split('\r') {
            let trimmed = part.trim_end_matches('\r');
            if !trimmed.is_empty() {
                clean.push(trimmed.to_string());
            }
        }
    }
    assert_eq!(clean[1], "Downloading 10%");
    assert_eq!(clean[2], "Downloading 50%");
    assert_eq!(clean[3], "Downloading 100%");
    let parsed = multitop::ansi::line_to_spans(&clean[4]);
    let plain: String = parsed.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(!plain.contains('\r') && !plain.contains('\x08') && !plain.contains("title"));
}

#[test]
fn upgrade_with_command_is_scheduled() {
    let _keychain = isolate_keychain();
    let mut servers = servers(2);
    servers[0].upgrade_cmd = Some("apt upgrade -y".into());
    let mut a = App::new(servers);
    let cmds = a.run_upgrade();
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::RunUpgrade { panel: 0, .. }));
    // The running state is shown by the pane's status header. It used to be a
    // line 0 "Upgrade running..." message, which `ui::draw` overwrote with the
    // host banner on every frame — the user never actually saw it.
    let running = multitop::ui::pane_lines(&a, 0, usize::MAX, 0, 0)
        .0
        .join("\n");
    assert!(running.contains("running"), "{running}");
    assert!(running.contains("do not quit"), "{running}");
    assert!(multitop::ui::pane_lines(&a, 1, usize::MAX, 0, 0)
        .0
        .join("\n")
        .contains("No upgrade_cmd"));
}

#[test]
fn upgrade_output_persists_until_dismissed() {
    let _keychain = isolate_keychain();
    let mut servers = servers(1);
    servers[0].upgrade_cmd = Some("apt upgrade -y".into());
    let mut a = App::new(servers);
    let cmds = a.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!()
    };

    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on s0".into()),
    });
    a.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "42 packages upgraded".into(),
    });
    a.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: None,
        success: true,
    });
    a.apply(Msg::Frame {
        panel: 0,
        epoch: 0,
        lines: vec!["cpu stats".into()],
    });

    assert!(multitop::ui::pane_lines(&a, 0, usize::MAX, 0, 0)
        .0
        .join("\n")
        .contains("42 packages upgraded"));
    a.switch_stats();
    assert_eq!(text(&a.panels[0]), "cpu stats");
}

#[test]
fn status_respects_generation() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let gen = a.panels[0].gen;
    a.apply(Msg::Status {
        panel: 0,
        gen,
        text: "installing agent".into(),
    });
    assert_eq!(text(&a.panels[0]), "installing agent");
    a.switch_stats();
    a.apply(Msg::Status {
        panel: 0,
        gen,
        text: "stale".into(),
    });
    assert!(!text(&a.panels[0]).contains("stale"));
}

#[test]
fn aux_done_can_append_a_note() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let gen = a.panels[0].gen;
    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: None,
    });
    a.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("exit 1".into()),
        success: true,
    });
    assert!(text(&a.panels[0]).contains("exit 1"));
}

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
    };
    let s2 = Server {
        host: "localhost".into(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
    };
    let s3 = Server {
        host: "192.168.0.33".into(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
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

#[test]
fn cleanup_old_agents_command_keeps_current_hashes() {
    let _keychain = isolate_keychain();
    let cmd = multitop::ssh::cleanup_old_agents_command();
    // The command should contain "cd ~/.cache/multitop"
    assert!(cmd.contains("cd ~/.cache/multitop"));
    // Should iterate over agent-* files
    assert!(cmd.contains("for f in agent-*"));
    // Should have a case statement
    assert!(cmd.contains("case"));
    // Should have "rm -f" for stale hashes
    assert!(cmd.contains("rm -f"));
    // When agent binaries are embedded, the keep patterns include "continue"
    // In debug builds without agents, keep_patterns may be empty
    if cmd.contains("continue") {
        // Verify the command structure is well-formed
        assert!(cmd.contains("esac"));
        assert!(cmd.contains("done"));
    }
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

    let said = app.panels[0]
        .view
        .iter()
        .any(|l| l.contains("could not save upgrade state"));

    let _ = std::fs::remove_file(&blocker);

    assert!(
        said,
        "a failed state write must reach the panel; the pane said: {:?}",
        app.panels[0].view
    );
}

/// The failed-state-write notice must survive the path that actually runs.
///
/// `confirm_upgrade` marks each host started -- which is where the write
/// happens, and where the notice is pushed -- and *then* calls `run_upgrade`,
/// which clears each panel's `last_upgrade` ring before streaming into it. The
/// panels are already in Upgrade mode by then, so `Panel::note` had put the
/// notice in exactly the buffer the next line clears.
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
    }]);

    let blocker =
        std::env::temp_dir().join(format!("multitop_confirm_blocker_{}", std::process::id()));
    let _ = std::fs::remove_file(&blocker);
    std::fs::write(&blocker, b"not a directory").unwrap();
    app.config_path = Some(blocker.join("config.toml"));

    // The real sequence: `u` to enter the view, then confirm.
    app.enter_upgrade_view();
    let _ = app.confirm_upgrade();

    let ring: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    let said = ring
        .iter()
        .any(|l| l.contains("could not save upgrade state"));

    let _ = std::fs::remove_file(&blocker);

    assert!(
        said,
        "the notice must still be in the pane the operator is looking at; \
         the ring holds: {ring:?}"
    );
}
