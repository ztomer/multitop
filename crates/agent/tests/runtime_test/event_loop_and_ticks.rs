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
        &ANSI,
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
        &ANSI,
        &mut out,
        &mut next,
    );

    // Three ticks, three frames — each a whole packet, back to back.
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
    assert_eq!(frames, 3);
}

#[test]
fn the_loop_stops_when_the_reader_hangs_up() {
    // The write fails; the loop must return rather than spin emitting frames
    // into a broken pipe forever.
    let mut monitor = Monitor::new("h".into());
    let mut sink = FailsAfter { writes_left: 1 };
    let mut ticks = 0;
    let mut next = || {
        ticks += 1;
        Some(2.0)
    };
    monitor_loop(
        &mut monitor,
        &Args::default(),
        false,
        &ANSI,
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
    monitor_loop(&mut monitor, &args, true, &PLAIN, &mut out, &mut next);

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
    assert!(!snap.agent_version.is_empty());
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
    let tiny = monitor.tick(1.0, 40, 6, SortBy::Cpu);
    let roomy = monitor.tick(1.0, 200, 60, SortBy::Cpu);
    assert!(
        tiny.procs.len() <= roomy.procs.len(),
        "a smaller frame asked for more processes than a larger one"
    );
}

#[test]
fn a_zero_interval_tick_reports_no_rate_rather_than_dividing_by_zero() {
    let mut monitor = Monitor::new("h".into());
    let snap = monitor.tick(0.0, 120, 50, SortBy::Cpu);
    assert_eq!(snap.rx_rate, 0.0);
    assert_eq!(snap.tx_rate, 0.0);
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
