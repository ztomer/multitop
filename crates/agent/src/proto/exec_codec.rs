//! The exec payload, both directions in one file.
//!
//! `encode.rs` and `decode.rs` are split by direction because every other
//! payload only ever travels one way. The exec channel does not:
//! [`ExecFrame::Request`] goes client → agent and the rest come back. Splitting
//! it across the two files would put the two halves of one frame's layout in
//! two places, which is precisely the failure `encode.rs` opens with a warning
//! about -- a field added on one side without its counterpart on the other does
//! not fail, it shifts every field after it into the wrong slot.

use super::decode::Cursor;
use crate::exec::{
    ExecFrame, MarkerKind, Stream, KIND_ALIVE, KIND_BEGIN, KIND_EXIT, KIND_MARKER, KIND_OUT,
    KIND_REQUEST, MAX_EXEC_CHUNK,
};

/// `None` for an absent optional, `Some` for a present one. A byte rather than
/// an empty string, because an empty password is a password and a missing one
/// is not.
const ABSENT: u8 = 0;
const PRESENT: u8 = 1;

fn put_str(s: &str, buf: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(len as u16).to_le_bytes());
    buf.extend_from_slice(&bytes[..len]);
}

fn put_blob(b: &[u8], buf: &mut Vec<u8>) {
    let len = b.len().min(u16::MAX as usize);
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(len as u16).to_le_bytes());
    buf.extend_from_slice(&b[..len]);
}

/// Serialise one frame's body. The header is written by `encode_packet`.
///
/// # Panics
///
/// Never. An oversized `Out` is refused by the caller rather than truncated --
/// see [`encode_exec`].
pub fn encode_exec(frame: &ExecFrame, buf: &mut Vec<u8>) {
    buf.push(frame.kind());
    match frame {
        ExecFrame::Request {
            command,
            password,
            use_lock,
            cols,
            rows,
        } => {
            put_str(command, buf);
            match password {
                Some(p) => {
                    buf.push(PRESENT);
                    put_str(p, buf);
                }
                None => buf.push(ABSENT),
            }
            buf.push(u8::from(*use_lock));
            buf.extend_from_slice(&cols.to_le_bytes());
            buf.extend_from_slice(&rows.to_le_bytes());
        }
        ExecFrame::Begin {
            host,
            agent_version,
            pid,
        } => {
            put_str(host, buf);
            put_str(agent_version, buf);
            buf.extend_from_slice(&pid.to_le_bytes());
        }
        ExecFrame::Out { stream, seq, bytes } => {
            buf.push(stream.as_u8());
            buf.extend_from_slice(&seq.to_le_bytes());
            // Clamped, not truncated silently: a chunk past the ceiling is a
            // caller that skipped `exec::chunks`, and losing an operator's
            // output to a quiet `min()` is the thing this channel exists to
            // stop. The debug assertion turns it into a test failure; release
            // keeps the packet well-formed rather than corrupting the stream.
            debug_assert!(
                bytes.len() <= MAX_EXEC_CHUNK,
                "Out chunk of {} bytes exceeds MAX_EXEC_CHUNK; use exec::chunks",
                bytes.len()
            );
            put_blob(&bytes[..bytes.len().min(MAX_EXEC_CHUNK)], buf);
        }
        ExecFrame::Marker(kind) => buf.push(kind.as_u8()),
        ExecFrame::Alive { elapsed_ms } => buf.extend_from_slice(&elapsed_ms.to_le_bytes()),
        ExecFrame::Exit { code, signalled } => {
            buf.extend_from_slice(&code.to_le_bytes());
            buf.push(u8::from(*signalled));
        }
    }
}

/// Parse one frame's body. `None` for anything this build cannot read.
///
/// An unknown `kind` is `None` rather than a skip: the frames are not
/// self-delimiting within the payload, so a reader that does not know a kind
/// does not know its length either and cannot honestly carry on.
pub fn decode_exec(cur: &mut Cursor) -> Option<ExecFrame> {
    match cur.read_u8()? {
        KIND_REQUEST => {
            let command = cur.read_str()?;
            let password = match cur.read_u8()? {
                PRESENT => Some(cur.read_str()?),
                ABSENT => None,
                _ => return None,
            };
            Some(ExecFrame::Request {
                command,
                password,
                use_lock: cur.read_u8()? != 0,
                cols: cur.read_u16()?,
                rows: cur.read_u16()?,
            })
        }
        KIND_BEGIN => Some(ExecFrame::Begin {
            host: cur.read_str()?,
            agent_version: cur.read_str()?,
            pid: cur.read_u32()?,
        }),
        KIND_OUT => {
            let stream = Stream::from_u8(cur.read_u8()?)?;
            let seq = cur.read_u32()?;
            Some(ExecFrame::Out {
                stream,
                seq,
                bytes: cur.read_blob()?,
            })
        }
        KIND_MARKER => MarkerKind::from_u8(cur.read_u8()?).map(ExecFrame::Marker),
        KIND_ALIVE => Some(ExecFrame::Alive {
            elapsed_ms: cur.read_u32()?,
        }),
        KIND_EXIT => Some(ExecFrame::Exit {
            code: cur.read_i32()?,
            signalled: cur.read_u8()? != 0,
        }),
        _ => None,
    }
}
