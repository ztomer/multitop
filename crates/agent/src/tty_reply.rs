//! Read a terminal's reply to a query, as bytes, bounded.
//!
//! A terminal answers `OSC 11 ; ?` ("what is your background?") on the
//! process's own stdin. The first reader took it through crossterm's event
//! parser, which turns `ESC ]` into an Alt+`]` KEY and `ESC \` into Alt+`\`,
//! so the ESC that both terminators need never reached the collected string:
//! measured on 2026-10-10 against a pty that answered with either terminator,
//! the probe timed out every time and reported `Unknown`. Detection then fell
//! back to `COLORFGBG`, which most terminals do not set, and the light-terminal
//! adaptation it feeds ran on a guess.
//!
//! The reply is not a keystroke, so it is read as what it is: bytes, with
//! `poll(2)` bounding the wait -- a terminal that does not know the answer says
//! nothing at all, and a read without a bound hangs startup (the bug the
//! original probe shipped with). One byte per read, so nothing typed after the
//! reply is consumed here: it belongs to the event loop.

use std::io;
use std::time::{Duration, Instant};

/// Read stdin until `done` says the bytes so far are a whole reply, or
/// `timeout` passes. `None` on timeout, end of input, or a read error.
///
/// Stdin must be a terminal in raw mode, or the reply waits for a newline the
/// terminal never sends.
pub fn read_reply(timeout: Duration, done: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut collected = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        // At least 1 ms, so a sub-millisecond remainder still waits rather
        // than spinning on a zero timeout.
        let ms = i32::try_from(remaining.as_millis())
            .unwrap_or(i32::MAX)
            .max(1);
        let mut fd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one live `pollfd`, and the count passed is one.
        let rc = unsafe { libc::poll(&raw mut fd, 1, ms) };
        if rc < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return None;
        }
        if rc == 0 {
            return None;
        }
        let mut byte = 0u8;
        // SAFETY: `byte` is a live local, valid for the one byte asked for.
        let n = unsafe { libc::read(libc::STDIN_FILENO, (&raw mut byte).cast(), 1) };
        if n <= 0 {
            return None;
        }
        collected.push(byte);
        if done(&collected) {
            return Some(collected);
        }
    }
}
