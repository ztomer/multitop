//! Compact binary telemetry protocol (b"MTOP") for streaming metrics over SSH.

use crate::docker::Row as DockerRow;
use crate::fetch::FetchSnapshot;
use crate::proc::{Proc, Usage};
use crate::render::{Snapshot, TempUnit};

pub const MAGIC: &[u8; 4] = b"MTOP";
pub const PROTO_VERSION: u8 = 1;

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

pub fn encode_packet(payload: &Payload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(crate::consts::PACKET_CAPACITY);
    // Header: magic(4) + version(1) + mode(1) + payload_len(2)
    buf.extend_from_slice(MAGIC);
    buf.push(PROTO_VERSION);
    let mode = match payload {
        Payload::Monitor(_) => ProtoMode::Monitor,
        Payload::Docker { .. } => ProtoMode::Docker,
        Payload::Fetch(_) => ProtoMode::Fetch,
    };
    buf.push(mode.as_u8());
    buf.push(0); // payload_len placeholder byte 1
    buf.push(0); // payload_len placeholder byte 2

    let payload_start = buf.len();

    match payload {
        Payload::Monitor(snap) => encode_snapshot(snap, &mut buf),
        Payload::Docker { host, rows } => encode_docker(host, rows, &mut buf),
        Payload::Fetch(snap) => encode_fetch(snap, &mut buf),
    }

    // Truncate rather than lie. `as u16` wrapped here, so a payload over 64 KiB
    // wrote a header claiming `len % 65536` -- the reader consumed that many
    // bytes and then parsed the middle of this payload as the next packet's
    // header, failed the magic check, and tore the connection down. A host with
    // enough containers could never show its Docker view.
    //
    // Truncating loses the frame (the decoder cannot parse a cut-off payload and
    // returns None), but the stream stays framed, so the next packet is read
    // correctly. Losing one frame beats losing the connection. The encoders
    // below keep within budget so this is a backstop, not the normal path.
    let payload_len = buf.len() - payload_start;
    let payload_len = if payload_len > MAX_PAYLOAD {
        buf.truncate(payload_start + MAX_PAYLOAD);
        MAX_PAYLOAD
    } else {
        payload_len
    };
    #[allow(clippy::cast_possible_truncation)]
    let len_bytes = (payload_len as u16).to_le_bytes();
    buf[6] = len_bytes[0];
    buf[7] = len_bytes[1];

    buf
}

fn encode_str(s: &str, buf: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&bytes[..len as usize]);
}

fn encode_snapshot(snap: &Snapshot, buf: &mut Vec<u8>) {
    encode_str(&snap.host, buf);
    encode_str(&snap.agent_version, buf);
    buf.extend_from_slice(&(snap.cpu_pct as f32).to_le_bytes());

    #[allow(clippy::cast_possible_truncation)]
    let num_cores = snap.cores.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&num_cores.to_le_bytes());
    // `.take` so the declared count and the emitted items cannot disagree.
    for &(idx, cpu, temp) in snap.cores.iter().take(num_cores as usize) {
        buf.extend_from_slice(&(idx as u16).to_le_bytes());
        buf.extend_from_slice(&(cpu as f32).to_le_bytes());
        let t = temp.unwrap_or(-1.0) as f32;
        buf.extend_from_slice(&t.to_le_bytes());
    }

    let tu = match snap.temp_unit {
        TempUnit::C => 0u8,
        TempUnit::F => 1u8,
    };
    buf.push(tu);

    buf.extend_from_slice(&snap.mem.total.to_le_bytes());
    buf.extend_from_slice(&snap.mem.used.to_le_bytes());
    buf.extend_from_slice(&snap.disk.total.to_le_bytes());
    buf.extend_from_slice(&snap.disk.used.to_le_bytes());
    buf.extend_from_slice(&(snap.rx_rate as f32).to_le_bytes());
    buf.extend_from_slice(&(snap.tx_rate as f32).to_le_bytes());

    let num_procs = snap.procs.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&num_procs.to_le_bytes());
    for p in &snap.procs {
        buf.extend_from_slice(&p.pid.to_le_bytes());
        buf.extend_from_slice(&(p.cpu as f32).to_le_bytes());
        buf.extend_from_slice(&p.mem.to_le_bytes());
        encode_str(&p.name, buf);
    }
}

fn encode_docker(host: &str, rows: &[DockerRow], buf: &mut Vec<u8>) {
    encode_str(host, buf);
    // The count is written after the rows, because how many fit is not known
    // until they are encoded. Writing `rows.len()` up front and then emitting
    // every row was wrong twice over: past 65535 rows the count wrapped, and
    // past 64 KiB the packet could not describe its own length at all.
    let count_pos = buf.len();
    buf.extend_from_slice(&0u16.to_le_bytes());
    let mut written: u16 = 0;
    for r in rows {
        if written == u16::MAX {
            break;
        }
        let before = buf.len();
        encode_str(&r.name, buf);
        encode_str(&r.status, buf);
        buf.extend_from_slice(&(r.cpu_pct as f32).to_le_bytes());
        encode_str(&r.cpu, buf);
        encode_str(&r.mem, buf);
        buf.extend_from_slice(&r.mem_bytes.to_le_bytes());
        if buf.len() - HEADER_LEN > MAX_PAYLOAD {
            // This row does not fit; drop it whole rather than half.
            buf.truncate(before);
            break;
        }
        written += 1;
    }
    buf[count_pos..count_pos + 2].copy_from_slice(&written.to_le_bytes());
}

