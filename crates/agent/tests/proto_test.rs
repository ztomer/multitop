//! The wire protocol, from both ends.
//!
//! The Docker payload already has framing tests beside the encoder; this
//! covers the Monitor and Fetch payloads and the decoder's behaviour on
//! packets that arrive damaged, which is what a half-read pipe looks like.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_agent::docker::Row as DockerRow;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::{
    decode_packet, encode_packet, Payload, ProtoMode, MAGIC, MODE_DOCKER, MODE_FETCH, MODE_MONITOR,
    PROTO_VERSION,
};
use multitop_agent::render::{Snapshot, TempUnit};

fn snapshot() -> Snapshot {
    Snapshot {
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        host: "web-01 (10.0.0.4)".into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 42.5,
        // One core with a temperature and one without: the absent case is
        // carried as a negative sentinel on the wire and has to come back as
        // `None`, not as -1 degrees.
        cores: vec![(0, 10.5, Some(48.0)), (1, 90.0, None)],
        temp_unit: TempUnit::C,
        mem: Usage::new(16 << 30, 4 << 30),
        disk: Usage::new(512 << 30, 100 << 30),
        rx_rate: 1_250_000.0,
        tx_rate: 640.0,
        procs: vec![
            Proc {
                pid: 1,
                name: "systemd".into(),
                cpu: 0.5,
                mem: 12_000_000,
            },
            Proc {
                pid: 4242,
                name: "an-executable-with-a-long-name".into(),
                cpu: 88.0,
                mem: 900_000_000,
            },
        ],
    }
}

fn fetch_snapshot() -> FetchSnapshot {
    FetchSnapshot {
        user_host: "root@web-01".into(),
        agent_version: "9.9.9".into(),
        os: "Debian GNU/Linux 12 (bookworm)".into(),
        kernel: "6.1.0-18-amd64".into(),
        uptime: "3d 4h 5m".into(),
        host_model: "QEMU Standard PC".into(),
        cpu_model: "AMD EPYC (8)".into(),
        memory_str: "4.0G/16.0G (25%)".into(),
        disk_str: "100.0G/512.0G (20%)".into(),
    }
}

#[test]
fn a_monitor_snapshot_survives_the_wire_intact() {
    let snap = snapshot();
    let decoded = decode_packet(&encode_packet(&Payload::Monitor(snap.clone()))).unwrap();
    let Payload::Monitor(got) = decoded else {
        panic!("wrong payload kind");
    };

    assert_eq!(got.host, snap.host);
    assert_eq!(got.agent_version, snap.agent_version);
    assert!((got.cpu_pct - snap.cpu_pct).abs() < 0.01);
    assert_eq!(got.cores.len(), 2);
    assert_eq!(got.cores[0].0, 0);
    assert!((got.cores[0].2.unwrap() - 48.0).abs() < 0.01);
    assert_eq!(
        got.cores[1].2, None,
        "an absent temperature must stay absent"
    );
    assert_eq!(got.mem.total, snap.mem.total);
    assert_eq!(got.mem.used, snap.mem.used);
    assert_eq!(got.disk.total, snap.disk.total);
    assert!((got.rx_rate - snap.rx_rate).abs() < 1.0);
    assert!((got.tx_rate - snap.tx_rate).abs() < 1.0);
    assert_eq!(got.procs.len(), 2);
    assert_eq!(got.procs[1].pid, 4242);
    assert_eq!(got.procs[1].name, "an-executable-with-a-long-name");
    assert_eq!(got.procs[1].mem, 900_000_000);
}

#[test]
fn the_temperature_unit_survives_the_wire() {
    for unit in [TempUnit::C, TempUnit::F] {
        let mut snap = snapshot();
        snap.temp_unit = unit;
        let Payload::Monitor(got) = decode_packet(&encode_packet(&Payload::Monitor(snap))).unwrap()
        else {
            panic!("wrong payload kind");
        };
        assert_eq!(got.temp_unit, unit);
    }
}

#[test]
fn an_empty_snapshot_still_round_trips() {
    let Payload::Monitor(got) =
        decode_packet(&encode_packet(&Payload::Monitor(Snapshot::default()))).unwrap()
    else {
        panic!("wrong payload kind");
    };
    assert!(got.cores.is_empty());
    assert!(got.procs.is_empty());
}

