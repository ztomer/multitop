#![no_main]

use libfuzzer_sys::fuzz_target;
use multitop_agent::{color, docker, exec, proto, render, SortBy};

fuzz_target!(|data: &[u8]| {
    // 1. Fuzz binary packet decoding
    if let Some(payload) = proto::decode_packet(data) {
        match payload {
            proto::Payload::Monitor(snap) => {
                // Test rendering decoded snapshot at various panel dimensions
                let pal = &color::ANSI;
                for &(cols, lines) in &[(40, 10), (80, 24), (120, 40), (200, 60)] {
                    let _ = render::render(&snap, cols, lines, render::bar_len_for(cols), pal);
                }
            }
            proto::Payload::Docker { host, rows } => {
                let pal = &color::ANSI;
                for &(cols, lines) in &[(40, 10), (80, 24), (120, 40)] {
                    let _ = docker::render(&host, cols, lines, &rows, pal, SortBy::Cpu);
                    let _ = docker::render(&host, cols, lines, &rows, pal, SortBy::Mem);
                }
            }
            proto::Payload::Fetch(snap) => {
                let _ = &snap.user_host;
                let _ = &snap.agent_version;
                let _ = &snap.os;
            }
            // The exec channel, which carries an upgrade's output and its exit
            // status. Worth fuzzing for a reason the other arms are not: an
            // `Out` frame is arbitrary bytes from a remote command rather than
            // a shape the agent built, and the frames are not self-delimiting
            // inside a payload -- a decoder that mis-reads one length is
            // reading the next frame from the wrong offset.
            proto::Payload::Exec(frame) => {
                // Re-encoding is the property that matters: a frame this build
                // accepted must survive the round trip, or the decoder and the
                // encoder disagree about the layout and one of them is writing
                // fields into the wrong slots.
                let round = proto::encode_packet(&proto::Payload::Exec(frame.clone()));
                if let Some(proto::Payload::Exec(again)) = proto::decode_packet(&round) {
                    assert_eq!(frame, again, "an accepted exec frame did not round-trip");
                }
                if let exec::ExecFrame::Out { bytes, .. } = &frame {
                    // The ceiling the length field can express. A chunk past it
                    // cannot describe itself, and the encoder's backstop would
                    // truncate an operator's output rather than say so.
                    assert!(bytes.len() <= exec::MAX_EXEC_CHUNK);
                }
            }
        }
    }
});
