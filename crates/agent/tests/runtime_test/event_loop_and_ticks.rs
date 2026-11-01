use super::*;

#[test]
fn the_loop_emits_a_frame_before_it_ever_waits() {
    // A client that has just connected must not sit through an interval
    // before seeing anything.
    let mut monitor = Monitor::new("h".into());
    let mut out = Vec::new();
    let mut ticks = 0;
    let mut next = || {
        ticks += 1;
        None
    };
    monitor_loop(
        &mut monitor,
        &Args::default(),
        false,
        &pal(),
        &PanelSize::default(),
        &mut out,
        &mut next,
    );

    assert_eq!(ticks, 1, "the loop waited before emitting");
    assert!(decode_packet(&out).is_some(), "no frame was emitted");
}

#[test]
fn the_loop_runs_until_the_tick_source_stops_it() {
    let mut monitor = Monitor::new("h".into());
    let mut out = Vec::new();
    let mut remaining = 3;
    let mut next = move || {
        remaining -= 1;
        (remaining > 0).then_some(2.0)
    };
    monitor_loop(
        &mut monitor,
        &Args::default(),
        false,
        &pal(),
        &PanelSize::default(),
        &mut out,
        &mut next,
    );

    // Hello packet followed by three ticks, three frames — four packets total.
    let mut rest = &out[..];
    let mut frames = 0;
    while !rest.is_empty() {
        let declared = u16::from_le_bytes([rest[6], rest[7]]) as usize;
        assert!(
            decode_packet(rest).is_some(),
            "frame {frames} did not decode"
        );
        rest = &rest[8 + declared..];
        frames += 1;
    }
    assert_eq!(frames, 4);
}

#[test]
fn the_loop_stops_when_the_reader_hangs_up() {
    // The write fails; the loop must return rather than spin emitting frames
    // into a broken pipe forever.
    let mut monitor = Monitor::new("h".into());
    // 1 write for Hello, 1 for the first frame, fails on subsequent write
    let mut sink = FailsAfter { writes_left: 2 };
    let mut ticks = 0;
    let mut next = || {
        ticks += 1;
        Some(2.0)
    };
    monitor_loop(
        &mut monitor,
        &Args::default(),
        false,
        &pal(),
        &PanelSize::default(),
        &mut sink,
        &mut next,
    );
    assert_eq!(ticks, 1, "the loop kept going after the pipe broke");
}

#[test]
fn the_loop_draws_to_a_terminal_when_there_is_one() {
    let args = Args {
        cols: 90,
        lines: 20,
        ..Args::default()
    };
    let mut monitor = Monitor::new("web-01".into());
    let mut out = Vec::new();
    let mut once = true;
    let mut next = || {
        let go = once;
        once = false;
        go.then_some(1.0)
    };
    monitor_loop(
        &mut monitor,
        &args,
        true,
        &multitop_agent::palette_view::PaletteView::for_theme(&PLAIN),
        &PanelSize::default(),
        &mut out,
        &mut next,
    );

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains(&fullwidth("web-01")));
    assert!(text.starts_with("\x1b[H\x1b[J"));
}

// ------------------------------------------------------------ live monitor

#[test]
fn a_tick_samples_this_host_and_names_it() {
    let mut monitor = Monitor::new("my-host".into());
    assert_eq!(monitor.host(), "my-host");

    let snap = monitor.tick(2.0, 120, 50, SortBy::Cpu);
    assert_eq!(snap.host, "my-host");
    multitop_testassert::assert_not_empty!(snap.agent_version);
    assert!(snap.cpu_pct >= 0.0);
    assert!(snap.mem.total > 0, "a tick must report real memory");
    assert!(snap.disk.total > 0, "a tick must report real disk");

    // A second tick has a baseline to subtract, so the rates are real.
    let next = monitor.tick(2.0, 120, 50, SortBy::Mem);
    assert!(next.rx_rate >= 0.0 && next.tx_rate >= 0.0);
}

#[test]
fn a_tick_never_asks_for_more_processes_than_the_frame_can_hold() {
    let mut monitor = Monitor::new("h".into());
    // A frame with almost no room must not come back with a full process list.
    let tiny = monitor.tick(1.0, 40, 6, SortBy::Cpu).procs.len();
    let roomy = monitor.tick(1.0, 200, 60, SortBy::Cpu).procs.len();
    assert!(
        tiny <= roomy,
        "a smaller frame asked for more processes than a larger one"
    );
}

