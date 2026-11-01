//! What the client says on the agent's stdin: its panel's size, and -- by
//! closing it -- that it has gone.
//!
//! The client draws the panel; the agent decides how many process rows to
//! sample for it. That count used to come from laying out a pretend 120x50
//! panel, whatever the client's actually was -- 92 rows on a small host, 72
//! on a 128-core one, while a full-screen panel on a 200x100 terminal draws
//! 192, and the user resizes the terminal whenever they like (roadmap M2). So
//! the client says the size, once the agent is running and again whenever it
//! changes, as one line: `size <cols> <lines>`.
//!
//! The line arrives on the stdin the agent already watches for EOF, so an
//! older agent reads it and throws it away, and a newer agent that is never
//! told keeps the old layout. It is written only after the agent's Hello,
//! because before that the bytes would reach the bootstrap `sh`, which reads
//! its script from the same stdin.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::consts::STDIN_WATCH_BUF;

/// The size the client last said its panel is, shared with the watcher
/// thread. Packed into one atomic so a reader never sees the columns of one
/// size with the lines of another.
#[derive(Clone, Debug, Default)]
pub struct PanelSize(Arc<AtomicU32>);

impl PanelSize {
    /// `(cols, lines)`, or `None` until the client has said.
    #[must_use]
    pub fn get(&self) -> Option<(usize, usize)> {
        let v = self.0.load(Ordering::Relaxed);
        (v != 0).then(|| (usize::from(hi(v)), usize::from(lo(v))))
    }

    pub fn set(&self, cols: u16, lines: u16) {
        self.0
            .store(u32::from(cols) << 16 | u32::from(lines), Ordering::Relaxed);
    }
}

const fn hi(v: u32) -> u16 {
    (v >> 16) as u16
}

const fn lo(v: u32) -> u16 {
    (v & 0xFFFF) as u16
}

/// The line that tells the agent its panel is `cols` x `lines`: the one
/// spelling, for the client that writes it and the agent that reads it.
#[must_use]
pub fn size_line(cols: u16, lines: u16) -> String {
    format!("size {cols} {lines}\n")
}

/// The size a `size <cols> <lines>` line states. Anything else -- another
/// word, a zero, a number past `u16` -- states nothing.
#[must_use]
pub fn parse_size_line(line: &[u8]) -> Option<(u16, u16)> {
    let line = std::str::from_utf8(line).ok()?;
    let mut words = line.split_ascii_whitespace();
    if words.next()? != "size" {
        return None;
    }
    let cols: u16 = words.next()?.parse().ok()?;
    let lines: u16 = words.next()?.parse().ok()?;
    (words.next().is_none() && cols > 0 && lines > 0).then_some((cols, lines))
}

/// Read `input` to its end, recording every size it states in `size`.
///
/// A line longer than the buffer is no command this reads, and is passed
/// over whole, as is a line that states nothing.
pub fn read_commands(input: impl Read, size: &PanelSize) {
    crate::proc_read::for_each_line_in(input, &mut [0u8; STDIN_WATCH_BUF], |line| {
        if let Some((cols, lines)) = parse_size_line(line.as_bytes()) {
            size.set(cols, lines);
        }
    });
}

/// Watch stdin: record the sizes the client states in `size`, and return the
/// flag that goes up when stdin reaches EOF, which is how the agent learns its
/// reader is gone.
///
/// Only a pipe or a socket is watched -- what a client's stdin is: a
/// terminal's stdin belongs to the user, and `/dev/null` says nothing. It is
/// asked of the descriptor itself (`fstat`), not of `/proc/self/fd/0`, which
/// a Mac does not have: the local panel's agent runs there.
#[must_use]
pub fn stdin_watcher(size: &PanelSize) -> Arc<AtomicBool> {
    use std::io::{self, IsTerminal};
    use std::os::fd::AsFd;
    use std::os::unix::fs::FileTypeExt;

    let stdin_gone = Arc::new(AtomicBool::new(false));
    let stdin_pipe = !io::stdin().is_terminal()
        && io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map(std::fs::File::from)
            .and_then(|f| f.metadata())
            .is_ok_and(|m| m.file_type().is_fifo() || m.file_type().is_socket());
    if stdin_pipe {
        let (sig, size) = (stdin_gone.clone(), size.clone());
        std::thread::spawn(move || {
            read_commands(io::stdin(), &size);
            sig.store(true, Ordering::Relaxed);
        });
    }
    stdin_gone
}