#[test]
fn a_fetch_snapshot_survives_the_wire_intact() {
    let snap = fetch_snapshot();
    let Payload::Fetch(got) = decode_packet(&encode_packet(&Payload::Fetch(snap.clone()))).unwrap()
    else {
        panic!("wrong payload kind");
    };
    assert_eq!(got, snap);
}

#[test]
fn a_packet_is_labelled_with_the_payload_it_carries() {
    let header_mode = |p: &Payload| encode_packet(p)[5];
    assert_eq!(
        header_mode(&Payload::Monitor(Snapshot::default())),
        MODE_MONITOR
    );
    assert_eq!(
        header_mode(&Payload::Docker {
            host: "h".into(),
            rows: vec![]
        }),
        MODE_DOCKER
    );
    assert_eq!(
        header_mode(&Payload::Fetch(FetchSnapshot::default())),
        MODE_FETCH
    );

    let pkt = encode_packet(&Payload::Fetch(FetchSnapshot::default()));
    assert_eq!(&pkt[..4], MAGIC);
    assert_eq!(pkt[4], PROTO_VERSION);
}

#[test]
fn mode_bytes_map_to_modes_and_back() {
    for (byte, mode) in [
        (0u8, ProtoMode::Monitor),
        (1, ProtoMode::Docker),
        (2, ProtoMode::Fetch),
        (3, ProtoMode::Exec),
        (4, ProtoMode::Hello),
    ] {
        assert_eq!(ProtoMode::try_from(byte).unwrap(), mode);
        assert_eq!(mode.as_u8(), byte);
    }
}

#[test]
fn an_unknown_mode_byte_is_rejected_rather_than_guessed() {
    // 4 is Hello as of protocol 5 companion. The byte chosen here has to be one no mode
    // uses, or this test passes for the wrong reason the moment a mode is
    // added.
    assert_eq!(ProtoMode::try_from(5), Err(5));
    assert_eq!(ProtoMode::try_from(255), Err(255));

    // And the decoder refuses the whole packet rather than returning an
    // arbitrary payload kind.
    let mut pkt = encode_packet(&Payload::Fetch(FetchSnapshot::default()));
    pkt[5] = 7;
    assert!(decode_packet(&pkt).is_none());
}

#[test]
fn a_packet_that_is_not_ours_is_declined() {
    let mut pkt = encode_packet(&Payload::Fetch(FetchSnapshot::default()));
    pkt[0] = b'X';
    assert!(decode_packet(&pkt).is_none(), "bad magic must not decode");
    assert!(decode_packet(b"").is_none());
    assert!(
        decode_packet(b"MTOP").is_none(),
        "a header alone is not a packet"
    );
}

#[test]
fn a_packet_cut_short_decodes_to_nothing_rather_than_to_junk() {
    let full = encode_packet(&Payload::Monitor(snapshot()));
    // Every prefix short of the whole packet: the header says how much body to
    // expect, so each of these must be refused rather than half-parsed.
    for cut in [8, 12, 20, full.len() - 1] {
        assert!(
            decode_packet(&full[..cut]).is_none(),
            "a {cut}-byte prefix decoded as a whole packet"
        );
    }
    assert!(
        decode_packet(&full).is_some(),
        "the whole packet must decode"
    );
}

#[test]
fn a_header_that_lies_about_its_body_is_refused() {
    let mut pkt = encode_packet(&Payload::Fetch(fetch_snapshot()));
    // Claim a body far longer than what follows, which is what a truncated
    // read off the pipe looks like.
    pkt[6] = 0xff;
    pkt[7] = 0xff;
    assert!(decode_packet(&pkt).is_none());
}

#[test]
fn a_body_that_ends_mid_field_is_refused() {
    // Well-formed header, declared length matches, but the body runs out
    // inside the core list. The decoder must not return a partial snapshot.
    let mut pkt = Vec::new();
    pkt.extend_from_slice(MAGIC);
    pkt.push(PROTO_VERSION);
    pkt.push(MODE_MONITOR);
    let body: Vec<u8> = {
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.push(b'h'); // host = "h"
        b.extend_from_slice(&0u16.to_le_bytes()); // agent_version = ""
        b.extend_from_slice(&0f32.to_le_bytes()); // cpu_pct
        b.extend_from_slice(&9u16.to_le_bytes()); // claims nine cores, sends none
        b
    };
    pkt.extend_from_slice(&(u16::try_from(body.len()).unwrap()).to_le_bytes());
    pkt.extend_from_slice(&body);
    assert!(decode_packet(&pkt).is_none());
}

