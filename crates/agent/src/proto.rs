//! Compact binary telemetry protocol (b"MTOP") for streaming metrics over SSH.

use crate::docker::Row as DockerRow;
use crate::proc::{Proc, Usage};
use crate::render::{Snapshot, TempUnit};

pub const MAGIC: &[u8; 4] = b"MTOP";
pub const PROTO_VERSION: u8 = 1;

pub const MODE_MONITOR: u8 = 0;
pub const MODE_DOCKER: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    Monitor(Snapshot),
    Docker { host: String, rows: Vec<DockerRow> },
}

pub fn encode_packet(payload: &Payload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
    // Placeholder for 6-byte header: magic(4) + version(1) + mode(1) + payload_len(2)
    buf.extend_from_slice(MAGIC);
    buf.push(PROTO_VERSION);
    let mode = match payload {
        Payload::Monitor(_) => MODE_MONITOR,
        Payload::Docker { .. } => MODE_DOCKER,
    };
    buf.push(mode);
    buf.extend_from_slice(&[0u8; 2]); // payload_len placeholder

    let payload_start = buf.len();

    match payload {
        Payload::Monitor(snap) => encode_snapshot(snap, &mut buf),
        Payload::Docker { host, rows } => encode_docker(host, rows, &mut buf),
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
    let _payload_len = u16::from_le_bytes([data[6], data[7]]);

    let mut cur = Cursor::new(&data[8..]);
    match mode {
        MODE_MONITOR => decode_snapshot(&mut cur).map(Payload::Monitor),
        MODE_DOCKER => decode_docker(&mut cur),
        _ => None,
    }
}

fn decode_snapshot(cur: &mut Cursor) -> Option<Snapshot> {
    let host = cur.read_str()?;
    let cpu_pct = cur.read_f32()? as f64;

    let num_cores = cur.read_u16()? as usize;
    let rem_cores = cur.remaining() / 10;
    let mut cores = Vec::with_capacity(num_cores.min(rem_cores));
    for _ in 0..num_cores {
        let idx = cur.read_u16()? as usize;
        let cpu = cur.read_f32()? as f64;
        let temp_raw = cur.read_f32()?;
        let temp = if temp_raw < 0.0 { None } else { Some(temp_raw as f64) };
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
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trip() {
        let snap = Snapshot {
            host: "server1 (10.0.0.1)".to_string(),
            cpu_pct: 42.5,
            cores: vec![(0, 40.0, Some(55.0)), (1, 45.0, None)],
            temp_unit: TempUnit::C,
            mem: Usage::new(16_000_000_000, 8_000_000_000),
            disk: Usage::new(100_000_000_000, 50_000_000_000),
            rx_rate: 10240.0,
            tx_rate: 20480.0,
            procs: vec![Proc {
                pid: 1234,
                name: "node".to_string(),
                cpu: 15.2,
                mem: 100_000_000,
            }],
        };

        let payload = Payload::Monitor(snap.clone());
        let encoded = encode_packet(&payload);
        assert_eq!(&encoded[..4], MAGIC);
        let decoded = decode_packet(&encoded).expect("decode successfully");

        if let Payload::Monitor(d_snap) = decoded {
            assert_eq!(d_snap.host, snap.host);
            assert!((d_snap.cpu_pct - snap.cpu_pct).abs() < 0.01);
            assert_eq!(d_snap.cores.len(), snap.cores.len());
            assert_eq!(d_snap.cores[0].0, snap.cores[0].0);
            assert!((d_snap.cores[0].1 - snap.cores[0].1).abs() < 0.01);
            assert_eq!(d_snap.cores[0].2, snap.cores[0].2);
            assert_eq!(d_snap.cores[1].2, None);
            assert_eq!(d_snap.mem, snap.mem);
            assert_eq!(d_snap.disk, snap.disk);
            assert_eq!(d_snap.procs.len(), snap.procs.len());
            assert_eq!(d_snap.procs[0].pid, snap.procs[0].pid);
            assert_eq!(d_snap.procs[0].name, snap.procs[0].name);
            assert_eq!(d_snap.procs[0].mem, snap.procs[0].mem);
            assert!((d_snap.procs[0].cpu - snap.procs[0].cpu).abs() < 0.01);
        } else {
            panic!("expected Monitor payload");
        }
    }

    #[test]
    fn docker_round_trip() {
        let row = DockerRow {
            name: "web_nginx_1".to_string(),
            status: "Up 3 days".to_string(),
            cpu: "1.25%".to_string(),
            cpu_pct: 1.25,
            mem: "15.4MiB / 2.0GiB".to_string(),
            mem_bytes: 16_148_070,
        };
        let payload = Payload::Docker {
            host: "docker-host-01".to_string(),
            rows: vec![row.clone()],
        };

        let encoded = encode_packet(&payload);
        assert_eq!(&encoded[..4], MAGIC);
        let decoded = decode_packet(&encoded).expect("decode docker packet");

        if let Payload::Docker { host, rows } = decoded {
            assert_eq!(host, "docker-host-01");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].name, row.name);
            assert_eq!(rows[0].status, row.status);
            assert_eq!(rows[0].cpu, row.cpu);
            assert!((rows[0].cpu_pct - row.cpu_pct).abs() < 0.01);
            assert_eq!(rows[0].mem, row.mem);
            assert_eq!(rows[0].mem_bytes, row.mem_bytes);
        } else {
            panic!("expected Docker payload");
        }
    }

    #[test]
    fn decode_fuzz_garbage_never_panics() {
        let garbage_inputs: Vec<&[u8]> = vec![
            b"",
            b"MTO",
            b"MTOP",
            b"MTOP\x01\x00\x00\x00",
            b"MTOP\x01\x00\x05\x00\x01\x02\x03", // length says 5, only 3 bytes
            b"MTOP\x01\x00\xff\xff12345",      // length says 65535, buffer tiny
            b"MTOP\x01\x02\x00\x00",            // unknown mode 2
            b"MTOP\x01\x00\x10\x00\xff\xff\x00\x00\x00\x00", // corrupted string length
            &[0u8; 100],
            &[0xff; 100],
        ];

        for input in garbage_inputs {
            let res = std::panic::catch_unwind(|| decode_packet(input));
            assert!(res.is_ok(), "decode_packet panicked on input: {:?}", input);
            assert!(decode_packet(input).is_none());
        }
    }

    #[test]
    fn boundary_values_and_unicode_handling() {
        let snap = Snapshot {
            host: "🚀-prod-node-üñîçødê".to_string(),
            cpu_pct: 99.99,
            cores: (0..128).map(|i| (i, 100.0, Some(85.5))).collect(),
            temp_unit: TempUnit::F,
            mem: Usage::new(u64::MAX, u64::MAX / 2),
            disk: Usage::new(u64::MAX, u64::MAX / 4),
            rx_rate: f64::MAX,
            tx_rate: 0.0,
            procs: vec![Proc {
                pid: u32::MAX,
                name: "🔥-worker-process".to_string(),
                cpu: 999.9,
                mem: u64::MAX,
            }],
        };

        let payload = Payload::Monitor(snap.clone());
        let encoded = encode_packet(&payload);
        let decoded = decode_packet(&encoded).expect("decode boundary snapshot");

        if let Payload::Monitor(d_snap) = decoded {
            assert_eq!(d_snap.host, snap.host);
            assert_eq!(d_snap.cores.len(), 128);
            assert_eq!(d_snap.temp_unit, TempUnit::F);
            assert_eq!(d_snap.mem, snap.mem);
            assert_eq!(d_snap.disk, snap.disk);
            assert_eq!(d_snap.procs[0].pid, u32::MAX);
            assert_eq!(d_snap.procs[0].name, "🔥-worker-process");
        } else {
            panic!("expected Monitor payload");
        }
    }
}
