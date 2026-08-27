#![allow(clippy::must_use_candidate)]
use crate::ssh;

pub fn painted_states(line: &str) -> impl DoubleEndedIterator<Item = &str> {
    line.trim_end_matches('\n')
        .split('\r')
        .map(|state| state.trim_end_matches('\r'))
}

/// A marker the remote prints for this program rather than for the operator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Marker {
    /// `sudo` refused the password we handed it.
    SudoFailed,
    /// Another run already holds the upgrade lock.
    LockHeld,
}

/// Recognise a marker on **either** stream, and take it out of the log.
///
/// It used to be one sentinel per stream -- `SUDO_FAILED` checked only on
/// stdout, `LOCK_HELD` only on stderr -- which describes the *local* shape,
/// where the two pipes stay separate.
///
/// A remote upgrade runs under `ssh -tt`, and **a pty has one stream**: sshd
/// merges the remote's stderr into it, so everything the remote writes arrives
/// on the local client's stdout. The held-lock sentinel therefore reached the
/// branch that was not looking for it. Two things followed. The detection was
/// dead for every remote host -- only the distinct exit code still caught it,
/// so the sentinel's whole purpose, surviving a lost exit status in a noisy
/// login shell, was gone. And because the stdout branch had no reason to skip
/// it, `__multitop_lock_held__` was printed into the operator's upgrade log
/// verbatim: an internal marker shown as output.
///
/// One scanner, both streams -- the same rule `is_sudo_help` is written under,
/// for the same reason: two streams disagreeing about what counts is how one of
/// them stops recognising it.
pub fn marker(trimmed: &str) -> Option<Marker> {
    if trimmed == ssh::SUDO_FAILED_SENTINEL {
        return Some(Marker::SudoFailed);
    }
    if trimmed == ssh::LOCK_HELD_SENTINEL {
        return Some(Marker::LockHeld);
    }
    None
}

/// Whether a line is sudo explaining that it could not ask for a password.
///
/// One copy, because the two streams disagreeing about what counts is how one
/// of them stops recognising it.
///
/// Also matches the shape a non-root `upgrade_cmd` produces: apt's "are you
/// root?" and dpkg's "permission denied" on its own lock files contain no
/// "sudo", so without these arms a command that merely forgot the `sudo`
/// prefix was reported as a failing command with no hint why.
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
    /// The text to show, with its control sequences consumed.
    pub text: String,
    /// How far back from the newest line to write it. `0` appends.
    pub back: usize,
    /// Rows below this one the tool has just erased, and which must be blanked
    /// rather than left showing the frame before last.
    pub erase_below: usize,
}

/// The screen a repainting tool thinks it is drawing on.
///
/// `painted_states` handles the tool that rewrites **one** line with carriage
/// returns — `apt`, `curl`. It cannot handle the tool that rewrites **several**:
/// `docker compose pull` prints a block, moves the cursor up over it with
/// `ESC[nA`, and prints the block again. Every repaint was one more copy of the
/// block in the log, so a pull of five layers buried the run in its own
/// progress display.
///
/// This is the second, smaller state machine the `ansi` module's SGR parser has
/// always needed beside it. It tracks one number — how far above the newest
/// line the cursor is — because that is all a line-oriented reader can know.
/// Columns are not modelled: a reader that hands over whole lines cannot see a
/// partial overwrite, and pretending otherwise would invent detail the input
/// does not carry.
#[derive(Default, Debug)]
pub struct Painter {
    /// Rows above the newest line that the next write lands on.
    up: usize,
}

impl Painter {
    #[must_use]
    pub const fn new() -> Self {
        Self { up: 0 }
    }

    /// Consume one line of output and say where it belongs.
    ///
    /// `None` when the line carried nothing but cursor movement — the movement
    /// is still recorded, so the next line that does carry text lands where the
    /// tool meant it to.
    pub fn feed(&mut self, raw: &str) -> Option<Paint> {
        let (moved_up, moved_down, erase_below, rest) = Self::consume_controls(raw);
        self.up = self.up.saturating_add(moved_up).saturating_sub(moved_down);

        // The last state a carriage return left the line in, as before: the
        // two mechanisms compose, because a tool may do both.
        let text = painted_states(&rest).next_back().unwrap_or("");
        let back = self.up;

        // Nothing to draw. The movement has already been recorded, so the next
        // line that does carry text lands where the tool meant it to — tools
        // routinely split the movement and the text across two writes.
        if text.is_empty() {
            // Nothing below the append point to erase, and nothing to draw.
            if erase_below == 0 || back == 0 {
                return None;
            }
            return Some(Paint {
                text: String::new(),
                back,
                erase_below,
            });
        }

        // A line written at a row means the cursor has moved past it.
        self.up = self.up.saturating_sub(1);
        Some(Paint {
            text: text.to_string(),
            back,
            erase_below,
        })
    }

    /// Split the leading control sequences off a line.
    ///
    /// Returns how far the cursor moved up, how far down, how many rows below
    /// were erased, and the text that is left.
    fn consume_controls(raw: &str) -> (usize, usize, usize, String) {
        let (mut up, mut down, mut erase) = (0usize, 0usize, 0usize);
        let mut rest = raw;
        let mut text_prefix = String::new();

        while !rest.is_empty() {
            if let Some(after_cr) = rest.strip_prefix('\r') {
                rest = after_cr;
                continue;
            }
            if let Some(after_esc) = rest.strip_prefix("\u{1b}[") {
                // Parse CSI parameter bytes (0x30..=0x3F: digits, ';', '?', etc.)
                let param_len = after_esc
                    .chars()
                    .take_while(|c| ('\x30'..='\x3F').contains(c))
                    .map(char::len_utf8)
                    .sum();
                let param = &after_esc[..param_len];
                let after_param = &after_esc[param_len..];

                // Parse intermediate bytes (0x20..=0x2F: space, '$', ''', etc.)
                let inter_len = after_param
                    .chars()
                    .take_while(|c| ('\x20'..='\x2F').contains(c))
                    .map(char::len_utf8)
                    .sum();
                let after_inter = &after_param[inter_len..];

                let Some(final_byte) = after_inter.chars().next() else {
                    // Truncated ESC sequence at end of string
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
                    'm' => {
                        // Preserve SGR color/style codes in the text
                        text_prefix.push_str(csi_full);
                    }
                    _ => {
                        text_prefix.push_str(csi_full);
                    }
                }
            } else {
                break;
            }
        }

        text_prefix.push_str(rest);
        (up, down, erase, text_prefix)
    }
}
