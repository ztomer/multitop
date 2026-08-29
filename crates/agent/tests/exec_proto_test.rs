//! L0 — the exec framing itself.
//!
//! Every property here is one the raw-text channel could not have: that a
//! record's end is stated rather than guessed, that bytes survive the trip
//! unaltered, and that a reader which cannot understand a frame says so instead
//! of carrying on from the wrong offset.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_agent::exec::{chunks, ExecFrame, MarkerKind, Stream, MAX_EXEC_CHUNK};
use multitop_agent::proto::{decode_packet, encode_packet, Payload};

fn round_trip(frame: &ExecFrame) -> ExecFrame {
    let pkt = encode_packet(&Payload::Exec(frame.clone()));
    match decode_packet(&pkt).expect("a packet this build wrote must decode") {
        Payload::Exec(got) => got,
        other => panic!("wrong payload kind: {other:?}"),
    }
}

#[test]
fn every_frame_survives_the_round_trip() {
    let frames = vec![
        ExecFrame::Request {
            command: "sudo apt update && sudo apt upgrade -y".into(),
            password: Some("hunter2".into()),
            use_lock: true,
            cols: 203,
            rows: 51,
        },
        ExecFrame::Request {
            command: "ls -la /etc".into(),
            password: None,
            use_lock: false,
            cols: 80,
            rows: 24,
        },
        ExecFrame::Begin {
            host: "beelink (192.168.0.33)".into(),
            agent_version: "0.42.1".into(),
            pid: 4_294_967_295,
        },
        ExecFrame::Out {
            stream: Stream::Stdout,
            seq: 7,
            bytes: b"Reading package lists... Done\r\n".to_vec(),
        },
        ExecFrame::Out {
            stream: Stream::Stderr,
            seq: 8,
            bytes: b"E: Could not get lock\n".to_vec(),
        },
        ExecFrame::Marker(MarkerKind::PwReady),
        ExecFrame::Marker(MarkerKind::SudoFailed),
        ExecFrame::Marker(MarkerKind::LockHeld),
        ExecFrame::Alive { elapsed_ms: 90_061 },
        ExecFrame::Exit {
            code: 0,
            signalled: false,
        },
        ExecFrame::Exit {
            code: 143,
            signalled: true,
        },
    ];
    for f in &frames {
        assert_eq!(&round_trip(f), f, "frame did not survive: {f:?}");
    }
}

