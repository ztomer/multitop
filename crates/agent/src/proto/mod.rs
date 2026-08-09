//! Compact binary telemetry protocol (b"MTOP") for streaming metrics over SSH.

use crate::docker::Row as DockerRow;
use crate::fetch::FetchSnapshot;
use crate::render::Snapshot;

pub const MAGIC: &[u8; 4] = b"MTOP";
/// Bumped whenever a payload's field layout changes.
///
/// 2 added the container image to each Docker row, so that `/` can find a host
/// by the image it is running.
///
/// 3 added the current CPU clock to the Monitor snapshot.
///
/// 4 added the full list of process names, so `/` can find a host by something
/// it is running rather than only by something its table had room to show.
pub const PROTO_VERSION: u8 = 4;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoMode {
    Monitor = 0,
    Docker = 1,
    Fetch = 2,
}

impl ProtoMode {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for ProtoMode {
    type Error = u8;
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(ProtoMode::Monitor),
            1 => Ok(ProtoMode::Docker),
            2 => Ok(ProtoMode::Fetch),
            other => Err(other),
        }
    }
}

pub const MODE_MONITOR: u8 = ProtoMode::Monitor.as_u8();
pub const MODE_DOCKER: u8 = ProtoMode::Docker.as_u8();
pub const MODE_FETCH: u8 = ProtoMode::Fetch.as_u8();

#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    Monitor(Snapshot),
    Docker { host: String, rows: Vec<DockerRow> },
    Fetch(FetchSnapshot),
}

/// Bytes of fixed header before the payload: magic(4) + version(1) + mode(1) + len(2).
const HEADER_LEN: usize = 8;
/// The payload length field is a u16, so this is the hard ceiling.
const MAX_PAYLOAD: usize = u16::MAX as usize;

mod decode;
mod encode;

pub use decode::{decode_packet, Cursor};
pub use encode::encode_packet;

#[cfg(test)]
mod framing_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn rows(n: usize) -> Vec<DockerRow> {
        (0..n)
            .map(|i| DockerRow {
                name: format!("container-with-a-fairly-long-name-{i}"),
                status: "Up 3 days (healthy) registry.example.com/team/svc".to_string(),
                image: "nginx:latest".into(),
                cpu: "12.5%".to_string(),
                cpu_pct: 12.5,
                mem: "128.0M/512.0M".to_string(),
                mem_bytes: 134_217_728,
            })
            .collect()
    }

    fn declared_len(pkt: &[u8]) -> usize {
        u16::from_le_bytes([pkt[6], pkt[7]]) as usize
    }

    #[test]
    fn the_header_length_always_matches_the_body() {
        // 700 of these rows encode to ~86 KiB. The length field is a u16, so
        // `as u16` wrapped and declared ~21 KiB: the reader consumed that much
        // and then read the middle of this payload as the next header, failed
        // the magic check, and dropped the connection.
        for n in [0, 1, 100, 500, 700, 2000] {
            let pkt = encode_packet(&Payload::Docker {
                host: "h".into(),
                rows: rows(n),
            });
            assert_eq!(
                declared_len(&pkt),
                pkt.len() - HEADER_LEN,
                "header and body disagree at {n} rows"
            );
            assert!(
                pkt.len() - HEADER_LEN <= MAX_PAYLOAD,
                "payload exceeds what the length field can express at {n} rows"
            );
        }
    }

    #[test]
    fn an_oversized_row_set_still_decodes_to_the_rows_that_fit() {
        // Truncating the packet would make it undecodable; dropping whole rows
        // keeps it valid, so a busy host still shows containers.
        let pkt = encode_packet(&Payload::Docker {
            host: "busy-host".into(),
            rows: rows(2000),
        });
        let decoded = decode_packet(&pkt).expect("an over-budget packet must still decode");
        match decoded {
            Payload::Docker { host, rows: got } => {
                assert_eq!(host, "busy-host");
                assert!(!got.is_empty(), "some rows must survive");
                assert!(got.len() < 2000, "not all rows can fit in 64 KiB");
                assert_eq!(got[0].name, "container-with-a-fairly-long-name-0");
            }
            other => panic!("wrong payload kind: {other:?}"),
        }
    }

    #[test]
    fn ordinary_packets_round_trip_unchanged() {
        let pkt = encode_packet(&Payload::Docker {
            host: "h".into(),
            rows: rows(3),
        });
        match decode_packet(&pkt).unwrap() {
            Payload::Docker { rows: got, .. } => assert_eq!(got.len(), 3),
            other => panic!("wrong payload kind: {other:?}"),
        }
    }
}
