#![no_main]

use libfuzzer_sys::fuzz_target;
use multitop_agent::{color, docker, proto, render, SortBy};

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
                // Test rendering docker payload
                let pal = &color::ANSI;
                for &(cols, lines) in &[(40, 10), (80, 24), (120, 40)] {
                    let _ = docker::render(&host, cols, lines, &rows, pal, SortBy::Cpu);
                    let _ = docker::render(&host, cols, lines, &rows, pal, SortBy::Mem);
                }
            }
        }
    }
});
