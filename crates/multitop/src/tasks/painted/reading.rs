//! One line of tool output, read once: where the cursor went, what was
//! written outside a save bracket, and what the line did to the status row.
//!
//! Split from `painted.rs` (the file cap), along the seam that was already
//! there: this is the reading, `Painter` is what is done with it.

/// Where the cursor stands relative to a save (`ESC 7` / `ESC[s`).
///
/// apt pins its progress bar to the bottom row with `ESC 7`, an absolute move,
/// the bar, `ESC 8`. Text written between an absolute move and the restore is
/// not a log line -- it lands on a row outside the scrolling log, and the
/// restore puts the cursor back as if it had never been written. Flattening it
/// into the log is what tore the upgrade window: every dpkg line arrived glued
/// to the bar that preceded it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Bracket {
    /// No save in force: text is log text.
    #[default]
    Open,
    /// Saved, not moved: text still lands on the log row the cursor is on.
    Saved,
    /// Saved and moved to an absolute row: text is the pinned status line.
    Pinned,
}

/// How the line being painted ended, as far as this paint knows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    /// Not yet: more of it may come.
    Open,
    /// A newline: the row is finished, blank or not.
    Newline,
    /// The output ended: the row is finished, but no newline wrote it.
    Eof,
}

/// One `\r`-delimited state of a line, as the tokenizer read it.
#[derive(Default)]
pub(super) struct State {
    pub(super) up: usize,
    pub(super) down: usize,
    pub(super) erase: usize,
    /// SGR and printable text, outside any save bracket.
    pub(super) text: String,
    /// Whether anything at all was written outside a save bracket. A state of
    /// pure `ESC[2K` is not empty -- it erased the line -- while one that only
    /// drew the pinned status left the line as it was.
    pub(super) touched: bool,
    /// Whether the row the cursor is on has been erased (`ESC[2K`, or `ESC[K`
    /// at the head of a state, which is column 0).
    pub(super) here_cleared: bool,
    /// Rows directly below the cursor that were erased on the way up. A live
    /// display rewinds with erase, up, erase, up -- and when it then draws a
    /// SHORTER block, the rows it erased and did not redraw are blank on a
    /// terminal. Measured against `vt100`: without this they stayed in the
    /// log showing the frame before.
    pub(super) cleared_below: usize,
    /// Whether a printable character has been written. Movement is read only
    /// at the head of a state, before its text, as it always was: the line
    /// model has no columns, so a move after text has nowhere to go.
    pub(super) printed: bool,
}

/// The last thing a line did to the status line.
pub(super) enum StatusWrite {
    Untouched,
    Set(String),
    Erased,
}

/// One line's bytes, read once.
pub(super) struct Reading {
    pub(super) states: Vec<State>,
    pub(super) bracket: Bracket,
    pub(super) status: StatusWrite,
}

impl Reading {
    pub(super) fn of(raw: &[u8], bracket: Bracket) -> Self {
        use multitop_agent::escape::{scan, Token};
        let mut r = Self {
            states: vec![State::default()],
            bracket,
            status: StatusWrite::Untouched,
        };
        scan(raw, |token| {
            let pinned = r.bracket == Bracket::Pinned;
            match token {
                Token::Control(b'\r') => {
                    if pinned {
                        r.status = StatusWrite::Set(String::new());
                    } else {
                        r.states.push(State::default());
                    }
                }
                Token::Print(c) => r.write(|s| s.push(c), true),
                Token::Control(b'\t') => r.write(|s| s.push('\t'), true),
                Token::Csi(csi) if csi.is_sgr() => r.write(|s| csi.write_to(s), false),
                Token::Esc {
                    intermediates: [],
                    byte: b'7',
                } => r.bracket = Bracket::Saved,
                Token::Esc {
                    intermediates: [],
                    byte: b'8',
                } => r.bracket = Bracket::Open,
                Token::Csi(csi) if csi.intermediates.is_empty() => match csi.action {
                    's' => r.bracket = Bracket::Saved,
                    'u' => r.bracket = Bracket::Open,
                    'H' | 'f' | 'd' if r.bracket != Bracket::Open => {
                        r.bracket = Bracket::Pinned;
                        r.status = StatusWrite::Set(String::new());
                    }
                    action if r.bracket == Bracket::Open => {
                        r.move_cursor(action, csi.count(), csi.first());
                    }
                    _ => {}
                },
                Token::Csi(_)
                | Token::Control(_)
                | Token::Esc { .. }
                | Token::Osc(_)
                | Token::Opaque => {
                    if !pinned {
                        r.touch();
                    }
                }
            }
        });
        r
    }

    fn state(&mut self) -> &mut State {
        if self.states.is_empty() {
            self.states.push(State::default());
        }
        let last = self.states.len() - 1;
        &mut self.states[last]
    }

    fn touch(&mut self) {
        self.state().touched = true;
    }

    /// Text goes to the status while pinned, to the line otherwise.
    fn write(&mut self, put: impl FnOnce(&mut String), printable: bool) {
        if self.bracket == Bracket::Pinned {
            if let StatusWrite::Set(status) = &mut self.status {
                put(status);
            }
            return;
        }
        let state = self.state();
        put(&mut state.text);
        state.touched = true;
        state.printed |= printable;
    }

    /// Relative movement and erase, counted at the head of a state only.
    fn move_cursor(&mut self, action: char, n: usize, first: u16) {
        let state = self.state();
        state.touched = true;
        if state.printed {
            return;
        }
        match action {
            'A' | 'F' => {
                state.up = state.up.saturating_add(n);
                // Contiguous only: a jump of more than one row passes rows it
                // did not erase.
                state.cleared_below = if n == 1 && state.here_cleared {
                    state.cleared_below.saturating_add(1)
                } else {
                    0
                };
                state.here_cleared = false;
            }
            'B' | 'E' => {
                state.down = state.down.saturating_add(n);
                state.cleared_below = 0;
                state.here_cleared = false;
            }
            'K' => state.here_cleared = true,
            'J' if first == 0 => {
                state.erase = state.erase.max(state.up.max(1));
                // Erase-below takes the pinned row with it: it is below the
                // cursor whenever the cursor is in the log. apt's last write
                // is exactly this, clearing its own bar.
                self.status = StatusWrite::Erased;
            }
            // Absolute moves with no save, modes: nothing in a line model.
            _ => {}
        }
    }
}
