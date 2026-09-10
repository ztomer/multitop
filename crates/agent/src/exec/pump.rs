//! Polling descriptors and streaming frames until the child completes.

#![expect(
    unsafe_code,
    reason = "FFI boundary; see the unsafe_code note in lib.rs"
)]

use std::io::Write;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use super::pty;
use super::run::{send, send_out, Request};
use super::sieve::{Piece, Sieve};
use super::{ExecFrame, MarkerKind, Stream};

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

/// Read both descriptors until they close or the child exits.
/// Returns `(reaped_outcome, sudo_rejected)`.
pub fn pump<W: Write>(
    req: &Request<'_>,
    out: &mut W,
    seq: &mut u32,
    child: &mut pty::Child,
) -> (Option<pty::Outcome>, bool) {
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
    let mut reaped: Option<pty::Outcome> = None;

    while child.master >= 0 || child.errpipe >= 0 {
        let (m_ready, e_ready) = pty::poll_both(child.master, child.errpipe, POLL_MS);

        if m_ready {
            read_master_chunk(
                child,
                &mut buf,
                &mut sieve,
                out,
                seq,
                &mut held,
                &mut done,
                req,
                &mut password_sent,
                &mut sudo_rejected,
            );
        }

        if e_ready {
            read_errpipe_chunk(
                child,
                &mut buf,
                &mut err_sieve,
                out,
                seq,
                &mut sudo_rejected,
            );
        }

        if reaped.is_none() {
            reaped = pty::try_wait(child.pid);
        }

        // If the shell process has exited, background daemons or subshells may
        // still hold the pty slave or stderr pipe open. Drain whatever is already
        // queued in OS buffers, flush the sieves, and exit the pump loop rather
        // than hanging forever.
        if reaped.is_some() {
            drain_post_exit(
                child,
                &mut buf,
                &mut sieve,
                &mut err_sieve,
                out,
                seq,
                &mut held,
                &mut done,
                req,
                &mut password_sent,
                &mut sudo_rejected,
            );
            break;
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
    (reaped, sudo_rejected)
}

#[allow(clippy::too_many_arguments)]
fn drain_post_exit<W: Write>(
    child: &mut pty::Child,
    buf: &mut [u8],
    sieve: &mut Sieve,
    err_sieve: &mut Sieve,
    out: &mut W,
    seq: &mut u32,
    held: &mut Option<Vec<u8>>,
    done: &mut bool,
    req: &Request<'_>,
    password_sent: &mut bool,
    sudo_rejected: &mut bool,
) {
    while child.master >= 0 || child.errpipe >= 0 {
        let (m_drain, e_drain) = pty::poll_both(child.master, child.errpipe, 0);
        if !m_drain && !e_drain {
            break;
        }
        if m_drain {
            read_master_chunk(
                child,
                buf,
                sieve,
                out,
                seq,
                held,
                done,
                req,
                password_sent,
                sudo_rejected,
            );
        }
        if e_drain {
            read_errpipe_chunk(child, buf, err_sieve, out, seq, sudo_rejected);
        }
    }
    if child.master >= 0 {
        let tail = sieve.finish();
        consume(
            &tail,
            out,
            seq,
            &mut Suppress { held, done },
            child.master,
            req,
            password_sent,
            sudo_rejected,
        );
        release(held, out, seq);
        unsafe { libc::close(child.master) };
        child.master = -1;
    }
    if child.errpipe >= 0 {
        let tail = err_sieve.finish();
        emit_stderr(&tail, out, seq, sudo_rejected);
        unsafe { libc::close(child.errpipe) };
        child.errpipe = -1;
    }
}

#[allow(clippy::too_many_arguments)]
fn read_master_chunk<W: Write>(
    child: &mut pty::Child,
    buf: &mut [u8],
    sieve: &mut Sieve,
    out: &mut W,
    seq: &mut u32,
    held: &mut Option<Vec<u8>>,
    done: &mut bool,
    req: &Request<'_>,
    password_sent: &mut bool,
    sudo_rejected: &mut bool,
) {
    match pty::read_fd(child.master, buf) {
        Ok(0) | Err(_) => {
            let tail = sieve.finish();
            consume(
                &tail,
                out,
                seq,
                &mut Suppress { held, done },
                child.master,
                req,
                password_sent,
                sudo_rejected,
            );
            release(held, out, seq);
            unsafe { libc::close(child.master) };
            child.master = -1;
        }
        Ok(n) => {
            let sifted = sieve.feed(&buf[..n]);
            consume(
                &sifted,
                out,
                seq,
                &mut Suppress { held, done },
                child.master,
                req,
                password_sent,
                sudo_rejected,
            );
        }
    }
}

fn read_errpipe_chunk<W: Write>(
    child: &mut pty::Child,
    buf: &mut [u8],
    err_sieve: &mut Sieve,
    out: &mut W,
    seq: &mut u32,
    sudo_rejected: &mut bool,
) {
    match pty::read_fd(child.errpipe, buf) {
        Ok(0) | Err(_) => {
            let tail = err_sieve.finish();
            emit_stderr(&tail, out, seq, sudo_rejected);
            unsafe { libc::close(child.errpipe) };
            child.errpipe = -1;
        }
        Ok(n) => {
            let sifted = err_sieve.feed(&buf[..n]);
            emit_stderr(&sifted, out, seq, sudo_rejected);
        }
    }
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
    sup: &mut Suppress<'_>,
    master: RawFd,
    req: &Request<'_>,
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
