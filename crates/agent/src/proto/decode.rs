//! Turning bytes back into a payload.
//!
//! Every read is fallible and every failure is `None`: a packet this build
//! cannot read is reported as exactly that by the caller, never as a closed
//! connection, because a host that is up and talking must not be described as
//! unreachable.

use super::{
    EXEC_MIN_VERSION, HEADER_LEN, MAGIC, MODE_DOCKER, MODE_EXEC, MODE_FETCH, MODE_MONITOR,
};
use crate::docker::Row as DockerRow;
use crate::fetch::FetchSnapshot;
use crate::proc::{Proc, Usage};
use crate::proto::Payload;
use crate::render::{Snapshot, TempUnit};

pub fn decode_packet(data: &[u8]) -> Option<Payload> {
    if data.len() < HEADER_LEN {
        return None;
    }
    if &data[..4] != MAGIC {
        return None;
    }
    let version = data[4];
    let mode = data[5];
    let payload_len = u16::from_le_bytes([data[6], data[7]]) as usize;
    if data.len() < HEADER_LEN + payload_len {
        return None;
    }

    let mut cur = Cursor::new(&data[8..]);
    match mode {
        MODE_MONITOR => decode_snapshot(&mut cur, version).map(Payload::Monitor),
        // The Docker row layout gained a field in protocol 2. Read with the
        // wrong layout the rows do not fail -- they come out as plausible
        // nonsense, one field shifted along -- so an older agent's are refused.
        // The caller says so and ends the session, which is what makes the
        // reconnect re-run the version check.
        //
        // Monitor and Fetch are deliberately still decoded at *any* version.
        // The agent-version mismatch that replaces the remote binary is read
        // out of a decoded Monitor packet, so refusing those on version alone
        // would leave a stale agent undetectable and therefore unreplaceable --
        // a wedge with no way out but editing the remote host by hand.
        MODE_DOCKER if version < 2 => None,
        MODE_DOCKER => decode_docker(&mut cur),
        MODE_FETCH => decode_fetch(&mut cur).map(Payload::Fetch),
        // Refused below the version that introduced it, for the reason Docker
        // is: an Exec frame read with an older layout does not fail, it comes
        // out as plausible nonsense one field along -- and here that nonsense
        // would be an exit code, which decides whether the operator is told the
        // upgrade worked.
        MODE_EXEC if version < EXEC_MIN_VERSION => None,
        MODE_EXEC => super::exec_codec::decode_exec(&mut cur).map(Payload::Exec),
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

/// `version` is the byte from the packet header.
///
/// The Monitor payload has to stay decodable at *every* version -- the
/// agent-version mismatch that replaces a stale remote binary is read out of a
/// decoded Monitor packet, so a snapshot this build refuses is a stale agent it
/// can never notice and never replace. A field added to this payload is
/// therefore read only when the sender's version says it is there, rather than
/// the payload being refused wholesale the way a Docker packet is.
fn decode_snapshot(cur: &mut Cursor, version: u8) -> Option<Snapshot> {
    let host = cur.read_str()?;
    let agent_version = cur.read_str()?;
    let cpu_pct = cur.read_f32()? as f64;
    let cpu_mhz = if version >= 3 {
        let raw = cur.read_f32()?;
        (raw > 0.0).then_some(f64::from(raw))
    } else {
        None
    };

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

    // Version-gated for the same reason `cpu_mhz` is, and safe for the same
    // reason: it is the last thing in the payload, so everything before it has
    // already been read into the right fields whatever the sender's version.
    let proc_names = if version >= 4 {
        let count = cur.read_u16()? as usize;
        let mut names = Vec::with_capacity(count.min(cur.remaining() / 2));
        for _ in 0..count {
            names.push(cur.read_str()?);
        }
        names
    } else {
        Vec::new()
    };

    Some(Snapshot {
        host,
        agent_version,
        cpu_pct,
        cpu_mhz,
        proc_names,
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
        let image = cur.read_str()?;
        let cpu_pct = cur.read_f32()? as f64;
        let cpu = cur.read_str()?;
        let mem = cur.read_str()?;
        let mem_bytes = cur.read_u64()?;
        rows.push(DockerRow {
            name,
            status,
            image,
            cpu,
            cpu_pct,
            mem,
            mem_bytes,
        });
    }
    Some(Payload::Docker { host, rows })
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

    pub(crate) fn read_u8(&mut self) -> Option<u8> {
        let b = self.read_bytes(1)?;
        Some(b[0])
    }

    pub(crate) fn read_u16(&mut self) -> Option<u16> {
        let b = self.read_bytes(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn read_u32(&mut self) -> Option<u32> {
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

    /// A signed count, for an exit status. Separate from `read_u32` because
    /// reinterpreting one as the other is silent and turns 255 into -1.
    pub(crate) fn read_i32(&mut self) -> Option<i32> {
        let b = self.read_bytes(4)?;
        Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Length-prefixed raw bytes.
    ///
    /// Distinct from `read_str`: terminal output is not guaranteed to be UTF-8
    /// and `from_utf8_lossy` would rewrite it. What the child wrote is what the
    /// operator has to be shown.
    pub(crate) fn read_blob(&mut self) -> Option<Vec<u8>> {
        let len = self.read_u16()? as usize;
        Some(self.read_bytes(len)?.to_vec())
    }

    pub(crate) fn read_str(&mut self) -> Option<String> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_bytes(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}
