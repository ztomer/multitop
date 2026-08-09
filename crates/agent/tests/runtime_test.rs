//! The agent's three views and its repaint loop, driven into a byte sink.
//!
//! `run_agent` itself owns stdout and never returns in monitor mode, so the
//! parts worth pinning are the emitters and the loop: given a snapshot and a
//! writer, what goes on the wire, and what makes the loop stop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, Write};

use multitop_agent::color::{ANSI, PLAIN};
use multitop_agent::docker::Row as DockerRow;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::fmt::fullwidth;
use multitop_agent::monitor::Monitor;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::{decode_packet, Payload};
use multitop_agent::render::Snapshot;
use multitop_agent::{
    emit_docker, emit_fetch, emit_monitor, monitor_loop, palette_for_env, parse_args, Args, Mode,
    SortBy,
};

/// A writer that fails on the nth write, standing in for the reader hanging up.
struct FailsAfter {
    writes_left: usize,
}

impl Write for FailsAfter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.writes_left == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "reader gone"));
        }
        self.writes_left -= 1;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        host: "web-01".into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 12.0,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        cores: vec![(0, 5.0, Some(40.0))],
        mem: Usage::new(8 << 30, 2 << 30),
        disk: Usage::new(256 << 30, 64 << 30),
        rx_rate: 1000.0,
        tx_rate: 2000.0,
        procs: vec![Proc {
            pid: 1,
            name: "init".into(),
            cpu: 1.0,
            mem: 1024,
        }],
        ..Default::default()
    }
}

fn fetch_snapshot() -> FetchSnapshot {
    FetchSnapshot {
        user_host: "root@web-01".into(),
        agent_version: "9.9.9".into(),
        os: "Debian GNU/Linux 12".into(),
        kernel: "6.1.0".into(),
        uptime: "3d 4h 5m".into(),
        host_model: "QEMU".into(),
        cpu_model: "AMD EPYC (8)".into(),
        memory_str: "2.0G/8.0G (25%)".into(),
        disk_str: "64.0G/256.0G (25%)".into(),
    }
}

fn docker_rows() -> Vec<DockerRow> {
    vec![
        DockerRow {
            name: "web".into(),
            status: "Up 3 days".into(),
            image: "nginx:latest".into(),
            cpu: "12.5%".into(),
            cpu_pct: 12.5,
            mem: "128.0M/512.0M".into(),
            mem_bytes: 134_217_728,
        },
        DockerRow {
            name: "db".into(),
            status: "Up 1 hour".into(),
            image: "nginx:latest".into(),
            cpu: "90.0%".into(),
            cpu_pct: 90.0,
            mem: "1.0G/2.0G".into(),
            mem_bytes: 1 << 30,
        },
    ]
}

// ------------------------------------------------------------------- fetch

#[test]
fn a_piped_fetch_view_is_a_packet_the_client_can_decode() {
    let snap = fetch_snapshot();
    let mut out = Vec::new();
    emit_fetch(&snap, 80, false, &ANSI, &mut out).unwrap();

    let Payload::Fetch(got) = decode_packet(&out).expect("must be one whole packet") else {
        panic!("wrong payload kind");
    };
    assert_eq!(got, snap);
}