#[test]
fn a_docker_payload_with_no_rows_round_trips() {
    let decoded = decode_packet(&encode_packet(&Payload::Docker {
        host: "empty-host".into(),
        rows: vec![],
    }))
    .unwrap();
    let Payload::Docker { host, rows } = decoded else {
        panic!("wrong payload kind");
    };
    assert_eq!(host, "empty-host");
    assert!(rows.is_empty());
}

#[test]
fn a_docker_row_survives_the_wire_field_for_field() {
    let row = DockerRow {
        name: "web".into(),
        status: "Up 3 days".into(),
        image: "nginx:latest".into(),
        cpu: "12.5%".into(),
        cpu_pct: 12.5,
        mem: "128.0M/512.0M".into(),
        mem_bytes: 134_217_728,
    };
    let decoded = decode_packet(&encode_packet(&Payload::Docker {
        host: "h".into(),
        rows: vec![row.clone()],
    }))
    .unwrap();
    let Payload::Docker { rows, .. } = decoded else {
        panic!("wrong payload kind");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, row.name);
    assert_eq!(rows[0].status, row.status);
    assert_eq!(rows[0].cpu, row.cpu);
    assert_eq!(rows[0].mem, row.mem);
    assert_eq!(rows[0].mem_bytes, row.mem_bytes);
    assert!((rows[0].cpu_pct - row.cpu_pct).abs() < 0.01);
}

#[test]
fn a_string_longer_than_the_length_field_is_truncated_not_wrapped() {
    // The per-string length is a u16. A longer name has to be cut, because a
    // wrapped length would desynchronise every field after it.
    let mut snap = snapshot();
    snap.host = "h".repeat(70_000);
    let pkt = encode_packet(&Payload::Monitor(snap));
    // The packet still describes itself correctly, which is the property that
    // keeps the stream framed.
    let declared = u16::from_le_bytes([pkt[6], pkt[7]]) as usize;
    assert_eq!(declared, pkt.len() - 8);
}

// ------------------------------------------------------ the version in the header

/// Rewrite a packet's protocol-version byte, leaving everything else alone.
fn with_version(mut packet: Vec<u8>, version: u8) -> Vec<u8> {
    packet[4] = version;
    packet
}

fn one_row() -> Vec<multitop_agent::docker::Row> {
    vec![multitop_agent::docker::Row {
        name: "billing-api".into(),
        status: "Up 3 hours".into(),
        image: "registry.example.com/team/billing:2.1".into(),
        cpu: "1.0%".into(),
        cpu_pct: 1.0,
        mem: "-".into(),
        mem_bytes: 0,
    }]
}

#[test]
fn a_docker_packet_from_before_the_image_field_is_refused_not_misread() {
    // Read with the old layout the rows do not fail -- they come out one field
    // shifted, which is plausible nonsense on screen. Refusing is what gets the
    // agent replaced instead.
    let packet = multitop_agent::proto::encode_packet(&Payload::Docker {
        host: "web-01".into(),
        rows: one_row(),
    });
    assert!(
        multitop_agent::proto::decode_packet(&packet).is_some(),
        "the current version must decode"
    );
    assert!(
        multitop_agent::proto::decode_packet(&with_version(packet, 1)).is_none(),
        "a protocol-1 Docker packet was read as if it carried an image"
    );
}

#[test]
fn a_monitor_packet_is_read_at_any_version_so_a_stale_agent_can_be_spotted() {
    // The mismatch that replaces the remote binary is read out of a *decoded*
    // Monitor packet. Refusing those on version alone would leave a stale agent
    // undetectable and unreplaceable.
    let packet = multitop_agent::proto::encode_packet(&Payload::Monitor(snapshot()));
    for version in [0u8, 1, 2, 99] {
        assert!(
            multitop_agent::proto::decode_packet(&with_version(packet.clone(), version)).is_some(),
            "a Monitor packet at protocol {version} could not be read, so the \
             agent behind it could never be replaced"
        );
    }
}

#[test]
fn the_container_image_survives_a_round_trip() {
    let packet = multitop_agent::proto::encode_packet(&Payload::Docker {
        host: "web-01".into(),
        rows: one_row(),
    });
    let Some(Payload::Docker { rows, .. }) = multitop_agent::proto::decode_packet(&packet) else {
        panic!("the packet must decode as Docker");
    };
    assert_eq!(rows[0].image, "registry.example.com/team/billing:2.1");
    assert_eq!(rows[0].name, "billing-api", "the fields came back shifted");
}

