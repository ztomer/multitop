use super::*;

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
    assert_eq!(a.toggle_docker((80, 24)), [] as [multitop::app::Command; 0]);
    assert_eq!(a.switch_stats(), [] as [multitop::app::Command; 0]);
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
    a.toggle_docker((80, 24));
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
    let cmds = a.toggle_docker((80, 24));
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
    a.toggle_docker((80, 24));
    let cmds = a.toggle_docker((80, 24));
    assert_eq!(cmds, [] as [multitop::app::Command; 0]);
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
    a.toggle_docker((80, 24));
    a.switch_stats();
    for (i, p) in a.panels.iter().enumerate() {
        assert_eq!(text(p), format!("data{i}"), "panel {i}");
    }
}

#[test]
fn switch_stats_without_data_says_waiting() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    a.toggle_docker((80, 24));
    a.switch_stats();
    assert!(text(&a.panels[0]).contains("waiting for data"));
}

#[test]
fn switch_stats_from_docker() {
    let _keychain = isolate_keychain();
    let mut a = app(3);
    a.toggle_docker((80, 24));
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
    a.toggle_docker((80, 24));
    let g1 = a.panels[0].gen;
    a.switch_stats();
    let g2 = a.panels[0].gen;
    assert!(g1 > g0 && g2 > g1);
}

#[test]
fn stale_results_are_dropped() {
    let _keychain = isolate_keychain();
    let mut a = app(1);
    let cmds = a.toggle_docker((80, 24));
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
    let cmds = a.toggle_docker((80, 24));
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
    // Row 0 is reserved for the banner `ui::draw` composes over it, so the
    // body starts at row 1. Asserting the whole buffer here encoded the shape
    // that ate a one-line body.
    assert!(text(&a.panels[0]).contains("container list"));
}
