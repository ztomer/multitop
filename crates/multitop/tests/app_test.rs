use multitop::app::*;
use multitop::config::Server;
use multitop::consts::MAX_AUX_LINES;

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
    let a = app(2);
    assert_eq!(a.panels.len(), 2);
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Monitor);
        assert!(text(p).contains("connecting..."));
    }
}

#[test]
fn empty_server_list_is_allowed() {
    let mut a = app(0);
    assert!(a.panels.is_empty());
    assert!(a.toggle_docker().is_empty());
    assert!(a.switch_stats().is_empty());
}

#[test]
fn frame_is_shown_in_monitor_mode() {
    let mut a = app(1);
    a.apply(Msg::Frame {
        panel: 0,
        lines: vec!["line1".into(), "line2".into()],
    });
    assert_eq!(text(&a.panels[0]), "line1\nline2");
}

#[test]
fn frame_is_stored_but_hidden_in_docker_mode() {
    let mut a = app(1);
    a.toggle_docker();
    a.apply(Msg::Frame {
        panel: 0,
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
    let mut a = app(1);
    a.apply(Msg::Frame {
        panel: 9,
        lines: vec!["x".into()],
    });
    assert!(text(&a.panels[0]).contains("connecting"));
}

#[test]
fn toggle_docker_switches_every_panel_at_once() {
    let mut a = app(3);
    let cmds = a.toggle_docker();
    assert_eq!(cmds.len(), 3);
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Docker);
        assert!(text(p).contains("Docker loading"));
    }
}

#[test]
fn toggle_docker_returns_every_panel_to_monitor() {
    let mut a = app(3);
    a.toggle_docker();
    let cmds = a.toggle_docker();
    assert!(cmds.is_empty());
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Monitor);
    }
}

#[test]
fn toggling_back_restores_the_last_frame() {
    let mut a = app(3);
    for i in 0..3 {
        a.apply(Msg::Frame {
            panel: i,
            lines: vec![format!("data{i}")],
        });
    }
    a.toggle_docker();
    a.toggle_docker();
    for (i, p) in a.panels.iter().enumerate() {
        assert_eq!(text(p), format!("data{i}"), "panel {i}");
    }
}

#[test]
fn toggling_back_without_data_says_waiting() {
    let mut a = app(1);
    a.toggle_docker();
    a.toggle_docker();
    assert!(text(&a.panels[0]).contains("waiting for data"));
}

#[test]
fn switch_stats_from_docker() {
    let mut a = app(3);
    a.toggle_docker();
    a.switch_stats();
    for p in &a.panels {
        assert_eq!(p.mode, Mode::Monitor);
    }
}

#[test]
fn every_transition_bumps_the_generation() {
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
    let mut a = app(1);
    let gen = a.panels[0].gen;
    a.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: None,
    });
    for i in 0..MAX_AUX_LINES + 500 {
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: format!("l{i}"),
        });
    }
    let view = &a.panels[0].view;
    assert_eq!(view.len(), MAX_AUX_LINES);
    assert_eq!(view.last().unwrap(), &format!("l{}", MAX_AUX_LINES + 499));
}

#[test]
fn upgrade_without_command_explains_itself() {
    let mut a = app(1);
    let cmds = a.run_upgrade();
    assert!(cmds.is_empty(), "nothing to run");
    assert!(text(&a.panels[0]).contains("No upgrade_cmd"));
}

#[test]
fn upgrade_with_command_is_scheduled() {
    let mut servers = servers(2);
    servers[0].upgrade_cmd = Some("apt upgrade -y".into());
    let mut a = App::new(servers);
    let cmds = a.run_upgrade();
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::RunUpgrade { panel: 0, .. }));
    assert!(text(&a.panels[0]).contains("Upgrade running"));
    assert!(text(&a.panels[1]).contains("No upgrade_cmd"));
}

#[test]
fn upgrade_output_persists_until_dismissed() {
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
    });
    a.apply(Msg::Frame {
        panel: 0,
        lines: vec!["cpu stats".into()],
    });

    assert!(text(&a.panels[0]).contains("42 packages upgraded"));
    a.switch_stats();
    assert_eq!(text(&a.panels[0]), "cpu stats");
}

#[test]
fn status_respects_generation() {
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
    });
    assert!(text(&a.panels[0]).contains("exit 1"));
}

#[test]
fn quit_sets_the_flag() {
    let mut a = app(1);
    assert!(!a.should_quit);
    a.quit();
    assert!(a.should_quit);
}

#[test]
fn helpers_wrap_in_ansi() {
    assert!(error_line("boom").contains("boom"));
    assert!(status_line("wait").contains("wait"));
    assert!(header_line("hi").contains("hi"));
}