#[test]
fn a_zero_interval_tick_reports_no_rate_rather_than_dividing_by_zero() {
    let mut monitor = Monitor::new("h".into());
    let snap = monitor.tick(0.0, 120, 50, SortBy::Cpu);
    assert_eq!(
        (snap.rx_rate).partial_cmp(&(0.0)),
        Some(std::cmp::Ordering::Equal),
        "{:?} vs {:?}",
        snap.rx_rate,
        0.0
    );
    assert_eq!(
        (snap.tx_rate).partial_cmp(&(0.0)),
        Some(std::cmp::Ordering::Equal),
        "{:?} vs {:?}",
        snap.tx_rate,
        0.0
    );
}

// ------------------------------------------------------------------- setup

#[test]
fn no_color_selects_the_palette_with_no_escapes_in_it() {
    // The variable is read at call time, so this only asserts the branch that
    // matches this process's environment — both are exercised directly by the
    // emit tests above.
    let pal = palette_for_env();
    let expected_bold = if std::env::var_os("NO_COLOR").is_some() {
        PLAIN.bold
    } else {
        ANSI.bold
    };
    assert_eq!(pal.bold, expected_bold);
}

#[test]
fn the_stream_samples_for_the_panel_the_client_says_it_has_and_follows_a_resize() {
    // The client draws the panel and the user resizes it at will; the agent
    // only decides how many rows to send. It laid out a pretend 120x50 panel
    // instead, so a full-screen 200x100 panel got 92 rows of the 192 it draws
    // (roadmap M2). The client now says its size on stdin; each tick reads it.
    let panel = PanelSize::default();
    let mut monitor = Monitor::new("h".into());
    let mut out = Vec::new();
    let mut sizes = vec![(40u16, 6u16), (200, 100)].into_iter();
    let mut next = || {
        let (cols, lines) = sizes.next()?;
        // `read_commands` is what the stdin thread runs on the client's bytes.
        multitop_agent::stdin_watch::read_commands(
            format!("size {cols} {lines}\n").as_bytes(),
            &panel,
        );
        Some(1.0)
    };
    monitor_loop(
        &mut monitor,
        &Args::default(),
        false,
        &pal(),
        &panel,
        &mut out,
        &mut next,
    );
    let frames = monitor_frames(&out);
    assert_eq!(frames.len(), 3, "one frame before any size, one per size");
    let budget = |s: &Snapshot, cols, lines| {
        multitop_agent::render::Chrome::of(s, cols, lines).proc_budget(lines)
    };
    let small = budget(&frames[1], 40, 6);
    assert!(
        frames[1].procs.len() <= small,
        "the 40x6 panel got {} rows for a budget of {small}",
        frames[1].procs.len()
    );
    // Stated as a precondition: a host running fewer processes than the
    // default panel holds has no more rows to send a bigger one.
    if frames[0].procs.len() == budget(&frames[0], 120, 50) {
        assert!(
            frames[2].procs.len() > frames[0].procs.len(),
            "a 200x100 panel got {} rows, no more than the default's {}",
            frames[2].procs.len(),
            frames[0].procs.len()
        );
    }
}

/// Every monitor frame in a stream of packets, in order.
fn monitor_frames(mut bytes: &[u8]) -> Vec<Snapshot> {
    let mut frames = Vec::new();
    while bytes.len() >= multitop_agent::proto::HEADER_LEN {
        let len = usize::from(u16::from_le_bytes([bytes[6], bytes[7]]));
        let end = multitop_agent::proto::HEADER_LEN + len;
        if let Some(Payload::Monitor(s)) = decode_packet(&bytes[..end]) {
            frames.push(s);
        }
        bytes = &bytes[end..];
    }
    frames
}

#[test]
fn only_a_well_formed_size_line_is_a_size() {
    use multitop_agent::stdin_watch::{parse_size_line, read_commands};
    assert_eq!(parse_size_line(b"size 200 100"), Some((200, 100)));
    for junk in [
        &b"size 0 24"[..],
        b"size 80",
        b"size 80 24 1",
        b"width 80 24",
        b"size 99999 24",
        b"\xff",
    ] {
        assert_eq!(parse_size_line(junk), None, "{junk:?}");
    }
    // An over-long line is passed over whole; the size after it still lands.
    let panel = PanelSize::default();
    let noise = "x".repeat(500);
    read_commands(format!("{noise}\nbogus\nsize 80 24\n").as_bytes(), &panel);
    assert_eq!(panel.get(), Some((80, 24)));
    assert_eq!(PanelSize::default().get(), None);
}
