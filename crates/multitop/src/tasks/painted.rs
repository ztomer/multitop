//! The screen a repainting tool thinks it is drawing on.
//!
//! Fed raw bytes now, not lines. That is the whole difference: the agent
//! forwards exactly what the child wrote, so a chunk boundary here is a real
//! write boundary rather than whatever a pipe happened to hand over, and the
//! reader no longer has to guess where a record ends.
//!
//! # What this replaced, and why the shape changed
//!
//! The previous version took `feed(&str, newline: bool)` and was called once
//! per `\r`- or `\n`-delimited fragment by a reader that also ran a 100 ms
//! timer to flush partial lines. Three defects lived in that arrangement:
//!
//! * the timer's flush did not clear the buffer it had just sent, so an
//!   unterminated line went out again on every tick;
//! * splitting at `\r` *and* `\n` fed a CRLF line to the painter twice, and the
//!   second, empty feed decremented the cursor a second time -- so `ESC[nA`
//!   block repaints drifted and appended copies instead of overwriting;
//! * and `painted_states("a\r")` is `["a", ""]`, whose last element is empty.
//!   Under a pty every line ends `\r\n`, so on the unmultiplexed transport --
//!   the one a stale file at the `ControlPath` silently selects -- every line
//!   of output collapsed to nothing. The line-based reader that came before it
//!   did not have this problem, because `tokio`'s `Lines` strips the `\r` of a
//!   CRLF for you. Splitting by hand lost that and nothing noticed.
//!
//! So: one entry point, fed bytes, idempotent in the only place it repeats.

/// Every state a line passed through, in order, as carriage returns rewrote it.
///
/// No `#[must_use]` here, and that is deliberate. `Iterator` is already
/// `#[must_use]`, so `impl DoubleEndedIterator` carries it and a dropped
/// result warns without the attribute; adding one is `clippy::double_must_use`
/// (measured 2026-10-04), which is a gate failure rather than a redundancy.
pub fn painted_states(line: &str) -> impl DoubleEndedIterator<Item = &str> {
    line.trim_end_matches('\n')
        .split('\r')
        .map(|state| state.trim_end_matches('\r'))
}

/// Whether a line is `sudo` explaining that it could not ask for a password.
///
/// Also matches the shape a non-root `upgrade_cmd` produces: apt's "are you
/// root?" and dpkg's "permission denied" on its own lock files contain no
/// "sudo", so without these arms a command that merely forgot the `sudo` prefix
/// was reported as a failing command with no hint why.
#[must_use]
pub fn is_sudo_help(lower: &str) -> bool {
    lower.contains("sudo")
        && (lower.contains("terminal")
            || lower.contains("password")
            || lower.contains("pre-authorized")
            || lower.contains("tty")
            || lower.contains("prompt on"))
        || lower.contains("are you root")
        || (lower.contains("permission denied")
            && (lower.contains("/var/lib/dpkg")
                || lower.contains("/var/lib/apt")
                || lower.contains("lock-frontend")))
}

/// Where one painted line lands in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paint {
    /// The text to show, with its cursor-movement sequences consumed and its
    /// colours left in.
    pub text: String,
    /// How far back from the append point to write it. `0` appends; `1` is the
    /// newest line already in the log.
    pub back: usize,
    /// Rows below this one the tool has just erased, which must be blanked
    /// rather than left showing the frame before last.
    pub erase_below: usize,
}

/// One thing a feed produced.
///
/// An enum rather than a second accessor on `Painter`, so a caller cannot
/// forget the status line exists: the match is not exhaustive without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Painted {
    /// A log line, and where it goes.
    Line(Paint),
    /// The tool's pinned status line changed. `Some` is its new text, colours
    /// kept; `None` is that the tool erased it, or that the run is over and a
    /// progress bar for a process that has gone would read as still running.
    Status(Option<String>),
}

mod reading;

use reading::{Bracket, End, Reading, StatusWrite};

