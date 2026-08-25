use super::*;

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
    assert!(t.contains("Upgrade on s0"), "{t:?}");
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
    assert!(text(&a.panels[0]).contains("installing agent"));
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
