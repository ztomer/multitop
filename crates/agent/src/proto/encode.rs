//! Turning a payload into bytes.
//!
//! The mirror of `decode.rs`, and the two have to be read together: a field
//! added here without its counterpart there does not fail, it shifts every
//! field after it into the wrong slot. That is why `PROTO_VERSION` exists and
//! why `decode` checks it.
//!
//! `Cursor` lives in `decode.rs`: it only ever reads.

use super::{ProtoMode, HEADER_LEN, MAGIC, MAX_PAYLOAD, PROTO_VERSION};
use crate::docker::Row as DockerRow;
use crate::fetch::FetchSnapshot;
use crate::proto::Payload;
use crate::render::{Snapshot, TempUnit};

pub fn encode_packet(payload: &Payload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(crate::consts::PACKET_CAPACITY);
    // Header: magic(4) + version(1) + mode(1) + payload_len(2)
    buf.extend_from_slice(MAGIC);
    buf.push(PROTO_VERSION);
    let mode = match payload {
        Payload::Monitor(_) => ProtoMode::Monitor,
        Payload::Docker { .. } => ProtoMode::Docker,
        Payload::Fetch(_) => ProtoMode::Fetch,
        Payload::Exec(_) => ProtoMode::Exec,
    };
    buf.push(mode.as_u8());
    buf.push(0); // payload_len placeholder byte 1
    buf.push(0); // payload_len placeholder byte 2

    let payload_start = buf.len();

    match payload {
        Payload::Monitor(snap) => encode_snapshot(snap, &mut buf),
        Payload::Docker { host, rows } => encode_docker(host, rows, &mut buf),
        Payload::Fetch(snap) => encode_fetch(snap, &mut buf),
        Payload::Exec(frame) => super::exec_codec::encode_exec(frame, &mut buf),
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
    // Same sentinel the per-core temperature uses a few lines down: a negative
    // is "not measured", because there is no such thing as a negative clock.
    buf.extend_from_slice(&(snap.cpu_mhz.unwrap_or(-1.0) as f32).to_le_bytes());

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

    encode_proc_names(&snap.proc_names, buf);
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
        encode_str(&r.image, buf);
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

/// The filter's name list, written last.
///
/// Last on purpose: everything before it is what protocol 3 already sent, so a
/// reader that stops here has still read a whole, correct snapshot. That is
/// what lets `decode_snapshot` gate this field on the version instead of
/// refusing the packet, and refusing Monitor packets is the one thing it must
/// never do -- the mismatch that replaces a stale agent is read out of one.
fn encode_proc_names(names: &[String], buf: &mut Vec<u8>) {
    let count_pos = buf.len();
    buf.extend_from_slice(&0u16.to_le_bytes());
    let mut written: u16 = 0;
    for name in names {
        if written == u16::MAX {
            break;
        }
        let before = buf.len();
        encode_str(name, buf);
        if buf.len() - HEADER_LEN > MAX_PAYLOAD {
            // Drop it whole rather than half, the same rule the Docker rows
            // follow. A truncated string here would desynchronise every field
            // after it -- there are none today, and there must be none for
            // this to stay true.
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