/// One run's output, turned into where each line belongs.
///
/// Columns are not modelled. A `\r` is treated as "this line starts again",
/// which is what every tool that uses one actually does -- `apt`, `curl` and
/// `docker` all rewrite the whole line after it. Modelling columns properly
/// would mean tracking which cells the SGR sequences in the text apply to, and
/// a half-done version of that corrupts colour rather than improving fidelity.
///
/// Every byte is read through `multitop_agent::escape`, the one tokenizer for
/// tool output, so a sequence this model gives no meaning to is still consumed
/// whole rather than leaking into the text.
#[derive(Default, Debug)]
pub struct Painter {
    /// Rows above the append point that the *next* line lands on, as of the
    /// start of the line currently being built.
    up: usize,
    /// Whether the line being built has already been emitted once. A prompt
    /// arrives before its newline does, and the operator cannot answer one they
    /// cannot see -- so it is emitted immediately and overwritten in place as
    /// the rest of it arrives.
    open: bool,
    /// The raw bytes of the line being built, without its terminator.
    ///
    /// Kept raw, and re-derived from scratch on every feed, because that makes
    /// the repeated emission of a growing line idempotent: cursor movement is
    /// counted from `up`, which does not change until the line ends.
    raw: Vec<u8>,
    /// The save bracket in force when the line being built began. Carried
    /// across lines like `up`, and for the same reason: a line is re-read
    /// from its start on every feed.
    bracket: Bracket,
    /// The status line as last emitted, so a re-read that finds the same
    /// status emits nothing.
    status: Option<String>,
}

impl Painter {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            up: 0,
            open: false,
            raw: Vec::new(),
            bracket: Bracket::Open,
            status: None,
        }
    }

    /// Consume output and say where each line belongs.
    ///
    /// May return more than one paint for one call, and may return a paint for
    /// a line it has already painted -- that is the unterminated line growing,
    /// and it is an overwrite, never a second copy.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Vec<Painted> {
        let mut out = Vec::new();
        let mut rest = bytes;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            self.raw.extend_from_slice(&rest[..i]);
            // The `\r` of a CRLF is part of the terminator, not the line. A pty
            // puts one on the end of every line it writes; treating it as
            // content is what made `painted_states` collapse each line to the
            // empty string after it.
            if self.raw.last() == Some(&b'\r') {
                self.raw.pop();
            }
            self.paint(End::Newline, &mut out);
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            self.raw.extend_from_slice(rest);
            self.paint(End::Open, &mut out);
        }
        out
    }

    /// Nothing more is coming. Emits the last line if it never got a newline,
    /// and takes down a status line the tool left up.
    ///
    /// A run whose final line has no terminator is ordinary -- a prompt left
    /// unanswered, a tool killed mid-write -- and it has usually just said why
    /// it stopped. It is already on screen by then; this exists so a caller can
    /// close the painter without wondering whether anything is owed.
    pub fn finish(&mut self) -> Vec<Painted> {
        let mut out = Vec::new();
        if !self.raw.is_empty() {
            self.paint(End::Eof, &mut out);
        }
        if self.status.take().is_some() {
            out.push(Painted::Status(None));
        }
        out
    }

    /// Paint the line being built, and any change it made to the status.
    ///
    /// No line paint when the line carried nothing but cursor movement. The
    /// movement has still been recorded, so the next line that does carry
    /// text lands where the tool meant it to -- tools routinely split the
    /// movement and the text across two writes.
    ///
    /// A line that is genuinely *empty* is not the same thing and does paint: a
    /// blank line between two paragraphs of `apt` output is output, and
    /// swallowing it runs them together.
    fn paint(&mut self, end: End, out: &mut Vec<Painted>) {
        let terminated = end != End::Open;
        // A chunk that ends inside an escape sequence is not a paint yet: the
        // rest of the sequence is still coming, and reading the fragment
        // would misplace it. Raw is kept, so the next feed re-derives from the
        // reassembled whole.
        if !terminated && !multitop_agent::escape::is_complete(&self.raw) {
            return;
        }
        let was_open = self.open;
        let reading = Reading::of(&self.raw, self.bracket);

        let status = match reading.status {
            StatusWrite::Untouched => None,
            StatusWrite::Set(text) => Some(Some(text)),
            StatusWrite::Erased => Some(None),
        };
        if let Some(status) = status {
            if status != self.status {
                self.status.clone_from(&status);
                out.push(Painted::Status(status));
            }
        }

        // Movement is consumed at the head of every `\r`-state, not just the
        // buffer head. Live renderers rewind AFTER drawing
        // (`┘<CR><ESC>[2K<ESC>[1A…<ESC>[2K┏┓`), so a head-only read keeps the
        // trailing choreography as text: every tick appends and nothing ever
        // repaints.
        //
        // The text is the last state a carriage return left the line in -- but
        // the last *touched* one. A line that ends in a bare `\r` has had its
        // cursor sent to column 0 and nothing written there yet: the screen
        // still shows what was there, and blanking it would make every progress
        // bar flicker between its value and nothing.
        let moved_up = reading
            .states
            .iter()
            .fold(0usize, |n, s| n.saturating_add(s.up));
        let moved_down = reading
            .states
            .iter()
            .fold(0usize, |n, s| n.saturating_add(s.down));
        let erased = reading.states.iter().map(|s| s.erase).max().unwrap_or(0);
        let erase_below = reading
            .states
            .iter()
            .map(|s| s.cleared_below)
            .max()
            .unwrap_or(0)
            .max(erased);
        let touched = reading.states.iter().rposition(|s| s.touched);
        let text = touched.map_or_else(String::new, |i| reading.states[i].text.clone());

        let line_up = self.up.saturating_add(moved_up).saturating_sub(moved_down);

        // A paint that starts on an open (unterminated) line addresses rows
        // from the content row the cursor is already sitting on -- one row
        // below the append point -- so it lands one row further back than
        // the same movement from a fresh row.
        let back = if line_up > 0 {
            line_up.saturating_add(usize::from(was_open))
        } else {
            usize::from(was_open)
        };

        // Rows a rewind erased on its way up are reported with the write that
        // follows it, not as a paint of their own: until text arrives the
        // rewind is still only movement, and raw is kept for the re-read.
        let movement_only = text.is_empty() && erased == 0 && (moved_up > 0 || moved_down > 0);
        // Only the status was drawn: the log row is as it was. A newline still
        // ends a (blank) row, exactly as a terminal shows it; otherwise there
        // is no row to paint -- not yet, or not at all if the output ended.
        let status_only = end != End::Newline && touched.is_none();

        if terminated {
            // The cursor moves past a row only when something was written on
            // it. A line of pure movement leaves the cursor where it put it.
            if movement_only {
                self.up = line_up;
            } else {
                // Resuming an open line does not advance past a row: the
                // `\n` only ends the content row the cursor was already on.
                // The open row itself stays carried by `was_open`, not by
                // `up`, so the next paint's movement counts from the append
                // point instead of double-counting it.
                self.up = line_up.saturating_sub(usize::from(!was_open));
            }
            self.open = false;
            self.bracket = reading.bracket;
            self.raw.clear();
        } else if movement_only || status_only {
            // Recorded nowhere: raw is kept and the next paint recomputes
            // from the whole, so counting it here would count it twice.
        } else {
            // The movement base resets to the append point. The open row the
            // cursor sits on is carried by `was_open` from here on; leaving
            // the old base in place counts it twice on the next rewind.
            self.up = 0;
            self.open = true;
        }

        if movement_only || status_only {
            return;
        }
        out.push(Painted::Line(Paint {
            text,
            back,
            erase_below,
        }));
    }
}

