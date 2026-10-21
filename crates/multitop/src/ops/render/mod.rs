//! The Ops view's frame for one host: row 0 is the host banner, then one
//! line per section - a glyph, the section's name, what it says - and under
//! a section in trouble, one line per thing in trouble.
//!
//! The glyph carries the meaning and colour only reinforces it (a greyscale
//! frame reads the same): `✓` clean, `✗` the host reports a failure, `⚠` an
//! answer that could not be read - the estate's own report of a failure
//! ("failed: ...") or a fact this client does not have ("unknown: ...") -
//! or alerts (events, not a current fault), `·` a stated absence (no Docker
//! here), `→` asking.
//!
//! The section names are the ESTATE's words, the ones the routines card uses
//! as well: `health`, `cron`, `containers`, `zfs`, `vpn`, `disk`. A view
//! that says `zfs.health` where the card says `zfs`, or `docker` where the
//! card says `containers`, is a view that disagrees with the card about the
//! same machine in the same minute (servers ROADMAP O92).

pub mod health;
pub mod sections;

use multitop_agent::fmt::center_header;
use multitop_agent::palette_view::PaletteView;

use super::{Catalogue, Miss, OpsState, Part, Proof};

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
    span(now - ts)
}

/// "45s", "12m", "3h": how long a span of seconds is.
#[must_use]
pub fn span(d: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;
    let d = d.max(0);
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

/// The tool list's own line: where the list came from, and how old it is.
///
/// The list is the panel's shape, so a list nobody can date is a shape
/// nobody can trust: a panel that drew yesterday's tools would look exactly
/// like one that drew today's. So the line says which it is, and a kept
/// list that had to be put aside says why.
///
/// Its own line, not the server's: the server's line says when the ANSWER
/// was asked, and this says when the SHAPE was taken. They are different
/// moments, and one line that said both would be cut short on a narrow
/// panel - which is exactly where a reader would lose the shape's age.
fn catalogue_note(c: &Catalogue) -> String {
    match c {
        Catalogue::Listed { ttl_s } => {
            format!("tool list: from the server (window {})", span(*ttl_s))
        }
        Catalogue::Kept {
            age_s,
            ttl_s,
            proof,
        } => format!(
            "tool list: kept {} old (window {}), {}",
            span(*age_s),
            span(*ttl_s),
            proof_note(proof)
        ),
        Catalogue::Relisted { miss, .. } => {
            format!("tool list: re-listed, {}", miss_reason(miss))
        }
    }
}

/// How the kept list was checked, in words.
///
/// The distinction is the point (servers ROADMAP E72): the server's digest
/// MATCHING is the server's own word, and a digest that was ABSENT is a
/// fact this client could not check - which is not the same as saying the
/// copy is fine. So the line says which of the two it was.
fn proof_note(proof: &Proof) -> String {
    match proof {
        Proof::Digest { .. } => "the server's digest matches".to_string(),
        Proof::Absent => "the server published no digest".to_string(),
    }
}

/// Why a kept list was not used, in words.
fn miss_reason(miss: &Miss) -> String {
    match miss {
        Miss::None => "there was no kept list".to_string(),
        Miss::Released { kept, now } => {
            format!("the kept list was for {kept}, the server is {now}")
        }
        Miss::Reshaped { .. } => "the host's tool set changed".to_string(),
        Miss::Expired { age_s, ttl_s } => format!(
            "the kept list had expired ({} old, window {})",
            span(*age_s),
            span(*ttl_s)
        ),
        Miss::Changed { .. } => "the server's catalogue digest changed".to_string(),
        Miss::Uncheckable => "the kept list carried no digest to check against".to_string(),
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

/// A section that is not ready: its absence, its failure, or the gap in
/// the reading, on one line.
///
/// **THE THREE SAY DIFFERENT THINGS.** An ABSENCE is a stated status ("no
/// Docker here"); a FAILURE is the estate's own report ("the tool ran and
/// said it failed"); an UNKNOWN is the answer that did not arrive, so
/// this client knows nothing about the thing it asked (servers ROADMAP
/// E74). The last is a gap in the reading - never an all-clear, and
/// never a fault of the estate.
fn not_ready<T>(f: &mut Frame<'_>, label: &str, part: &Part<T>) -> bool {
    match part {
        Part::Ready(_) => false,
        Part::Absent(why) => {
            f.line(Tone::Quiet, label, why);
            true
        }
        Part::Failed(why) => {
            f.line(Tone::Warn, label, &format!("failed: {why}"));
            true
        }
        Part::Unknown(why) => {
            f.line(Tone::Warn, label, &format!("unknown: {why}"));
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
            f.note(&catalogue_note(&s.catalogue));
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
