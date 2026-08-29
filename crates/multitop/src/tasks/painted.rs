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

#![allow(clippy::must_use_candidate)]

/// Every state a line passed through, in order, as carriage returns rewrote it.
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

/// One run's output, turned into where each line belongs.
///
/// Columns are not modelled. A `\r` is treated as "this line starts again",
/// which is what every tool that uses one actually does -- `apt`, `curl` and
/// `docker` all rewrite the whole line after it. Modelling columns properly
/// would mean tracking which cells the SGR sequences in the text apply to, and
/// a half-done version of that corrupts colour rather than improving fidelity.
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
}

impl Painter {
    pub const fn new() -> Self {
        Self {
            up: 0,
            open: false,
            raw: Vec::new(),
        }
    }

    /// Consume output and say where each line belongs.
    ///
    /// May return more than one paint for one call, and may return a paint for
    /// a line it has already painted -- that is the unterminated line growing,
    /// and it is an overwrite, never a second copy.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Vec<Paint> {
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
            out.extend(self.paint(true));
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            self.raw.extend_from_slice(rest);
            out.extend(self.paint(false));
        }
        out
    }

    /// Nothing more is coming. Emits the last line if it never got a newline.
    ///
    /// A run whose final line has no terminator is ordinary -- a prompt left
    /// unanswered, a tool killed mid-write -- and it has usually just said why
    /// it stopped. It is already on screen by then; this exists so a caller can
    /// close the painter without wondering whether anything is owed.
    pub fn finish(&mut self) -> Option<Paint> {
        if self.raw.is_empty() {
            return None;
        }
        self.paint(true)
    }

    /// `None` when the line carried nothing but cursor movement.
    ///
    /// The movement has still been recorded, so the next line that does carry
    /// text lands where the tool meant it to -- tools routinely split the
    /// movement and the text across two writes.
    ///
    /// A line that is genuinely *empty* is not the same thing and does paint: a
    /// blank line between two paragraphs of `apt` output is output, and
    /// swallowing it runs them together.
    fn paint(&mut self, terminated: bool) -> Option<Paint> {
        let raw = String::from_utf8_lossy(&self.raw).into_owned();
        let (moved_up, moved_down, erase_below, body) = Self::consume_controls(&raw);
        let line_up = self.up.saturating_add(moved_up).saturating_sub(moved_down);

        // The last state a carriage return left the line in -- but the last
        // *non-empty* one. A line that ends in a bare `\r` has had its cursor
        // sent to column 0 and nothing written there yet: the screen still
        // shows what was there, and blanking it would make every progress bar
        // flicker between its value and nothing.
        let text = painted_states(&body)
            .rfind(|state| !state.is_empty())
            .unwrap_or("")
            .to_string();

        let back = if line_up > 0 {
            line_up
        } else {
            usize::from(self.open)
        };

        let movement_only = text.is_empty() && erase_below == 0 && (moved_up > 0 || moved_down > 0);

        if terminated {
            // The cursor moves past a row only when something was written on
            // it. A line of pure movement leaves the cursor where it put it.
            if movement_only {
                self.up = line_up;
            } else {
                self.up = line_up.saturating_sub(1);
            }
            self.open = false;
            self.raw.clear();
        } else if !movement_only {
            self.open = true;
        }

        if movement_only {
            return None;
        }
        Some(Paint {
            text,
            back,
            erase_below,
        })
    }

    /// Split the cursor-movement sequences off a line.
    ///
    /// Returns how far the cursor moved up, how far down, how many rows below
    /// were erased, and the text that is left -- with SGR colour sequences kept,
    /// because they are what the line looks like rather than where it goes.
    fn consume_controls(raw: &str) -> (usize, usize, usize, String) {
        let (mut up, mut down, mut erase) = (0usize, 0usize, 0usize);
        let mut rest = raw;
        let mut text_prefix = String::new();

        while !rest.is_empty() {
            if let Some(after_cr) = rest.strip_prefix('\r') {
                rest = after_cr;
                continue;
            }
            let Some(after_esc) = rest.strip_prefix("\u{1b}[") else {
                break;
            };
            // CSI parameter bytes (0x30..=0x3F: digits, ';', '?').
            let param_len = after_esc
                .chars()
                .take_while(|c| ('\x30'..='\x3F').contains(c))
                .map(char::len_utf8)
                .sum();
            let param = &after_esc[..param_len];
            let after_param = &after_esc[param_len..];

            // Intermediate bytes (0x20..=0x2F).
            let inter_len = after_param
                .chars()
                .take_while(|c| ('\x20'..='\x2F').contains(c))
                .map(char::len_utf8)
                .sum();
            let after_inter = &after_param[inter_len..];

            let Some(final_byte) = after_inter.chars().next() else {
                // A sequence cut in half by a chunk boundary. Left in place: the
                // rest of it is in the next chunk, and the whole line is
                // re-derived from raw bytes when that arrives.
                break;
            };

            let total_len = 2 + param_len + inter_len + final_byte.len_utf8();
            let csi_full = &rest[..total_len];
            rest = &after_inter[final_byte.len_utf8()..];

            let digits: String = param.chars().filter(char::is_ascii_digit).collect();
            let n = digits.parse::<usize>().unwrap_or(1).max(1);

            match final_byte {
                'A' | 'F' => up = up.saturating_add(n),
                'B' | 'E' => down = down.saturating_add(n),
                'J' if param.is_empty() || param == "0" => erase = erase.max(1),
                'K' | 'h' | 'l' | 'G' | 'd' | 'H' | 'f' => {}
                // Everything else, SGR included, is part of how the line looks.
                _ => text_prefix.push_str(csi_full),
            }
        }

        text_prefix.push_str(rest);
        (up, down, erase, text_prefix)
    }
}
