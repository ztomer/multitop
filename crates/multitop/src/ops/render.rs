//! The Ops view's frame for one host: row 0 is the host banner, then one
//! line per section - a glyph, the section's name, what it says - and under
//! a section in trouble, one line per thing in trouble.
//!
//! The glyph carries the meaning and colour only reinforces it (a greyscale
//! frame reads the same): `✓` clean, `✗` the host reports a failure, `⚠` an
//! answer that could not be read, or alerts (events, not a current fault),
//! `·` a stated absence (no Docker here), `→` asking.

use multitop_agent::color::Palette;
use multitop_agent::fmt::center_header;

use super::{Alert, Check, Container, Job, OpsState, Part, Snapshot};

/// Section names are padded to this, so the summaries line up.
const LABEL_W: usize = 7;
/// Items under a section are indented this far.
const ITEM_INDENT: &str = "    ";

#[derive(Clone, Copy)]
enum Tone {
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

    const fn color(self, pal: &Palette) -> &'static str {
        match self {
            Self::Good => pal.green,
            Self::Bad => pal.red,
            Self::Warn => pal.yellow,
            Self::Quiet => pal.gray,
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

struct Frame<'a> {
    out: Vec<String>,
    cols: usize,
    pal: &'a Palette,
}

impl Frame<'_> {
    fn line(&mut self, tone: Tone, label: &str, text: &str) {
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

    fn item(&mut self, text: &str) {
        let body = clip(text, self.cols.saturating_sub(ITEM_INDENT.len()));
        self.out.push(format!("{ITEM_INDENT}{body}"));
    }

    fn note(&mut self, text: &str) {
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

fn check_name(name: &str) -> &str {
    name.strip_prefix("native:").unwrap_or(name)
}

fn health(f: &mut Frame<'_>, checks: &[Check], now: i64) {
    let bad: Vec<&Check> = checks.iter().filter(|c| !c.ok).collect();
    if bad.is_empty() {
        f.line(
            Tone::Good,
            "health",
            &format!("{} ok", count(checks.len(), "check")),
        );
        return;
    }
    f.line(
        Tone::Bad,
        "health",
        &format!("{} of {} failing", bad.len(), checks.len()),
    );
    for c in bad {
        f.item(&format!(
            "{}  rc {}, {} ago",
            check_name(&c.name),
            c.rc,
            age(c.ts, now)
        ));
    }
}

fn cron(f: &mut Frame<'_>, jobs: &[Job], now: i64) {
    let bad: Vec<&Job> = jobs.iter().filter(|j| j.in_trouble()).collect();
    let running: Vec<&str> = jobs
        .iter()
        .filter(|j| j.running_since.is_some())
        .map(|j| j.label.as_str())
        .collect();
    let busy = if running.is_empty() {
        String::new()
    } else {
        format!(", running: {}", running.join(" "))
    };
    if bad.is_empty() {
        f.line(
            Tone::Good,
            "cron",
            &format!("{} ok{busy}", count(jobs.len(), "job")),
        );
        return;
    }
    f.line(
        Tone::Bad,
        "cron",
        &format!("{} of {} failing{busy}", bad.len(), jobs.len()),
    );
    for j in bad {
        let mut why = Vec::new();
        if let Some(rc) = j.last_rc.filter(|rc| *rc != 0) {
            let when = j
                .last_start
                .map(|t| format!(", {} ago", age(t, now)))
                .unwrap_or_default();
            why.push(format!("rc {rc}{when}"));
        }
        if let Some(t) = j.alert_since {
            why.push(format!("alerting {}", age(t, now)));
        }
        f.item(&format!("{}  {}", j.label, why.join(", ")));
    }
}

fn containers(f: &mut Frame<'_>, cs: &[Container]) {
    let bad: Vec<&Container> = cs.iter().filter(|c| c.in_trouble()).collect();
    let running = cs.iter().filter(|c| c.state == "running").count();
    let healthy = cs.iter().filter(|c| c.health == "healthy").count();
    if bad.is_empty() {
        f.line(
            Tone::Good,
            "docker",
            &format!("{running} running, {healthy} healthy"),
        );
        return;
    }
    f.line(
        Tone::Bad,
        "docker",
        &format!("{} of {} in trouble", bad.len(), cs.len()),
    );
    for c in bad {
        let why = if c.health == "unhealthy" {
            "unhealthy"
        } else {
            c.status.as_str()
        };
        f.item(&format!("{}  {why}", c.name));
    }
}

fn alerts(f: &mut Frame<'_>, events: &[Alert], now: i64) {
    if events.is_empty() {
        f.line(Tone::Good, "alerts", "none in 24 h");
        return;
    }
    f.line(Tone::Warn, "alerts", &format!("{} in 24 h", events.len()));
    let mut newest: Vec<&Alert> = events.iter().collect();
    newest.sort_by_key(|a| std::cmp::Reverse(a.ts));
    for a in newest {
        f.item(&format!("{} ago  {}", age(a.ts, now), a.subject));
    }
}

fn sections(f: &mut Frame<'_>, s: &Snapshot, now: i64) {
    if !not_ready(f, "health", &s.health) {
        if let Part::Ready(c) = &s.health {
            health(f, c, now);
        }
    }
    if !not_ready(f, "cron", &s.cron) {
        if let Part::Ready(j) = &s.cron {
            cron(f, j, now);
        }
    }
    if !not_ready(f, "docker", &s.containers) {
        if let Part::Ready(c) = &s.containers {
            containers(f, c);
        }
    }
    if !not_ready(f, "alerts", &s.alerts) {
        if let Part::Ready(a) = &s.alerts {
            alerts(f, a, now);
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
    pal: &Palette,
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
                sections(&mut f, s, now);
            }
        }
        OpsState::Ready(s) => {
            f.note(&format!(
                "{} {} · asked {} ago",
                s.server.0,
                s.server.1,
                age(s.at, now)
            ));
            sections(&mut f, s, now);
        }
    }
    if f.out.len() > rows && rows > 1 {
        let hidden = f.out.len() - (rows - 1);
        f.out.truncate(rows - 1);
        f.note(&format!("  +{hidden} more"));
    }
    f.out
}