#[test]
fn a_fetch_view_on_a_terminal_is_text_with_every_field_labelled() {
    let mut out = Vec::new();
    emit_fetch(&fetch_snapshot(), 80, true, &PLAIN, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    // The header is drawn in fullwidth glyphs, so it is the transformed
    // spelling that has to be present.
    assert!(
        text.contains(&fullwidth("root@web-01")),
        "the header names the host"
    );
    for label in ["OS", "Kernel", "Uptime", "Host", "CPU", "Memory", "Disk"] {
        assert!(text.contains(label), "{label} row missing from:\n{text}");
    }
    assert!(text.contains("Debian GNU/Linux 12"));
    assert!(text.contains("3d 4h 5m"));
    // The plain palette must not smuggle escapes into a NO_COLOR terminal.
    assert!(!text.contains('\x1b'), "plain palette emitted an escape");
}

#[test]
fn a_fetch_view_that_cannot_be_written_reports_the_failure() {
    let mut sink = FailsAfter { writes_left: 0 };
    assert!(emit_fetch(&fetch_snapshot(), 80, false, &ANSI, &mut sink).is_err());
    assert!(emit_fetch(&fetch_snapshot(), 80, true, &ANSI, &mut sink).is_err());
}

// ------------------------------------------------------------------ docker

#[test]
fn a_piped_docker_view_is_a_packet_carrying_every_row() {
    let mut out = Vec::new();
    emit_docker(
        "web-01",
        docker_rows(),
        &Args::default(),
        false,
        &ANSI,
        &mut out,
    )
    .unwrap();

    let Payload::Docker { host, rows } = decode_packet(&out).expect("one whole packet") else {
        panic!("wrong payload kind");
    };
    assert_eq!(host, "web-01");
    assert_eq!(rows.len(), 2);
}

#[test]
fn a_docker_view_on_a_terminal_is_a_drawn_table() {
    let args = Args {
        cols: 100,
        lines: 24,
        ..Args::default()
    };
    let mut out = Vec::new();
    emit_docker("web-01", docker_rows(), &args, true, &PLAIN, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(text.contains("NAME"));
    assert!(text.contains("STATUS"));
    assert!(text.contains("web"));
    assert!(text.contains("db"));
}

#[test]
fn an_empty_docker_view_says_so_rather_than_drawing_a_bare_frame() {
    let mut out = Vec::new();
    emit_docker("web-01", vec![], &Args::default(), true, &PLAIN, &mut out).unwrap();
    assert!(String::from_utf8(out)
        .unwrap()
        .contains("No running containers"));
}

#[test]
fn a_docker_view_that_cannot_be_written_reports_the_failure() {
    let mut sink = FailsAfter { writes_left: 0 };
    assert!(emit_docker(
        "h",
        docker_rows(),
        &Args::default(),
        false,
        &ANSI,
        &mut sink
    )
    .is_err());
    assert!(emit_docker("h", docker_rows(), &Args::default(), true, &ANSI, &mut sink).is_err());
}

// ----------------------------------------------------------------- monitor

#[test]
fn a_piped_monitor_frame_is_a_packet_the_client_can_decode() {
    let mut buf = String::new();
    let mut out = Vec::new();
    emit_monitor(
        &snapshot(),
        &Args::default(),
        false,
        &ANSI,
        &mut buf,
        &mut out,
    )
    .unwrap();

    let Payload::Monitor(got) = decode_packet(&out).expect("one whole packet") else {
        panic!("wrong payload kind");
    };
    assert_eq!(got.host, "web-01");
    assert_eq!(got.procs.len(), 1);
    // Nothing was drawn, so the render buffer stays untouched.
    assert!(buf.is_empty());
}

#[test]
fn a_monitor_frame_on_a_terminal_repaints_in_place() {
    let args = Args {
        cols: 100,
        lines: 24,
        ..Args::default()
    };
    let mut buf = String::new();
    let mut out = Vec::new();
    emit_monitor(&snapshot(), &args, true, &ANSI, &mut buf, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    // Home-and-clear, so consecutive frames overwrite rather than scroll.
    assert!(
        text.starts_with("\x1b[H\x1b[J"),
        "frame does not reset the cursor"
    );
    assert!(text.contains(&fullwidth("web-01")));
    assert!(text.contains("init"));
}

#[test]
fn the_render_buffer_is_reused_without_accumulating_frames() {
    let args = Args {
        cols: 80,
        lines: 20,
        ..Args::default()
    };
    let mut buf = String::from("left over from an earlier frame");
    let mut out = Vec::new();
    emit_monitor(&snapshot(), &args, true, &ANSI, &mut buf, &mut out).unwrap();
    assert!(
        !buf.contains("left over"),
        "stale frame text survived into the next frame"
    );

    let first = buf.len();
    emit_monitor(&snapshot(), &args, true, &ANSI, &mut buf, &mut out).unwrap();
    assert_eq!(buf.len(), first, "the buffer grew across frames");
}

// -------------------------------------------------------------------- loop

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

#[test]
fn the_sort_word_matches_the_argument_that_selects_it() {
    assert_eq!(SortBy::Cpu.word(), "cpu");
    assert_eq!(SortBy::Mem.word(), "mem");
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "mem".into()
        ])
        .sort,
        SortBy::Mem
    );
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "MEMORY".into()
        ])
        .sort,
        SortBy::Mem
    );
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "cpu".into()
        ])
        .sort,
        SortBy::Cpu
    );
    // An unrecognised word leaves the default alone rather than failing.
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "sideways".into()
        ])
        .sort,
        SortBy::Cpu
    );
}

#[test]
fn each_mode_has_the_word_that_selects_it_on_the_command_line() {
    for (word, mode) in [
        ("monitor", Mode::Monitor),
        ("docker", Mode::Docker),
        ("fetch", Mode::Fetch),
    ] {
        assert_eq!(mode.word(), word);
        assert_eq!(parse_args([word.to_string()]).mode, mode);
    }
}

// ---------------------------------------------------------------- entry point

#[test]
fn the_fetch_mode_entry_point_runs_end_to_end() {
    // Under `cargo test` stdout is captured, so this takes the piped branch
    // and writes a packet to the captured buffer. Nothing to assert on but
    // that the whole path — sample, encode, write — completes.
    multitop_agent::run_agent(["fetch".to_string(), String::new(), "80".into(), "24".into()]);
}

