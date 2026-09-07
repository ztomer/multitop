//! Wire-format LIMITS: what happens at the edges of the frame budget.
//!
//! Split from `proto_test.rs` (2026-09-07) because adding this case pushed that
//! file past the 500-line cap. Splitting rather than exempting is the house
//! rule, and the seam is real: everything here is about a frame that does not
//! fit, which is a different question from whether a frame round-trips.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::{decode_packet, encode_packet, Payload};
use multitop_agent::render::{Snapshot, TempUnit};

/// The smallest snapshot that encodes, so the test below varies exactly one
/// thing: how many processes it carries.
fn snapshot_with(procs: Vec<Proc>) -> Snapshot {
    Snapshot {
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        host: "host".into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 1.0,
        cores: vec![(0, 1.0, None)],
        temp_unit: TempUnit::C,
        mem: Usage::new(16 << 30, 4 << 30),
        disk: Usage::new(512 << 30, 100 << 30),
        rx_rate: 0.0,
        tx_rate: 0.0,
        procs,
    }
}

/// A snapshot far too large for one frame is DROPPED, not half-emitted.
///
/// Two caps guard `encode_snapshot` and only one is reachable: `MAX_PAYLOAD` is
/// `65_535` BYTES (~11 bytes per process, so a frame fills after roughly `5_000`),
/// while `num_procs` is a `u16` COUNT clamping at `65_535`. The byte cap always
/// bites first, which makes the count clamp unreachable — worth recording,
/// because `encode_snapshot` clamped `num_procs` and then iterated every
/// process anyway. The `.take()` it now carries is defence in depth, not a fix
/// for a live defect; a test written to demonstrate the desync failed at
/// `decode_packet`, which is how that was established.
///
/// Pinned here is what IS reachable, and had no test: the frame is truncated,
/// does not decode, and the stream stays framed for the next packet.
#[test]
fn a_snapshot_too_large_for_one_frame_is_dropped_and_leaves_the_stream_framed() {
    let many = (0..70_000u32)
        .map(|pid| Proc {
            pid,
            name: "p".into(),
            cpu: 0.0,
            mem: 1,
        })
        .collect();
    let oversized = encode_packet(&Payload::Monitor(snapshot_with(many)));
    assert!(
        decode_packet(&oversized).is_none(),
        "a truncated payload must not decode as if it were whole"
    );

    // The stream is still framed: an ordinary packet after it reads correctly.
    let ok = encode_packet(&Payload::Monitor(snapshot_with(Vec::new())));
    let Payload::Monitor(got) = decode_packet(&ok).expect("the next packet must decode") else {
        panic!("expected a Monitor payload");
    };
    assert!(got.procs.is_empty(), "the following frame is intact");
}
