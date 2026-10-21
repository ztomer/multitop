//! The machine's health section: the PUBLISHED verdict, and how old it is.
//!
//! Split from `sections.rs` for the house's line cap, along the seam that
//! was already there: the health verdict is its own fact, with its own age
//! and its own rules, and the section that drew it grew with them (the
//! age, the staleness, the client's own sampling rate).
//!
//! The verdict is read, not run (servers ROADMAP E12), so it is as old as
//! the estate's last suite run - and the age is on the line, because a
//! verdict that cannot be dated is a claim nobody can check. A verdict that
//! is too old says so; a machine that published none says so; neither is
//! ever an all-clear.

use super::super::model::Check;
use super::super::model::Health;
use super::{count, Frame, Tone};

/// The machine's health: the PUBLISHED verdict, and how old it is.
///
/// `sampled_every_s` is the rate THIS answer was sampled at, and it is
/// carried by the answer: a client that samples slower than half the
/// PUBLISHED cadence says so here, where the verdict is shown, because
/// the age on the line cannot say it - a verdict two publications behind
/// looks exactly like one that is fresh (DECISIONS D15).
///
/// **THE WARNING GETS A LINE OF ITS OWN.** Appended to the summary, the
/// panel's width cut the meaning off the end of it: "sampled every 15 min:
/// the verdict can be" - a sentence with the point taken off the end, which
/// says nothing about what the verdict can be behind. Under the section it
/// is said whole, and the section above it is not green.
pub(super) fn health(f: &mut Frame<'_>, health: &Health, _now: i64, sampled_every_s: i64) {
    // The client's own sampling: too slow for the cadence the server
    // PUBLISHED, and the verdict can be two publications behind - which no
    // age on the line can say.
    let slow = super::super::sampling_warning(sampled_every_s, &health.cadence);
    // A cadence the server could not read: the rate could not be read,
    // and this client keeps the rate it had. That is a gap in the
    // reading, not a fault - and it is said, not left in silence.
    let unknown = if health.cadence.is_known() {
        None
    } else {
        Some(super::super::cadence_unknown_note(sampled_every_s))
    };
    let mut caveats: Vec<&str> = slow.iter().map(String::as_str).collect();
    caveats.extend(unknown.iter().map(String::as_str));
    if health.checks.is_empty() {
        f.line(
            Tone::Warn,
            "health",
            "no verdict published: nothing has run on this machine",
        );
        for w in caveats {
            f.item(w);
        }
        return;
    }
    let age = health.oldest_age.as_deref().unwrap_or("of unknown age");
    let bad: Vec<&Check> = health.checks.iter().filter(|c| !c.ok).collect();
    if bad.is_empty() {
        let stale = health.is_stale();
        let detail = if stale {
            format!("{} ok · {age} · stale", count(health.checks.len(), "check"))
        } else {
            format!("{} ok · {age}", count(health.checks.len(), "check"))
        };
        f.line(
            if stale || slow.is_some() {
                Tone::Warn
            } else {
                Tone::Good
            },
            "health",
            &detail,
        );
        for w in caveats {
            f.item(w);
        }
        return;
    }
    f.line(
        Tone::Bad,
        "health",
        &format!("{} of {} failing · {age}", bad.len(), health.checks.len()),
    );
    for c in bad {
        f.item(&format!(
            "{}  rc {}, {}",
            check_name(&c.name),
            c.rc,
            c.age.as_deref().unwrap_or("of unknown age")
        ));
    }
    for w in caveats {
        f.item(w);
    }
}

fn check_name(name: &str) -> &str {
    name.strip_prefix("native:").unwrap_or(name)
}
