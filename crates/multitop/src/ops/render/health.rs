//! The machine's health section: the PUBLISHED verdict, and how old it is.
//!
//! Split from `sections.rs` for the house's line cap, along the seam that
//! was already there: the health verdict is its own fact, with its own age
//! and its own rules, and the section that draws it grew with them (the
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
pub(super) fn health(f: &mut Frame<'_>, health: &Health, _now: i64) {
    if health.checks.is_empty() {
        f.line(
            Tone::Warn,
            "health",
            "no verdict published: nothing has run on this machine",
        );
        return;
    }
    let age = health.oldest_age.as_deref().unwrap_or("of unknown age");
    // The client's own sampling: too slow, and the verdict can be two
    // publications behind - which no age on the line can say.
    let slow = super::super::sampling_warning(
        crate::tasks::ops_poll::OPS_POLL
            .as_secs()
            .try_into()
            .unwrap_or(i64::MAX),
    );
    let bad: Vec<&Check> = health.checks.iter().filter(|c| !c.ok).collect();
    if bad.is_empty() {
        let stale = health.is_stale();
        let mut detail = if stale {
            format!("{} ok · {age} · stale", count(health.checks.len(), "check"))
        } else {
            format!("{} ok · {age}", count(health.checks.len(), "check"))
        };
        if let Some(w) = &slow {
            detail.push_str(" · ");
            detail.push_str(w);
        }
        f.line(
            if stale || slow.is_some() {
                Tone::Warn
            } else {
                Tone::Good
            },
            "health",
            &detail,
        );
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
}

fn check_name(name: &str) -> &str {
    name.strip_prefix("native:").unwrap_or(name)
}