/// Route anything a painter produced to the message that carries it.
///
/// `style` is applied to a log line's text (stderr arrives red) and never to
/// the status line, which carries the tool's own colours.
pub fn painted_msg(
    panel: usize,
    gen: u64,
    painted: Painted,
    style: impl FnOnce(String) -> String,
) -> crate::app::Msg {
    match painted {
        Painted::Line(paint) => {
            let line = style(paint.text.clone());
            paint_msg(panel, gen, &paint, line)
        }
        Painted::Status(status) => crate::app::Msg::AuxStatus { panel, gen, status },
    }
}

/// Route one paint to the message that places it: a paint that moved nowhere
/// appends, anything else overwrites in place.
///
/// One definition shared by the upgrade runner and the generic exec runner,
/// so the two cannot disagree about which paint overwrites. Callers style
/// the line -- stderr arrives red -- while placement is decided here.
#[must_use]
pub const fn paint_msg(panel: usize, gen: u64, paint: &Paint, line: String) -> crate::app::Msg {
    if paint.back == 0 && paint.erase_below == 0 {
        crate::app::Msg::AuxLine { panel, gen, line }
    } else {
        crate::app::Msg::AuxRepaint {
            panel,
            gen,
            line,
            back: paint.back,
            erase_below: paint.erase_below,
        }
    }
}
