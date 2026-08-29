//! Taking the agent's own markers out of the child's output.
//!
//! The markers are still printed by shell code inside the child -- that is the
//! only way a `sudo` preamble can say "echo is off, send it now". What changed
//! is where they are recognised: **here**, on the host, inside the process that
//! owns the pty and therefore knows exactly what a line is. They never reach
//! the wire as text, so they cannot be mistaken for output and cannot be missed
//! for want of a newline.
//!
//! That is the fix for a defect that has happened twice already:
//! `__multitop_lock_held__` was printed into an operator's upgrade log
//! verbatim, and the same marker's detection was dead on every remote host
//! because it was being looked for on the stream it did not arrive on.
//!
//! # Why a partial line is usually released rather than held
//!
//! A prompt is a line with no newline on the end -- `Continue? [Y/n] ` -- and
//! the operator cannot answer one they cannot see. So the trailing partial is
//! held back **only** when it is still a possible prefix of a marker. Anything
//! else goes out the moment it arrives.
//!
//! The old reader had no such rule. It held every partial and flushed it on a
//! 100 ms timer, which is how a prompt arrived a tenth of a second late, and --
//! because the flush did not clear the buffer it had just sent -- how the same
//! text was emitted again on every tick after that.

use super::MarkerKind;

/// The lines the agent speaks to itself in, and what each one means.
const MARKERS: [(&str, MarkerKind); 5] = [
    (super::PW_READY_SENTINEL, MarkerKind::PwReady),
    (super::SUDO_FAILED_SENTINEL, MarkerKind::SudoFailed),
    (super::LOCK_HELD_SENTINEL, MarkerKind::LockHeld),
    (super::STARTED_SENTINEL, MarkerKind::Started),
    (super::DONE_SENTINEL, MarkerKind::Done),
];

/// One thing the sieve found, in the order it appeared.
///
/// Ordered, and not two separate lists of output and markers. The first
/// version of this returned `{ out, markers }`, which loses the interleaving
/// inside a single read -- and the interleaving is load-bearing twice over:
/// `Started` says which side of it output belongs on, and `PwReady` says the
/// far side has turned echo off, so anything written before it is echoed back
/// into the operator's log. Both of those are "where in the stream", and a
/// structure that cannot express where cannot carry them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    Out(Vec<u8>),
    Mark(MarkerKind),
}

/// What one feed produced, in order.
pub type Sifted = Vec<Piece>;

/// Append `bytes` to the last run of output, or start a new one.
///
/// Coalescing keeps one line of output from becoming one frame per read.
fn push_out(sifted: &mut Sifted, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    match sifted.last_mut() {
        Some(Piece::Out(buf)) => buf.extend_from_slice(bytes),
        _ => sifted.push(Piece::Out(bytes.to_vec())),
    }
}

/// Splits a byte stream into lines only far enough to find markers in it.
#[derive(Debug, Default)]
pub struct Sieve {
    partial: Vec<u8>,
}

impl Sieve {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            partial: Vec::new(),
        }
    }

    /// Consume a chunk.
    pub fn feed(&mut self, chunk: &[u8]) -> Sifted {
        let mut sifted = Sifted::new();
        self.partial.extend_from_slice(chunk);

        // Complete lines first. `\n` and not `\r`, because a `\r` is a repaint
        // of the line being written, not the end of it -- splitting on it would
        // make a progress bar into a hundred lines, which is a defect this
        // project has already shipped once.
        while let Some(nl) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=nl).collect();
            match marker_of(&line) {
                Some(kind) => sifted.push(Piece::Mark(kind)),
                None => push_out(&mut sifted, &line),
            }
        }

        // Then the tail, unless it might still turn into a marker.
        if !self.partial.is_empty() && !could_become_marker(&self.partial) {
            let tail = std::mem::take(&mut self.partial);
            push_out(&mut sifted, &tail);
        }
        sifted
    }

    /// Release whatever is still held, at end of stream.
    ///
    /// A marker needs its newline to be a marker; a partial that never got one
    /// is output, and output is never dropped. Losing the last line of a run
    /// because it looked like it might have become something else is how a
    /// failure ends with an empty log.
    pub fn finish(&mut self) -> Sifted {
        let mut sifted = Sifted::new();
        if self.partial.is_empty() {
            return sifted;
        }
        match marker_of(&self.partial) {
            Some(kind) => {
                self.partial.clear();
                sifted.push(Piece::Mark(kind));
            }
            None => {
                let tail = std::mem::take(&mut self.partial);
                push_out(&mut sifted, &tail);
            }
        }
        sifted
    }
}

/// Whether a whole line is exactly one marker.
///
/// Every state a carriage return left the line in is checked, not only the last
/// one: a marker printed onto a line a progress bar had already written to
/// arrives as `…\r__multitop_lock_held__\n`, and looking only at the whole line
/// would miss it.
fn marker_of(line: &[u8]) -> Option<MarkerKind> {
    let text = String::from_utf8_lossy(line);
    text.split('\r')
        .map(str::trim)
        .find_map(|state| MARKERS.iter().find(|(s, _)| *s == state).map(|(_, k)| *k))
}

/// Whether a partial line could still grow into a marker.
///
/// Compared against the last carriage-return state for the same reason
/// [`marker_of`] scans them all.
fn could_become_marker(partial: &[u8]) -> bool {
    // A marker line is short; anything longer cannot become one, and this stops
    // a tool that writes a megabyte without a newline from being buffered.
    let longest = MARKERS.iter().map(|(s, _)| s.len()).max().unwrap_or(0);
    if partial.len() > longest {
        return false;
    }
    let text = String::from_utf8_lossy(partial);
    let Some(state) = text.split('\r').next_back() else {
        return false;
    };
    let state = state.trim_start();
    if state.is_empty() {
        return false;
    }
    MARKERS.iter().any(|(s, _)| s.starts_with(state))
}
