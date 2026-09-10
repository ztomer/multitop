//! One run, start to finish, reported as frames.
//!
//! The contract this module exists to keep: **an [`ExecFrame::Exit`] is written
//! on every path out of [`run`], including the ones that fail before the child
//! starts.** The old client-side reader had two `return`s that skipped its
//! equivalent, and the cost is recorded at the top of `tasks/upgrade.rs`: the
//! panel stays in `STARTED` for the rest of the session, `upgrades_in_flight()`
//! never clears, quitting needs a confirmation for a run that ended long ago,
//! and no further upgrade can be started on any host. A run that cannot say it
//! finished is worse than a run that fails.
//!
//! So `run` has exactly one exit, and the terminal frame is written there.

use std::ffi::CString;
use std::io::{Read, Write};
use std::time::Duration;

use super::script::{shell_argv, wrap};
use super::{
    chunks, lock, pty, ExecFrame, MarkerKind, Stream, LOCK_HELD_CODE, NO_SHELL_CODE,
    SUDO_FAILED_CODE,
};
use crate::proto::{decode_packet, encode_packet, Payload, HEADER_LEN};

/// What to run, already parsed off the wire.
pub struct Request<'a> {
    pub command: &'a str,
    pub password: Option<&'a str>,
    pub use_lock: bool,
    pub cols: u16,
    pub rows: u16,
    pub host: &'a str,
    /// Where the lock lives. Threaded rather than resolved inside, so a test
    /// can contend two runs against each other without either touching the
    /// operator's real lock -- and so two tests running at once cannot block
    /// one another on it. `None` means [`lock::default_path`].
    pub lock_path: Option<&'a std::path::Path>,
}

/// Write one frame. Errors are dropped: the only failure is the client having
/// gone, and there is nowhere left to report that to.
pub(crate) fn send<W: Write>(out: &mut W, frame: &ExecFrame) {
    let pkt = encode_packet(&Payload::Exec(frame.clone()));
    let _ = out.write_all(&pkt);
    let _ = out.flush();
}

/// Send raw output, split so no frame can exceed what its length field can
/// describe.
pub(crate) fn send_out<W: Write>(out: &mut W, stream: Stream, seq: &mut u32, bytes: &[u8]) {
    for chunk in chunks(bytes) {
        send(
            out,
            &ExecFrame::Out {
                stream,
                seq: *seq,
                bytes: chunk.to_vec(),
            },
        );
        *seq = seq.wrapping_add(1);
    }
}

/// Read one framed request off `input`.
///
/// `read_exact` for both the header and the body, never `read`: a pipe may hand
/// back fewer bytes than were asked for, and this project has already shipped
/// the defect where a magic header split across two reads was compared four
/// bytes at a time against a buffer holding one. Every packet after it was then
/// read from the wrong offset.
pub fn read_request<R: Read>(input: &mut R) -> Option<ExecFrame> {
    let mut header = [0u8; HEADER_LEN];
    input.read_exact(&mut header).ok()?;
    let len = u16::from_le_bytes([header[HEADER_LEN - 2], header[HEADER_LEN - 1]]) as usize;
    let mut body = vec![0u8; len];
    input.read_exact(&mut body).ok()?;
    let mut packet = header.to_vec();
    packet.append(&mut body);
    match decode_packet(&packet)? {
        Payload::Exec(frame @ ExecFrame::Request { .. }) => Some(frame),
        // A well-formed packet that is not a request is a client sending the
        // wrong thing, which is a different fault from a truncated stream and
        // must not be reported as one.
        _ => None,
    }
}

/// Report a failure that happened before a child could be started, and end the
/// run properly.
///
/// Public because the one caller is the CLI entry point, which cannot reach
/// [`run`] -- there is no request to run. It still owes the client an `Exit`.
pub fn emit_failure<W: Write>(out: &mut W, seq: &mut u32, stream: Stream, why: &str) {
    send_out(out, stream, seq, format!("{why}\n").as_bytes());
    send(
        out,
        &ExecFrame::Exit {
            code: 1,
            signalled: false,
        },
    );
}