/// An empty password is a password; a missing one is not. Encoding the absent
/// case as an empty string would turn "this host needs no sudo" into "this
/// host's sudo password is the empty string", and the difference decides
/// whether a preamble waits on a `read` that never gets an answer.
#[test]
fn an_absent_password_is_distinct_from_an_empty_one() {
    let absent = ExecFrame::Request {
        command: "ls".into(),
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    let empty = ExecFrame::Request {
        command: "ls".into(),
        password: Some(String::new()),
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    assert_eq!(round_trip(&absent), absent);
    assert_eq!(round_trip(&empty), empty);
    assert_ne!(round_trip(&absent), empty);
}

/// Terminal output is not guaranteed to be UTF-8 -- a `latin-1` locale, a
/// binary accidentally `cat`ed, a partial multi-byte character split across two
/// reads. `String::from_utf8_lossy` would rewrite those bytes into replacement
/// characters, and what the child wrote is what the operator has to see.
#[test]
fn out_carries_bytes_that_are_not_utf8() {
    let raw = vec![0x1b, b'[', b'3', b'1', b'm', 0xff, 0xfe, 0x00, b'x', 0x80];
    let frame = ExecFrame::Out {
        stream: Stream::Stdout,
        seq: 0,
        bytes: raw.clone(),
    };
    match round_trip(&frame) {
        ExecFrame::Out { bytes, .. } => assert_eq!(bytes, raw),
        other => panic!("wrong frame: {other:?}"),
    }
}

/// The length field is a `u16`. A payload past that ceiling cannot describe
/// itself, and the encoder's backstop truncates -- which for output means lines
/// silently missing from an operator's log. Chunking at the source is what
/// keeps the backstop unreachable.
#[test]
fn no_chunk_can_overflow_the_length_field() {
    let big = vec![b'x'; 5 * MAX_EXEC_CHUNK + 17];
    let mut seen = 0usize;
    for chunk in chunks(&big) {
        assert!(chunk.len() <= MAX_EXEC_CHUNK, "chunk of {}", chunk.len());
        let pkt = encode_packet(&Payload::Exec(ExecFrame::Out {
            stream: Stream::Stdout,
            seq: 0,
            bytes: chunk.to_vec(),
        }));
        let declared = u16::from_le_bytes([pkt[6], pkt[7]]) as usize;
        assert_eq!(
            declared,
            pkt.len() - 8,
            "header and body disagree, which desynchronises the stream"
        );
        seen += chunk.len();
    }
    assert_eq!(seen, big.len(), "chunking must not lose bytes");
}

/// An exit code is signed. Read back as unsigned, 255 becomes 4294967295 and
/// -1 becomes a success-shaped number; the operator is told the wrong thing
/// about whether their upgrade worked.
#[test]
fn a_negative_exit_code_survives() {
    for code in [-1, 0, 1, 111, 125, 143, 255, i32::MIN, i32::MAX] {
        let f = ExecFrame::Exit {
            code,
            signalled: false,
        };
        assert_eq!(round_trip(&f), f, "code {code}");
    }
}

/// Refused rather than misread. An Exec frame parsed with an older layout does
/// not fail -- it comes out one field along, and here that nonsense would be an
/// exit code.
#[test]
fn an_older_protocol_version_is_refused_not_misread() {
    let mut pkt = encode_packet(&Payload::Exec(ExecFrame::Exit {
        code: 0,
        signalled: false,
    }));
    pkt[4] = 4;
    assert!(
        decode_packet(&pkt).is_none(),
        "a v4 Exec frame must be refused"
    );
}

/// A short read must not be parsed as a whole packet.
#[test]
fn a_truncated_packet_is_refused() {
    let pkt = encode_packet(&Payload::Exec(ExecFrame::Begin {
        host: "h".into(),
        agent_version: "0.1.0".into(),
        pid: 1,
    }));
    for cut in 1..pkt.len() {
        assert!(
            decode_packet(&pkt[..cut]).is_none(),
            "{cut} of {} bytes decoded as a whole packet",
            pkt.len()
        );
    }
}

/// The frames are not self-delimiting inside the payload, so a reader that does
/// not know a kind does not know its length either and cannot honestly skip it.
#[test]
fn an_unknown_frame_kind_is_refused() {
    let mut pkt = encode_packet(&Payload::Exec(ExecFrame::Marker(MarkerKind::PwReady)));
    pkt[8] = 99;
    assert!(decode_packet(&pkt).is_none());
}

/// A stream byte the reader does not recognise would otherwise default to one
/// of the two, and silently mis-colour the reason a run failed.
#[test]
fn an_unknown_stream_byte_is_refused() {
    let mut pkt = encode_packet(&Payload::Exec(ExecFrame::Out {
        stream: Stream::Stdout,
        seq: 0,
        bytes: b"x".to_vec(),
    }));
    pkt[9] = 7;
    assert!(decode_packet(&pkt).is_none());
}

/// Only `Exit` ends a run. If anything else were terminal, a client could stop
/// reading early and report a result the host never gave it.
#[test]
fn exit_is_the_only_terminal_frame() {
    let non_terminal = [
        ExecFrame::Begin {
            host: "h".into(),
            agent_version: "v".into(),
            pid: 1,
        },
        ExecFrame::Out {
            stream: Stream::Stdout,
            seq: 0,
            bytes: vec![],
        },
        ExecFrame::Marker(MarkerKind::LockHeld),
        ExecFrame::Alive { elapsed_ms: 0 },
    ];
    for f in &non_terminal {
        assert!(!f.is_terminal(), "{f:?} must not end a run");
    }
    assert!(ExecFrame::Exit {
        code: 0,
        signalled: false
    }
    .is_terminal());
}

/// Reordering the enum must not renumber the wire.
#[test]
fn the_wire_tags_are_pinned() {
    let cases: [(ExecFrame, u8); 6] = [
        (
            ExecFrame::Request {
                command: String::new(),
                password: None,
                use_lock: false,
                cols: 0,
                rows: 0,
            },
            0,
        ),
        (
            ExecFrame::Begin {
                host: String::new(),
                agent_version: String::new(),
                pid: 0,
            },
            1,
        ),
        (
            ExecFrame::Out {
                stream: Stream::Stdout,
                seq: 0,
                bytes: vec![],
            },
            2,
        ),
        (ExecFrame::Marker(MarkerKind::PwReady), 3),
        (ExecFrame::Alive { elapsed_ms: 0 }, 4),
        (
            ExecFrame::Exit {
                code: 0,
                signalled: false,
            },
            5,
        ),
    ];
    for (frame, tag) in &cases {
        assert_eq!(frame.kind(), *tag, "{frame:?}");
        let pkt = encode_packet(&Payload::Exec(frame.clone()));
        assert_eq!(pkt[8], *tag, "tag on the wire for {frame:?}");
    }
    assert_eq!(MarkerKind::PwReady.as_u8(), 0);
    assert_eq!(MarkerKind::SudoFailed.as_u8(), 1);
    assert_eq!(MarkerKind::LockHeld.as_u8(), 2);
    assert_eq!(Stream::Stdout.as_u8(), 0);
    assert_eq!(Stream::Stderr.as_u8(), 1);
}
