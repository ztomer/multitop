use super::*;

#[test]
fn a_piped_fetch_view_is_a_packet_the_client_can_decode() {
    let snap = fetch_snapshot();
    let mut out = Vec::new();
    emit_fetch(&snap, 80, false, &ANSI, &mut out).unwrap();

    let hello = decode_packet(&out).expect("hello packet");
    assert!(matches!(hello, Payload::Hello(_)));
    let declared = u16::from_le_bytes([out[6], out[7]]) as usize;
    let fetch_bytes = &out[8 + declared..];
    let Payload::Fetch(got) = decode_packet(fetch_bytes).expect("must be fetch packet") else {
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

    let hello = decode_packet(&out).expect("hello packet");
    assert!(matches!(hello, Payload::Hello(_)));
    let declared = u16::from_le_bytes([out[6], out[7]]) as usize;
    let docker_bytes = &out[8 + declared..];
    let Payload::Docker { host, rows } = decode_packet(docker_bytes).expect("docker packet") else {
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
