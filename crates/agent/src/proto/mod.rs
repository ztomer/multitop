//! Compact binary telemetry protocol (b"MTOP") for streaming metrics over SSH.

use crate::docker::Row as DockerRow;
use crate::exec::ExecFrame;
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
///
/// 5 added the Exec mode, which moved the upgrade off raw text over `ssh -tt`
/// and onto this framing. It is the only mode that travels in both directions.
pub const PROTO_VERSION: u8 = 5;

/// The version that introduced [`ProtoMode::Exec`]. An Exec frame from an older
/// agent is refused rather than misread: read with the wrong layout it would
/// come out one field along, and here that nonsense is an exit code -- the one
/// number that decides whether an operator is told their upgrade worked.
pub const EXEC_MIN_VERSION: u8 = 5;

/// Lowest proto version this build can speak. Hello negotiation uses this
/// to find an overlap; outside the range the session is replaced.
pub const PROTO_MIN_VERSION: u8 = 2;

/// Longest `agent_version` Hello will accept. Sane semver is <20 chars;
/// 64 leaves room for pre-release while bounding what a malicious hello can make
/// the receiver allocate.
pub const MAX_AGENT_VERSION_LEN: usize = 64;

/// Highest proto version Hello will accept. Current wire is 5; 20 leaves headroom
/// without letting a garbage 255 negotiate.
pub const MAX_PROTO_VERSION: u8 = 20;

/// Sentinel when no agent was embedded. Must never negotiate as valid.
pub const MISSING_VERSION: &str = "missing";

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoMode {
    Monitor = 0,
    Docker = 1,
    Fetch = 2,
    Exec = 3,
    Hello = 4,
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
            3 => Ok(ProtoMode::Exec),
            4 => Ok(ProtoMode::Hello),
            other => Err(other),
        }
    }
}

pub const MODE_MONITOR: u8 = ProtoMode::Monitor.as_u8();
pub const MODE_DOCKER: u8 = ProtoMode::Docker.as_u8();
pub const MODE_FETCH: u8 = ProtoMode::Fetch.as_u8();
pub const MODE_EXEC: u8 = ProtoMode::Exec.as_u8();
pub const MODE_HELLO: u8 = ProtoMode::Hello.as_u8();

#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    pub agent_version: String,
    pub proto_version: u8,
    pub min_proto_version: u8,
}

impl Hello {
    #[must_use]
    pub fn new(agent_version: String) -> Self {
        const { assert!(PROTO_MIN_VERSION <= PROTO_VERSION) }
        Self {
            agent_version,
            proto_version: PROTO_VERSION,
            min_proto_version: PROTO_MIN_VERSION,
        }
    }

    /// Validate wire invariants. Foolproof means a truncated or malicious
    /// Hello never negotiates.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.agent_version.is_empty() || self.agent_version.len() > MAX_AGENT_VERSION_LEN {
            return false;
        }
        if self.agent_version == MISSING_VERSION {
            return false;
        }
        if self.proto_version == 0 || self.min_proto_version == 0 {
            return false;
        }
        if self.min_proto_version > self.proto_version {
            return false;
        }
        if self.proto_version > MAX_PROTO_VERSION || self.min_proto_version > MAX_PROTO_VERSION {
            return false;
        }
        // agent_version should be dotted numeric, but allow at least one dot
        // to reject garbage while still permitting future pre-release tags.
        if !self
            .agent_version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
        {
            return false;
        }
        true
    }

    /// Whether this hello overlaps with the local build's range.
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        if !self.is_valid() {
            return false;
        }
        self.proto_version >= PROTO_MIN_VERSION && PROTO_VERSION >= self.min_proto_version
    }

    /// Parse "0.43.0" as (major, minor, patch). Returns None for garbage.
    fn parse_version(v: &str) -> Option<(u16, u16, u16)> {
        let mut parts = v.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch_str = parts.next()?;
        // strip pre-release/build metadata
        let patch = patch_str.split(['-', '+']).next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some((major, minor, patch))
    }

    /// Whether the agent binary should be replaced. Foolproof: never downgrade
    /// a newer remote, and never negotiate with an invalid Hello.
    #[must_use]
    pub fn needs_replacement(&self, local_version: &str) -> bool {
        if !self.is_valid() || !self.is_compatible() {
            return true;
        }
        if self.agent_version == local_version {
            return false;
        }
        // If versions parse, only replace when remote is older.
        if let (Some(remote), Some(local)) = (
            Self::parse_version(&self.agent_version),
            Self::parse_version(local_version),
        ) {
            return remote < local;
        }
        // Unparseable but not equal → replace to converge
        true
    }

    /// Human-readable reason for replacement.
    #[must_use]
    pub fn mismatch_reason(&self, local_version: &str) -> String {
        if !self.is_valid() {
            return format!(
                "invalid Hello (agent_version={:?} proto={} min={})",
                self.agent_version, self.proto_version, self.min_proto_version
            );
        }
        if !self.is_compatible() {
            return format!(
                "incompatible proto remote {} (min {}) vs local {} (min {})",
                self.proto_version, self.min_proto_version, PROTO_VERSION, PROTO_MIN_VERSION
            );
        }
        if self.agent_version != local_version {
            if let (Some(remote), Some(local)) = (
                Self::parse_version(&self.agent_version),
                Self::parse_version(local_version),
            ) {
                if remote > local {
                    return format!(
                        "remote {} newer than local {} — update local",
                        self.agent_version, local_version
                    );
                }
            }
            return format!("remote {} vs local {}", self.agent_version, local_version);
        }
        String::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    Monitor(Snapshot),
    Docker { host: String, rows: Vec<DockerRow> },
    Fetch(FetchSnapshot),
    Exec(ExecFrame),
    Hello(Hello),
}

