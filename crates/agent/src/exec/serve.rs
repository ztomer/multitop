//! The `exec` mode's entry point.
//!
//! Split from `lib.rs` because that file is the agent's own CLI and this is a
//! different job with a different contract -- and because the file-length gate
//! was right to say so.

use std::io::Write;

use super::run::{emit_failure, run, Request};
use super::{ExecFrame, Stream};

/// Serve one exec request read from stdin.
///
/// The request is framed, not argued. A command and a sudo password on the
/// command line would be readable by every account on the host for as long as
/// the run lasted -- which is the reason the password came off argv in the
/// first place. The command joins it because an upgrade command routinely names
/// private hosts and internal package mirrors.
///
/// A request that never arrives, or arrives unreadable, still ends in an `Exit`
/// frame. A client given silence cannot tell a refused request from a wedged
/// host, and that ambiguity is what pinned panels in `STARTED`.
pub fn serve<W: Write>(host: &str, cols: usize, lines: usize, out: &mut W) {
    let Some(ExecFrame::Request {
        command,
        password,
        use_lock,
        cols: want_cols,
        rows: want_rows,
    }) = super::run::read_request(&mut std::io::stdin().lock())
    else {
        let mut seq = 0;
        emit_failure(
            out,
            &mut seq,
            Stream::Stderr,
            "no readable exec request on stdin",
        );
        return;
    };

    // The request's window wins when it names one; the positional arguments are
    // the fallback, so `multitop-agent exec` run by hand still gets a sane pty
    // rather than a 0x0 one.
    let req = Request {
        command: &command,
        password: password.as_deref(),
        use_lock,
        cols: if want_cols == 0 { fit(cols) } else { want_cols },
        rows: if want_rows == 0 {
            fit(lines)
        } else {
            want_rows
        },
        host,
        lock_path: None,
    };
    run(&req, out);
}

/// A terminal dimension that fits in the field a `winsize` has for it.
fn fit(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}