/// Run the request, reporting to `out`.
///
/// Returns the exit code it reported, for the caller's own process status.
pub fn run<W: Write>(req: &Request, out: &mut W) -> i32 {
    let mut seq: u32 = 0;
    let outcome = execute(req, out, &mut seq);
    send(
        out,
        &ExecFrame::Exit {
            code: outcome.code,
            signalled: outcome.signalled,
        },
    );
    outcome.code
}

/// Everything between the request and the exit frame.
///
/// Split from [`run`] so that every `return` here is still followed by the
/// terminal frame. The obligation is structural rather than remembered.
fn execute<W: Write>(req: &Request, out: &mut W, seq: &mut u32) -> pty::Outcome {
    let default_lock;
    let lock_path = if let Some(p) = req.lock_path {
        p
    } else {
        default_lock = lock::default_path();
        &default_lock
    };
    let _guard = if req.use_lock {
        match lock::acquire(lock_path) {
            Ok(g) => Some(g),
            Err(lock::Denied::Held) => {
                send(out, &ExecFrame::Marker(MarkerKind::LockHeld));
                return pty::Outcome {
                    code: LOCK_HELD_CODE,
                    signalled: false,
                };
            }
            Err(lock::Denied::Failed(why)) => {
                // Not contention. Saying "another upgrade is running" about a
                // read-only home sends the operator hunting a process that
                // does not exist.
                send_out(
                    out,
                    Stream::Stderr,
                    seq,
                    format!("could not take the upgrade lock: {why}\n").as_bytes(),
                );
                return pty::Outcome {
                    code: 1,
                    signalled: false,
                };
            }
        }
    } else {
        None
    };

    let script = wrap(req.command, req.password.is_some());
    let Some(argv) = shell_argv(&script) else {
        send_out(
            out,
            Stream::Stderr,
            seq,
            b"the command contains a NUL byte and cannot be run\n",
        );
        return pty::Outcome {
            code: 1,
            signalled: false,
        };
    };

    let mut child = match spawn_with_retry(&argv, req.cols, req.rows) {
        Ok(c) => c,
        Err(e) => {
            send_out(
                out,
                Stream::Stderr,
                seq,
                format!("could not start a shell: {e}\n").as_bytes(),
            );
            return pty::Outcome {
                code: NO_SHELL_CODE,
                signalled: false,
            };
        }
    };

    send(
        out,
        &ExecFrame::Begin {
            host: req.host.to_string(),
            agent_version: crate::consts::AGENT_VERSION.to_string(),
            #[allow(clippy::cast_sign_loss)]
            pid: child.pid as u32,
        },
    );

    let (reaped, sudo_rejected) = super::pump::pump(req, out, seq, &mut child);
    child.close();
    let mut outcome = reaped.unwrap_or_else(|| pty::wait(child.pid));
    // The marker is the authority when it fired: a shell can lose an exit
    // status through a login profile, and reporting a refused password as
    // "exited 1" is what sent operators to read a correct upgrade script.
    if sudo_rejected && outcome.code != SUDO_FAILED_CODE {
        outcome.code = SUDO_FAILED_CODE;
        outcome.signalled = false;
    }
    outcome
}

/// Start the child, retrying a failure that may be transient.
///
/// A pty is a finite resource: a host already running many of them can refuse
/// one for a moment and grant it a moment later. Reporting that as "could not
/// start a shell" makes an operator go looking for a broken shell, and asking
/// them to press `u` again is asking them to be the retry loop.
///
/// Bounded, and it does not retry a failure that will not change -- there is no
/// point asking twice for a shell that does not exist.
fn spawn_with_retry(argv: &[CString], cols: u16, rows: u16) -> std::io::Result<pty::Child> {
    const ATTEMPTS: usize = 3;
    const PAUSE: Duration = Duration::from_millis(50);
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match pty::spawn(argv, cols, rows) {
            Ok(c) => return Ok(c),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::PermissionDenied
                {
                    return Err(e);
                }
                last = Some(e);
                if attempt + 1 < ATTEMPTS {
                    std::thread::sleep(PAUSE);
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| {
        std::io::Error::other("could not start a shell, and no reason was reported")
    }))
}
