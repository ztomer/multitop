//! The Ops view's frame for one host: row 0 is the host banner, then one
//! line per section - a glyph, the section's name, what it says - and under
//! a section in trouble, one line per thing in trouble.
//!
//! The glyph carries the meaning and colour only reinforces it (a greyscale
//! frame reads the same): `✓` clean, `✗` the host reports a failure, `⚠` an
//! answer that could not be read, or alerts (events, not a current fault),
//! `·` a stated absence (no Docker here), `→` asking.
//!
//! The section names are the ESTATE's words, the ones the routines card uses
//! as well: `health`, `cron`, `containers`, `zfs`, `vpn`, `disk`. A view
//! that says `zfs.health` where the card says `zfs`, or `docker` where the
//! card says `containers`, is a view that disagrees with the card about the
//! same machine in the same minute (servers ROADMAP O92).

pub mod sections;

use multitop_agent::fmt::center_header;
use multitop_agent::palette_view::PaletteView;

use super::{OpsState, Part};

/// Section names are padded to this, so the summaries line up. It is the
/// width of the longest one, `containers`.
const LABEL_W: usize = 10;
/// Items under a section are indented this far.
const ITEM_INDENT: &str = "    ";

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Good,
    Bad,
    Warn,
    Quiet,
}

impl Tone {
    const fn glyph(self) -> &'static str {
        match self {
            Self::Good => "✓",
            Self::Bad => "✗",
            Self::Warn => "⚠",
            Self::Quiet => "·",
        }
    }

    const fn color(self, pal: &PaletteView) -> &str {
        match self {
            Self::Good => pal.green.as_str(),
            Self::Bad => pal.red.as_str(),
            Self::Warn => pal.yellow.as_str(),
            Self::Quiet => pal.gray.as_str(),
        }
    }
}

/// `text` cut to `width` characters.
fn clip(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

/// `n` and `noun`, plural unless there is exactly one.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// "12s", "5m", "3h", "2d": how long before `now` `ts` was.
#[must_use]
pub fn age(ts: i64, now: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;
    let d = (now - ts).max(0);
    if d < MIN {
        format!("{d}s")
    } else if d < HOUR {
        format!("{}m", d / MIN)
    } else if d < DAY {
        format!("{}h", d / HOUR)
    } else {
        format!("{}d", d / DAY)
    }
}

pub(super) struct Frame<'a> {
    pub(super) out: Vec<String>,
    cols: usize,
    pal: &'a PaletteView,
}

impl Frame<'_> {
    pub(super) fn line(&mut self, tone: Tone, label: &str, text: &str) {
        let body = clip(
            &format!("{label:<LABEL_W$} {text}"),
            self.cols.saturating_sub(2),
        );
        self.out.push(format!(
            "{}{}{} {body}",
            tone.color(self.pal),
            tone.glyph(),
            self.pal.reset
        ));
    }

    pub(super) fn item(&mut self, text: &str) {
        let body = clip(text, self.cols.saturating_sub(ITEM_INDENT.len()));
        self.out.push(format!("{ITEM_INDENT}{body}"));
    }

    pub(super) fn note(&mut self, text: &str) {
        let body = clip(text, self.cols);
        self.out
            .push(format!("{}{body}{}", self.pal.muted(), self.pal.reset));
    }
}

/// A section that is not ready: its absence or its failure, on one line.
fn not_ready<T>(f: &mut Frame<'_>, label: &str, part: &Part<T>) -> bool {
    match part {
        Part::Ready(_) => false,
        Part::Absent(why) => {
            f.line(Tone::Quiet, label, why);
            true
        }
        Part::Failed(why) => {
            f.line(Tone::Warn, label, &format!("could not read: {why}"));
            true
        }
    }
}

/// The frame for `host` in `state`, `cols` wide and at most `rows` tall,
/// at `now` (epoch seconds).
#[must_use]
pub fn render(
    host: &str,
    state: &OpsState,
    cols: usize,
    rows: usize,
    now: i64,
    pal: &PaletteView,
) -> Vec<String> {
    let mut f = Frame {
        out: vec![center_header(host, cols, pal)],
        cols,
        pal,
    };
    match state {
        OpsState::NotConfigured => {
            f.line(Tone::Quiet, "ops", "no mcp_host command for this host");
            f.note("  add  mcp = \"<command that starts mcp_host>\"  to its [[servers]] entry");
        }
        OpsState::Asking => f.note("→ asking mcp_host..."),
        OpsState::Failed { why, last } => {
            f.line(Tone::Warn, "mcp", why);
            if let Some(s) = last {
                f.note(&format!("  the last answer, {} old:", age(s.at, now)));
                sections::sections(&mut f, s, now);
            }
        }
        OpsState::Ready(s) => {
            f.note(&format!(
                "{} {} · asked {} ago",
                s.server.0,
                s.server.1,
                age(s.at, now)
            ));
            sections::sections(&mut f, s, now);
        }
    }
    if f.out.len() > rows && rows > 1 {
        let hidden = f.out.len() - (rows - 1);
        f.out.truncate(rows - 1);
        f.note(&format!("  +{hidden} more"));
    }
    f.out
}