fn encode_fetch(snap: &FetchSnapshot, buf: &mut Vec<u8>) {
    encode_str(&snap.user_host, buf);
    encode_str(&snap.agent_version, buf);
    encode_str(&snap.os, buf);
    encode_str(&snap.kernel, buf);
    encode_str(&snap.uptime, buf);
    encode_str(&snap.host_model, buf);
    encode_str(&snap.cpu_model, buf);
    encode_str(&snap.memory_str, buf);
    encode_str(&snap.disk_str, buf);
}

pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.pos + n <= self.data.len() {
            let res = &self.data[self.pos..self.pos + n];
            self.pos += n;
            Some(res)
        } else {
            None
        }
    }

    fn read_u8(&mut self) -> Option<u8> {
        let b = self.read_bytes(1)?;
        Some(b[0])
    }

    fn read_u16(&mut self) -> Option<u16> {
        let b = self.read_bytes(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Option<u32> {
        let b = self.read_bytes(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&mut self) -> Option<u64> {
        let b = self.read_bytes(8)?;
        Some(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_f32(&mut self) -> Option<f32> {
        let b = self.read_bytes(4)?;
        Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_str(&mut self) -> Option<String> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_bytes(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub fn decode_packet(data: &[u8]) -> Option<Payload> {
    if data.len() < HEADER_LEN {
        return None;
    }
    if &data[..4] != MAGIC {
        return None;
    }
    let _version = data[4];
    let mode = data[5];
    let payload_len = u16::from_le_bytes([data[6], data[7]]) as usize;
    if data.len() < HEADER_LEN + payload_len {
        return None;
    }

    let mut cur = Cursor::new(&data[8..]);
    match mode {
        MODE_MONITOR => decode_snapshot(&mut cur).map(Payload::Monitor),
        MODE_DOCKER => decode_docker(&mut cur),
        MODE_FETCH => decode_fetch(&mut cur).map(Payload::Fetch),
        _ => None,
    }
}

fn decode_fetch(cur: &mut Cursor) -> Option<FetchSnapshot> {
    Some(FetchSnapshot {
        user_host: cur.read_str()?,
        agent_version: cur.read_str()?,
        os: cur.read_str()?,
        kernel: cur.read_str()?,
        uptime: cur.read_str()?,
        host_model: cur.read_str()?,
        cpu_model: cur.read_str()?,
        memory_str: cur.read_str()?,
        disk_str: cur.read_str()?,
    })
}

fn decode_snapshot(cur: &mut Cursor) -> Option<Snapshot> {
    let host = cur.read_str()?;
    let agent_version = cur.read_str()?;
    let cpu_pct = cur.read_f32()? as f64;

    let num_cores = cur.read_u16()? as usize;
    let rem_cores = cur.remaining() / 10;
    let mut cores = Vec::with_capacity(num_cores.min(rem_cores));
    for _ in 0..num_cores {
        let idx = cur.read_u16()? as usize;
        let cpu = cur.read_f32()? as f64;
        let temp_raw = cur.read_f32()?;
        let temp = if temp_raw < 0.0 {
            None
        } else {
            Some(temp_raw as f64)
        };
        cores.push((idx, cpu, temp));
    }

    let tu_u8 = cur.read_u8()?;
    let temp_unit = match tu_u8 {
        1 => TempUnit::F,
        _ => TempUnit::C,
    };

    let mem_total = cur.read_u64()?;
    let mem_used = cur.read_u64()?;
    let disk_total = cur.read_u64()?;
    let disk_used = cur.read_u64()?;
    let rx_rate = cur.read_f32()? as f64;
    let tx_rate = cur.read_f32()? as f64;

    let num_procs = cur.read_u16()? as usize;
    let rem_procs = cur.remaining() / 18;
    let mut procs = Vec::with_capacity(num_procs.min(rem_procs));
    for _ in 0..num_procs {
        let pid = cur.read_u32()?;
        let cpu = cur.read_f32()? as f64;
        let mem = cur.read_u64()?;
        let name = cur.read_str()?;
        procs.push(Proc {
            pid,
            name,
            cpu,
            mem,
        });
    }

    Some(Snapshot {
        host,
        agent_version,
        cpu_pct,
        cores,
        temp_unit,
        mem: Usage::new(mem_total, mem_used),
        disk: Usage::new(disk_total, disk_used),
        rx_rate,
        tx_rate,
        procs,
    })
}

fn decode_docker(cur: &mut Cursor) -> Option<Payload> {
    let host = cur.read_str()?;
    let num_rows = cur.read_u16()? as usize;
    let rem_rows = cur.remaining() / 22;
    let mut rows = Vec::with_capacity(num_rows.min(rem_rows));
    for _ in 0..num_rows {
        let name = cur.read_str()?;
        let status = cur.read_str()?;
        let cpu_pct = cur.read_f32()? as f64;
        let cpu = cur.read_str()?;
        let mem = cur.read_str()?;
        let mem_bytes = cur.read_u64()?;
        rows.push(DockerRow {
            name,
            status,
            cpu,
            cpu_pct,
            mem,
            mem_bytes,
        });
    }
    Some(Payload::Docker { host, rows })
}

#[cfg(test)]
mod framing_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn rows(n: usize) -> Vec<DockerRow> {
        (0..n)
            .map(|i| DockerRow {
                name: format!("container-with-a-fairly-long-name-{i}"),
                status: "Up 3 days (healthy) registry.example.com/team/svc".to_string(),
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
