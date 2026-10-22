//! The client's SAMPLING RATE: derived from the cadence the server
//! publishes, and never carried as a number of this client's own.
//!
//! servers ROADMAP E53, and DECISIONS D15 (owner, 2026-10-20): a client
//! that samples at the rate the estate publishes can ALIAS - its read
//! lands just before a publication and then waits a whole cycle, and its
//! verdict is two publications behind the truth. So a client samples at
//! TWICE the publication rate.
//!
//! The publication rate used to be a number COPIED into this client
//! (450 s - half of a fifteen-minute publication), which is why a server
//! that published every five minutes would not have been noticed here at
//! all: neither this client's rate nor its staleness threshold moved, so
//! the aliasing the ruling forbids would have come back silently. The
//! cadence now travels WITH the verdict (`health://latest`, beside the
//! age), and everything here is derived from what was published.
//!
//! Four facts, and they are four different things:
//!
//! - the RATE: half the published cadence. The rule (D15), read from the
//!   estate rather than copied into this client.
//! - the CARRIED rate: what an answer says it was taken at. The rule's
//!   number, raised to the gap since the last answer when the client
//!   missed its schedule - a client that slept through a cycle says so
//!   rather than carrying a rate it did not keep.
//! - the WARNING: the carried rate is slower than half the PUBLISHED
//!   cadence, so the verdict can be two publications behind. The age on
//!   the line cannot say it: a verdict two publications behind looks
//!   exactly like a fresh one.
//! - the UNKNOWN: the server could not read its own cadence. That is a
//!   gap in the reading, not a fault: the client keeps the rate it had,
//!   and says the rate could not be read.

use std::time::Duration;

use super::model_health::Cadence;

/// How long past the published cadence a verdict may be before this
/// client calls it STALE: a grace for a suite that is still running.
pub const STALE_GRACE_S: i64 = 300;

/// The rate this client samples at when it has nothing to go on: half of
/// the cadence the estate published last time this client saw one.
///
/// A FALLBACK, not the rule. The rule is derived from the cadence the
/// server publishes with the verdict; this is the rate for a host this
/// client has never read, whose server could not read its own cadence.
pub const FALLBACK_INTERVAL_S: i64 = 450;

/// A minute, in seconds: the unit a rate and a cadence are read in.
pub(crate) const SECS_A_MINUTE: i64 = 60;
/// An hour, in seconds.
pub(crate) const SECS_AN_HOUR: i64 = 3_600;
/// A day, in seconds.
pub(crate) const SECS_A_DAY: i64 = 86_400;

/// What this client knew before a gather: when the last answer was taken,
/// and at what rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Previous {
    /// When the last answer was gathered.
    pub at: Option<i64>,
    /// The rate the last answer was taken at.
    pub rate_s: Option<i64>,
}

/// The rate for a cadence in seconds: HALF of it.
///
/// The cadence itself is an Option at the call sites: a server that
/// published no cadence has no rate to derive, and the client keeps the
/// one it had.
#[must_use]
pub fn rate_for(cadence_s: i64) -> i64 {
    (cadence_s / 2).max(1)
}

/// The rate this client samples at: the published cadence's half, or the
/// rate it had, or the fallback.
#[must_use]
pub fn interval_for(cadence_s: Option<i64>, kept_s: Option<i64>) -> Duration {
    let secs = cadence_s
        .map(rate_for)
        .or(kept_s)
        .unwrap_or(FALLBACK_INTERVAL_S)
        .max(1);
    Duration::from_secs(u64::try_from(secs).unwrap_or(u64::MAX))
}

/// The rate an answer carries: the rule's number, unless the client
/// missed its schedule - then the gap since the last answer, which is
/// the truth.
#[must_use]
pub fn carried_rate(configured: i64, previous_at: Option<i64>, now: i64) -> i64 {
    previous_at.map_or(configured, |at| configured.max(now.saturating_sub(at)))
}

/// The warning: this client's rate is slower than half the PUBLISHED
/// cadence, so its verdict can be two publications behind.
///
/// A cadence this client could not read is not a warning: it cannot tell
/// whether it is keeping up, and a gap in the reading is not a fault.
///
/// The wording is kept inside the panel's width: a warning whose point is
/// cut off the end of the line says nothing (servers ROADMAP E12's own
/// note on the age).
#[must_use]
pub fn sampling_warning(rate_s: i64, cadence: &Cadence) -> Option<String> {
    let cadence_s = cadence.secs?;
    if rate_s.saturating_mul(2) <= cadence_s {
        return None;
    }
    Some(format!(
        "sampled {}, server publishes {}: two publications behind",
        rate_words(rate_s),
        cadence.say()
    ))
}

/// The note an answer carries when the server could not read its own
/// cadence: the rate could not be read, and this client kept the rate it
/// had.
#[must_use]
pub fn cadence_unknown_note(rate_s: i64) -> String {
    format!(
        "the cadence could not be read: this client keeps its rate ({})",
        rate_words(rate_s)
    )
}

/// A rate in words: "every 15 min", "every 7.5 min", "every 30 s".
#[must_use]
pub fn rate_words(secs: i64) -> String {
    if secs < SECS_A_MINUTE {
        format!("every {secs} s")
    } else if secs % SECS_A_MINUTE == 0 {
        format!("every {} min", secs / SECS_A_MINUTE)
    } else {
        format!("every {} min", tenths_of_a_minute(secs))
    }
}

/// The minutes, with one decimal: 450 s is seven and a half minutes, and
/// "7 min" would understate the rate by half a minute.
fn tenths_of_a_minute(secs: i64) -> String {
    let tenths = secs.saturating_mul(10) / SECS_A_MINUTE;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// A cadence in words: "every 15 min", "every 4 h", "every day".
///
/// The server's own shape (its `cadence` field says the same words), so
/// a client that has to read the number itself says what the server
/// would have said.
#[must_use]
pub fn cadence_words(secs: i64) -> String {
    if secs < SECS_A_MINUTE {
        format!("every {secs} s")
    } else if secs < SECS_AN_HOUR {
        format!("every {} min", secs / SECS_A_MINUTE)
    } else if secs < SECS_A_DAY {
        format!("every {} h", secs / SECS_AN_HOUR)
    } else {
        format!("every {} days", secs / SECS_A_DAY)
    }
}