#[test]
fn watching_stdin_for_a_hangup_is_safe_wherever_stdin_came_from() {
    // Only a pipe is watched. On a host with no `/proc/self/fd/0` — every
    // macOS build machine — this must decide "not a pipe" rather than fail.
    let gone = multitop_agent::stdin_eof_watcher();
    assert!(!gone.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn collecting_from_the_ambient_daemon_never_panics() {
    // Whether this host runs Docker or not, the answer is a table.
    let _ = multitop_agent::docker::collect();
}

// -------------------------------------------------------------- tiny frames

#[test]
fn a_frame_with_no_room_at_all_still_produces_something() {
    use multitop_agent::render::{bar_len_for, render, Chrome};

    for (cols, lines) in [(1usize, 1usize), (4, 1), (10, 2), (20, 3), (0, 0)] {
        let snap = snapshot();
        let chrome = Chrome::of(&snap, cols, lines);
        // Below the smallest useful frame the chrome asks for no CPU rows at
        // all rather than a negative or wrapped count.
        let _ = chrome.cpu_rows();
        assert!(chrome.height() >= 1, "chrome claimed a zero-height frame");
        let frame = render(&snap, cols, lines, bar_len_for(cols), &PLAIN);
        assert_eq!(
            frame.len(),
            chrome.height() + chrome.table_height(snap.procs.len()),
            "the predicted height disagreed with what was drawn"
        );
    }
}

// ------------------------------------------------------------ docker sorting

#[test]
fn the_docker_table_can_be_ordered_by_memory_instead_of_cpu() {
    use multitop_agent::docker::render;

    // `db` uses the most memory; `web` uses the most CPU. Which one leads
    // has to follow the sort the user picked.
    let rows = docker_rows();
    let by_cpu = render("h", 100, 24, &rows, &PLAIN, SortBy::Cpu);
    let by_mem = render("h", 100, 24, &rows, &PLAIN, SortBy::Mem);

    // Frame layout: header, column titles, rule, then the body rows.
    let first_body = |frame: &[String]| frame[3].clone();
    assert!(first_body(&by_cpu).contains("db"), "cpu order: {by_cpu:?}");
    assert!(first_body(&by_mem).contains("db"), "mem order: {by_mem:?}");

    // Now make the leaders differ, so the two orders cannot coincide.
    let mut rows = docker_rows();
    rows[0].cpu_pct = 99.0; // web: most cpu
    rows[0].mem_bytes = 1; //        least memory
    let by_cpu = render("h", 100, 24, &rows, &PLAIN, SortBy::Cpu);
    let by_mem = render("h", 100, 24, &rows, &PLAIN, SortBy::Mem);
    assert!(first_body(&by_cpu).contains("web"));
    assert!(first_body(&by_mem).contains("db"));
}

#[test]
fn a_docker_table_taller_than_the_frame_says_how_much_it_hid() {
    use multitop_agent::docker::render;

    let rows: Vec<DockerRow> = (0u32..40)
        .map(|i| DockerRow {
            name: format!("c{i}"),
            status: "Up".into(),
            image: "nginx:latest".into(),
            cpu: "0.0%".into(),
            cpu_pct: f64::from(i),
            mem: "-".into(),
            mem_bytes: u64::from(i),
        })
        .collect();

    let frame = render("h", 100, 12, &rows, &PLAIN, SortBy::Mem);
    let text = frame.join("\n");
    assert!(
        text.contains("more"),
        "an overflowing table must say so: {text}"
    );
    assert!(frame.len() <= 12, "the frame overflowed its budget");
}

#[test]
fn a_lone_escape_that_is_not_a_csi_is_dropped_rather_than_printed() {
    use multitop_agent::color::strip_ansi;

    // `strip_ansi` only ever expects `CSI ... m`, because that is all the agent
    // emits. Anything else has to vanish rather than reach the screen as a
    // stray glyph — and an ESC not followed by `[` takes the byte after it with
    // it, which is what an escape's second byte would have been. Pinned rather
    // than asserted as ideal: the input cannot arise from the agent, and the
    // property that matters is that no escape byte survives.
    assert_eq!(strip_ansi("before\x1bafter"), "beforefter");
    assert_eq!(strip_ansi("trailing\x1b"), "trailing");
    assert_eq!(strip_ansi("\x1bNfoo"), "foo");
    for out in [
        strip_ansi("before\x1bafter"),
        strip_ansi("trailing\x1b"),
        strip_ansi("\x1bNfoo"),
    ] {
        assert!(!out.contains('\x1b'), "an escape byte survived: {out:?}");
    }
    assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(strip_ansi("plain"), "plain");
}
