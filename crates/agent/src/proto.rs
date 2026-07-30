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

pub fn encode_packet(payload: &Payload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
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

    let payload_len = (buf.len() - payload_start) as u16;
    let len_bytes = payload_len.to_le_bytes();
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

    let num_cores = snap.cores.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&num_cores.to_le_bytes());
    for &(idx, cpu, temp) in &snap.cores {
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
    let num_rows = rows.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&num_rows.to_le_bytes());
    for r in rows {
        encode_str(&r.name, buf);
        encode_str(&r.status, buf);
        buf.extend_from_slice(&(r.cpu_pct as f32).to_le_bytes());
        encode_str(&r.cpu, buf);
        encode_str(&r.mem, buf);
        buf.extend_from_slice(&r.mem_bytes.to_le_bytes());
    }
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
    if data.len() < 8 {
        return None;
    }
    if &data[..4] != MAGIC {
        return None;
    }
    let _version = data[4];
    let mode = data[5];
    let payload_len = u16::from_le_bytes([data[6], data[7]]) as usize;
    if data.len() < 8 + payload_len {
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