/// Bytes of fixed header before the payload: magic(4) + version(1) + mode(1) + len(2).
///
/// Public because a reader on the other end of a pipe has to know how many
/// bytes to `read_exact` before it can learn the payload's length.
pub const HEADER_LEN: usize = 8;
/// The payload length field is a u16, so this is the hard ceiling.
const MAX_PAYLOAD: usize = u16::MAX as usize;

mod decode;
mod encode;
mod exec_codec;

pub use decode::{decode_packet, Cursor};
pub use encode::encode_packet;
pub use exec_codec::{decode_exec, encode_exec};

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

    #[test]
    fn hello_round_trips_and_validates() {
        let hello = Hello::new("0.43.0".into());
        assert!(hello.is_valid());
        assert!(hello.is_compatible());
        assert!(!hello.needs_replacement("0.43.0"));
        let pkt = encode_packet(&Payload::Hello(hello.clone()));
        match decode_packet(&pkt).unwrap() {
            Payload::Hello(got) => assert_eq!(got, hello),
            other => panic!("wrong payload kind: {other:?}"),
        }
    }

    #[test]
    fn hello_foolproof_rejects_garbage() {
        // empty, missing, proto 0, min>proto, overly long
        for bad in [
            Hello {
                agent_version: "".into(),
                proto_version: 5,
                min_proto_version: 2,
            },
            Hello {
                agent_version: "missing".into(),
                proto_version: 5,
                min_proto_version: 2,
            },
            Hello {
                agent_version: "0.43.0".into(),
                proto_version: 0,
                min_proto_version: 0,
            },
            Hello {
                agent_version: "0.43.0".into(),
                proto_version: 2,
                min_proto_version: 5,
            },
            Hello {
                agent_version: "a".repeat(65),
                proto_version: 5,
                min_proto_version: 2,
            },
            Hello {
                agent_version: "bad/../../".into(),
                proto_version: 5,
                min_proto_version: 2,
            },
        ] {
            assert!(!bad.is_valid(), "should be invalid: {bad:?}");
            assert!(!bad.is_compatible());
            assert!(bad.needs_replacement("0.43.0"));
        }
    }

    #[test]
    fn hello_never_downgrades_newer_remote() {
        let hello = Hello {
            agent_version: "0.44.0".into(),
            proto_version: 5,
            min_proto_version: 2,
        };
        assert!(hello.is_valid());
        assert!(hello.is_compatible());
        // local 0.43.0 is older, should NOT replace newer remote
        assert!(!hello.needs_replacement("0.44.0"));
        assert!(
            !hello.needs_replacement("0.43.0"),
            "should not downgrade 0.44 onto 0.43"
        );
        assert!(hello.mismatch_reason("0.43.0").contains("newer than local"));
    }

    #[test]
    fn hello_proto_incompatible_needs_replacement() {
        let hello = Hello {
            agent_version: "0.43.0".into(),
            proto_version: 99,
            min_proto_version: 99,
        };
        assert!(!hello.is_compatible());
        assert!(hello.needs_replacement("0.43.0"));
    }
}
