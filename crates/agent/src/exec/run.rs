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
use std::time::{Duration, Instant};

use super::script::{shell_argv, wrap};
use super::sieve::{Piece, Sieve};
use super::{
    chunks, lock, pty, ExecFrame, MarkerKind, Stream, LOCK_HELD_CODE, NO_SHELL_CODE,
    SUDO_FAILED_CODE,
};
use crate::proto::{decode_packet, encode_packet, Payload, HEADER_LEN};

/// How long a poll waits before looking at the clock. Also the worst case
/// latency of a heartbeat, and of noticing a child that has exited without
/// closing its pty.
const POLL_MS: i32 = 100;
/// How often a heartbeat goes out while the child is alive.
const ALIVE_EVERY: Duration = Duration::from_secs(1);
/// One read from either descriptor.
const READ_BUF: usize = 8192;
/// How much output may be held back waiting for the shell to say it has
/// started.
///
/// The hold exists to drop an interactive login shell's startup noise. It is
/// bounded because the alternative is a run whose output never appears: a shell
/// that dies during its own rc files never prints the marker, and the thing it
/// printed instead is the only explanation the operator will get. Past this
/// much, the suppression gives up and everything held is released -- a little
/// noise is a far smaller failure than a silent log.
const STARTUP_HOLD_LIMIT: usize = 64 * 1024;

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
fn send<W: Write>(out: &mut W, frame: &ExecFrame) {
    let pkt = encode_packet(&Payload::Exec(frame.clone()));
    let _ = out.write_all(&pkt);
    let _ = out.flush();
}