#[test]
fn the_core_clock_survives_a_round_trip_and_its_absence_does_too() {
    for mhz in [Some(3600.0), Some(800.5), None] {
        let mut snap = snapshot();
        snap.cpu_mhz = mhz;
        let packet = multitop_agent::proto::encode_packet(&Payload::Monitor(snap));
        let Some(Payload::Monitor(back)) = multitop_agent::proto::decode_packet(&packet) else {
            panic!("the packet must decode as Monitor");
        };
        match (mhz, back.cpu_mhz) {
            (None, got) => assert_eq!(got, None, "an absent clock came back as a number"),
            (Some(want), Some(got)) => assert!((got - want).abs() < 1.0, "{want} -> {got}"),
            (Some(want), None) => panic!("a clock of {want} was lost"),
        }
    }
}

#[test]
fn a_monitor_packet_from_before_the_clock_field_still_decodes() {
    // The invariant this payload is held to: it must be readable at *every*
    // version, because the agent-version mismatch that replaces a stale remote
    // binary is read out of a decoded Monitor packet. A snapshot refused on
    // version alone is a stale agent that can never be noticed or replaced.
    //
    // So the field is read only when the sender says it is there -- and the
    // rest of the payload must still land in the right fields without it.
    let mut snap = snapshot();
    snap.cpu_mhz = Some(3600.0);
    snap.host = "web-01".into();
    let packet = multitop_agent::proto::encode_packet(&Payload::Monitor(snap));

    let old = with_version(packet, 2);
    let Some(Payload::Monitor(back)) = multitop_agent::proto::decode_packet(&old) else {
        panic!("a protocol-2 Monitor packet must still decode");
    };
    assert_eq!(back.host, "web-01", "the fields came back shifted");
    assert_eq!(
        back.cpu_mhz, None,
        "a clock was read out of a packet without one"
    );
    assert!(
        !back.agent_version.is_empty(),
        "the version that triggers replacing the agent was lost"
    );
}

#[test]
fn the_process_name_list_survives_a_round_trip() {
    let mut snap = snapshot();
    snap.proc_names = vec!["nginx".into(), "postgres".into(), "sshd".into()];
    let packet = multitop_agent::proto::encode_packet(&Payload::Monitor(snap));
    let Some(Payload::Monitor(back)) = multitop_agent::proto::decode_packet(&packet) else {
        panic!("the packet must decode as Monitor");
    };
    assert_eq!(back.proc_names, vec!["nginx", "postgres", "sshd"]);
}

#[test]
fn a_monitor_packet_from_before_the_name_list_still_decodes_whole() {
    // The name list is written last, which is what makes gating it on the
    // version safe: everything before it is what protocol 3 already sent, so a
    // reader that stops there has still read a correct snapshot -- including
    // the agent version that triggers replacing the remote binary.
    let mut snap = snapshot();
    snap.host = "web-01".into();
    snap.proc_names = vec!["postgres".into()];
    let packet = multitop_agent::proto::encode_packet(&Payload::Monitor(snap));

    for version in [1u8, 2, 3] {
        let old = with_version(packet.clone(), version);
        let Some(Payload::Monitor(back)) = multitop_agent::proto::decode_packet(&old) else {
            panic!("a protocol-{version} Monitor packet must still decode");
        };
        assert_eq!(
            back.host, "web-01",
            "at v{version} the fields came back shifted"
        );
        assert!(
            back.proc_names.is_empty(),
            "at v{version} a name list was read out of a packet without one"
        );
        assert!(
            !back.agent_version.is_empty(),
            "at v{version} the version that replaces the agent was lost"
        );
    }
}

#[test]
fn a_name_list_too_large_for_one_packet_is_truncated_rather_than_split() {
    // A name is dropped whole or not at all. A half-written string would
    // desynchronise everything after it.
    let mut snap = snapshot();
    snap.proc_names = (0..20_000)
        .map(|i| format!("process-with-a-long-name-{i}"))
        .collect();
    let packet = multitop_agent::proto::encode_packet(&Payload::Monitor(snap));
    let Some(Payload::Monitor(back)) = multitop_agent::proto::decode_packet(&packet) else {
        panic!("an over-long list must still produce a readable packet");
    };
    assert!(!back.proc_names.is_empty(), "everything was dropped");
    assert!(back.proc_names.len() < 20_000, "nothing was dropped");
    for name in &back.proc_names {
        assert!(
            name.starts_with("process-with-a-long-name-"),
            "a name came back cut: {name}"
        );
    }
}