/// Send raw output, split so no frame can exceed what its length field can
/// describe.
fn send_out<W: Write>(out: &mut W, stream: Stream, seq: &mut u32, bytes: &[u8]) {
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
    let lock_path = match req.lock_path {
        Some(p) => p,
        None => {
            default_lock = lock::default_path();
            &default_lock
        }
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

    let sudo_rejected = pump(req, out, seq, &mut child);
    child.close();
    let mut outcome = pty::wait(child.pid);
    // The marker is the authority when it fired: a shell can lose an exit
    // status through a login profile, and reporting a refused password as
    // "exited 1" is what sent operators to read a correct upgrade script.
    if sudo_rejected && outcome.code != SUDO_FAILED_CODE {
        outcome.code = SUDO_FAILED_CODE;
        outcome.signalled = false;
    }
    outcome
}

/// Read both descriptors until they close. Returns whether sudo refused.
fn pump<W: Write>(req: &Request, out: &mut W, seq: &mut u32, child: &mut pty::Child) -> bool {
    let started = Instant::now();
    let mut last_alive = Instant::now();
    let mut sieve = Sieve::new();
    // Its own sieve, because a marker must be recognised on whichever stream it
    // arrives on. One scanner per stream and not one *rule* per stream: the
    // rule was written twice once before, each half looking at a different
    // stream, and that is how `__multitop_lock_held__` came to be printed into
    // an operator's log verbatim while its detection sat on the stream it never
    // arrived on.
    let mut err_sieve = Sieve::new();
    let mut buf = [0u8; READ_BUF];
    let mut sudo_rejected = false;
    let mut password_sent = false;
    // Everything stdout produced before the shell said it had finished
    // starting. Released, not dropped, if the marker never comes.
    let mut held: Option<Vec<u8>> = Some(Vec::new());
    // Set once the command itself has finished. What a login shell writes on
    // its way out is never the operator's output.
    let mut done = false;

    while child.master >= 0 || child.errpipe >= 0 {
        let (m_ready, e_ready) = pty::poll_both(child.master, child.errpipe, POLL_MS);

        if m_ready {
            match pty::read_fd(child.master, &mut buf) {
                Ok(0) | Err(_) => {
                    let tail = sieve.finish();
                    consume(
                        &tail,
                        out,
                        seq,
                        &mut Suppress {
                            held: &mut held,
                            done: &mut done,
                        },
                        child.master,
                        req,
                        &mut password_sent,
                        &mut sudo_rejected,
                    );
                    // The shell never said it started, so what it did say is
                    // all the explanation there is. Release it.
                    release(&mut held, out, seq);
                    // SAFETY: opened by `pty::spawn`, closed once here; the
                    // sentinel keeps `poll` from being handed a stale number.
                    unsafe { libc::close(child.master) };
                    child.master = -1;
                }
                Ok(n) => {
                    let sifted = sieve.feed(&buf[..n]);
                    consume(
                        &sifted,
                        out,
                        seq,
                        &mut Suppress {
                            held: &mut held,
                            done: &mut done,
                        },
                        child.master,
                        req,
                        &mut password_sent,
                        &mut sudo_rejected,
                    );
                }
            }
        }

        if e_ready {
            match pty::read_fd(child.errpipe, &mut buf) {
                Ok(0) | Err(_) => {
                    let tail = err_sieve.finish();
                    emit_stderr(&tail, out, seq, &mut sudo_rejected);
                    // SAFETY: as above.
                    unsafe { libc::close(child.errpipe) };
                    child.errpipe = -1;
                }
                Ok(n) => {
                    let sifted = err_sieve.feed(&buf[..n]);
                    emit_stderr(&sifted, out, seq, &mut sudo_rejected);
                }
            }
        }

        if last_alive.elapsed() >= ALIVE_EVERY {
            last_alive = Instant::now();
            #[allow(clippy::cast_possible_truncation)]
            send(
                out,
                &ExecFrame::Alive {
                    elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
                },
            );
        }
    }
    sudo_rejected
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

/// Act on one feed's pieces **in order**.
///
/// Order is the whole reason this takes a sequence rather than two lists. A
/// single 8 KiB read routinely contains the shell's startup noise, the
/// `Started` marker, and the first lines of real output; handled out of order,
/// the real output is dropped with the noise. That is not hypothetical -- it is
/// what the first version of this did.
#[allow(clippy::too_many_arguments)]
fn consume<W: Write>(
    pieces: &[Piece],
    out: &mut W,
    seq: &mut u32,
    sup: &mut Suppress,
    master: std::os::fd::RawFd,
    req: &Request,
    password_sent: &mut bool,
    sudo_rejected: &mut bool,
) {
    for piece in pieces {
        match piece {
            Piece::Out(bytes) => {
                if !*sup.done {
                    stash(sup.held, out, seq, bytes);
                }
            }
            // The two boundaries. Neither is news for the client: they say
            // which side of the command a byte fell on, and the bytes outside
            // it were the shell talking to itself.
            Piece::Mark(MarkerKind::Started) => *sup.held = None,
            Piece::Mark(MarkerKind::Done) => *sup.done = true,
            Piece::Mark(k) => {
                send(out, &ExecFrame::Marker(*k));
                match k {
                    MarkerKind::PwReady if !*password_sent => {
                        // Echo is off on the far side now; before this point
                        // the pty would print the password straight back into
                        // the operator's log.
                        if let Some(p) = req.password {
                            let mut line = p.as_bytes().to_vec();
                            line.push(b'\n');
                            let _ = pty::write_fd(master, &line);
                            *password_sent = true;
                        }
                    }
                    MarkerKind::SudoFailed => *sudo_rejected = true,
                    _ => {}
                }
            }
        }
    }
}

/// Forward stderr, with the agent's own markers taken out of it.
///
/// `Started` and `Done` bracket stdout only -- they are printed by the wrapper
/// to the pty -- so on this stream they are ordinary text and would be a marker
/// the operator typed. They are dropped either way: a line that is exactly one
/// of our sentinels is ours by definition, and showing it would be showing an
/// internal marker.
fn emit_stderr<W: Write>(pieces: &[Piece], out: &mut W, seq: &mut u32, sudo_rejected: &mut bool) {
    for piece in pieces {
        match piece {
            Piece::Out(bytes) => send_out(out, Stream::Stderr, seq, bytes),
            Piece::Mark(MarkerKind::Started | MarkerKind::Done) => {}
            Piece::Mark(k) => {
                if *k == MarkerKind::SudoFailed {
                    *sudo_rejected = true;
                }
                send(out, &ExecFrame::Marker(*k));
            }
        }
    }
}

/// Which side of the operator's command the reader is on.
struct Suppress<'a> {
    /// Output held while the login shell is still starting, or `None` once the
    /// command has begun.
    held: &'a mut Option<Vec<u8>>,
    /// Set once the command has finished.
    done: &'a mut bool,
}

/// Hold output back while the shell is still starting, or forward it.
///
/// The hold is bounded: past [`STARTUP_HOLD_LIMIT`] it is abandoned and
/// everything since the start of the run is forwarded. A quiet log is a worse
/// failure than a noisy one.
fn stash<W: Write>(held: &mut Option<Vec<u8>>, out: &mut W, seq: &mut u32, bytes: &[u8]) {
    let Some(buf) = held.as_mut() else {
        send_out(out, Stream::Stdout, seq, bytes);
        return;
    };
    buf.extend_from_slice(bytes);
    if buf.len() >= STARTUP_HOLD_LIMIT {
        release(held, out, seq);
    }
}

/// Forward whatever is still held and stop holding.
fn release<W: Write>(held: &mut Option<Vec<u8>>, out: &mut W, seq: &mut u32) {
    if let Some(buf) = held.take() {
        if !buf.is_empty() {
            send_out(out, Stream::Stdout, seq, &buf);
        }
    }
}
