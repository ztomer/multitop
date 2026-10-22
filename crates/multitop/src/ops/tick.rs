//! The AGENT'S TICK: how long the installed agent waits between passes,
//! derived from the cadence the estate publishes.
//!
//! servers ROADMAP E53: the installed agent's tick was a number this repo
//! carried (`StartInterval 390` in the install template), and launchd fixes
//! an interval when the job is loaded - so the tick could not follow the
//! estate. Had the publication moved from every 15 minutes to every 5, the
//! agent would still have read every 6.5 minutes: slower than the
//! publication, so its verdict could be two publications behind, and the
//! client's own warning would have fired on every tick with nothing to do
//! about it but reinstall the agent.
//!
//! So the tick is READ, like every other rate in this client (DECISIONS
//! D15): the estate publishes its cadence with its verdict
//! (`health://latest`, beside the age), and the agent derives its rate from
//! what it read - half the cadence, the rule, less a small margin so the
//! tick's own jitter can never land a second past the rule.
//!
//! **THE TICK IS THE FASTEST RATE ANY HOST NEEDS.** One agent reads every
//! host, so one tick has to serve them all: the SHORTEST cadence any host
//! published, halved. A host that publishes faster is read faster; a host
//! that publishes slower is read at the pace its own cadence allows - the
//! pass's due rule (see [`crate::ops::cache::due`]) leaves it alone until
//! its own answer is old enough to be worth asking for.
//!
//! **A CADENCE THAT COULD NOT BE READ IS STATED, NEVER ASSUMED.** A host
//! that published no cadence keeps the rate this client was keeping for it,
//! and the log says so; a host this client has never read keeps the
//! client's own fallback rate, and the log says that too. Neither is
//! called the published rate, because neither was published.

use super::cache::{self, Cache, RawSnapshot};
use super::rate;

/// The margin the tick keeps under the rule, in seconds.
///
/// The rule is this client's own knife-edge: an answer carries the rate it
/// was taken at, and that rate is raised to the gap since the last answer -
/// so a tick one second past the rule would make every answer say it was
/// sampled slower than the rate it kept, and the panel would warn about a
/// client that is exactly on time. A few seconds is enough for the tick's
/// own jitter (the wait is a sleep, and the pass takes time of its own) and
/// small enough that the tick is still the rule.
pub const TICK_MARGIN_S: i64 = 5;

/// The shortest wait the agent will take, in seconds.
///
/// The estate's published cadence is the only thing that sets the tick.
/// This says only that a tick of zero is a busy loop.
pub const MIN_TICK_S: i64 = 1;

/// Where a rate came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The cadence the estate published with its verdict.
    Published {
        /// The cadence, in seconds.
        cadence_s: i64,
    },
    /// The cadence could not be read: the rate this host was keeping.
    Kept {
        /// The rate this client was keeping, in seconds.
        rate_s: i64,
    },
    /// No answer has ever been read for this host: the client's fallback.
    Fallback {
        /// The client's own fallback rate, in seconds.
        rate_s: i64,
    },
}

/// One host's rate, and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRate {
    /// The host's address.
    pub host: String,
    /// The rate this client keeps for it, in seconds.
    pub rate_s: i64,
    /// Where that rate came from.
    pub source: Source,
    /// When this host's answer was read, in epoch seconds.
    pub at: Option<i64>,
}

impl HostRate {
    /// What this host's rate is called, in the operator's words.
    ///
    /// The wording is the client's own (see [`rate::cadence_unknown_note`]):
    /// a cadence that could not be read is said in words, and never given
    /// the number a published cadence would have.
    #[must_use]
    pub fn note(&self) -> String {
        match self.source {
            Source::Published { cadence_s } => {
                format!("the estate publishes {}", rate::cadence_words(cadence_s))
            }
            Source::Kept { rate_s } => rate::cadence_unknown_note(rate_s),
            Source::Fallback { rate_s } => format!(
                "{}, and this host has never been read",
                rate::cadence_unknown_note(rate_s)
            ),
        }
    }
}

/// The agent's tick: the period it keeps, and where that period came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tick {
    /// The period between passes, in seconds.
    pub secs: i64,
    /// The rate the period was derived from: the rule.
    pub rate_s: i64,
    /// The host whose rate set the tick: the fastest any host needs.
    pub host: String,
    /// Where that rate came from.
    pub source: Source,
    /// When that host's answer was read, in epoch seconds.
    pub at: Option<i64>,
    /// The hosts whose cadence could not be read.
    pub unread: Vec<HostRate>,
}

impl Tick {
    /// The line the agent's log carries about the tick.
    ///
    /// A number in a log that names nothing cannot be checked, so the line
    /// says what the estate publishes, where it was read from, and how old
    /// that reading is.
    #[must_use]
    pub fn line(&self, now: i64) -> String {
        let head = format!("tick {} ({} s)", rate::rate_words(self.secs), self.secs);
        match self.source {
            Source::Published { cadence_s } => format!(
                "{head} · the shortest cadence any host published: {} ({} s), read from {} on {}{}",
                rate::cadence_words(cadence_s),
                cadence_s,
                super::HEALTH_URI,
                self.host,
                self.read_at(now),
            ),
            Source::Kept { rate_s } => format!(
                "{head} · {}, read from {} on {}{}",
                rate::cadence_unknown_note(rate_s),
                super::HEALTH_URI,
                self.host,
                self.read_at(now),
            ),
            Source::Fallback { rate_s } if self.host.is_empty() => format!(
                "{head} · {}, and no host has ever been read",
                rate::cadence_unknown_note(rate_s),
            ),
            Source::Fallback { rate_s } => format!(
                "{head} · {}, and this host has never been read",
                rate::cadence_unknown_note(rate_s),
            ),
        }
    }

    /// How long ago the answer that carried the cadence was read.
    fn read_at(&self, now: i64) -> String {
        self.at.map_or_else(String::new, |at| {
            format!(", {}", ago(now.saturating_sub(at)))
        })
    }
}

/// The rate for one host, and where it came from.
///
/// The cadence the estate published is the rule's source (half of it). A
/// host that published none keeps the rate this client was keeping for it,
/// and a host this client has never read keeps the client's own fallback.
#[must_use]
pub fn rate_of(host: &str, raw: Option<&RawSnapshot>) -> HostRate {
    let Some(raw) = raw else {
        return HostRate {
            host: host.to_string(),
            rate_s: rate::FALLBACK_INTERVAL_S,
            source: Source::Fallback {
                rate_s: rate::FALLBACK_INTERVAL_S,
            },
            at: None,
        };
    };
    raw.cadence_s().map_or_else(
        || HostRate {
            host: host.to_string(),
            rate_s: raw.rule_every_s,
            source: Source::Kept {
                rate_s: raw.rule_every_s,
            },
            at: Some(raw.at),
        },
        |cadence_s| HostRate {
            host: host.to_string(),
            rate_s: rate::rate_for(cadence_s),
            source: Source::Published { cadence_s },
            at: Some(raw.at),
        },
    )
}

/// The tick for these hosts: the fastest rate any of them needs.
///
/// The shortest cadence any host published, halved (the rule), less the
/// margin - and never faster than the shortest wait.
#[must_use]
pub fn tick_of(rates: &[HostRate]) -> Tick {
    let mut fastest: Option<&HostRate> = None;
    let mut unread: Vec<HostRate> = Vec::new();
    for rate in rates {
        if !matches!(rate.source, Source::Published { .. }) {
            unread.push(rate.clone());
        }
        fastest = Some(fastest.map_or(rate, |best| faster(rate, best)));
    }
    match fastest {
        Some(fastest) => Tick {
            secs: (fastest.rate_s - TICK_MARGIN_S).max(MIN_TICK_S),
            rate_s: fastest.rate_s,
            host: fastest.host.clone(),
            source: fastest.source,
            at: fastest.at,
            unread,
        },
        None => Tick {
            secs: (rate::FALLBACK_INTERVAL_S - TICK_MARGIN_S).max(MIN_TICK_S),
            rate_s: rate::FALLBACK_INTERVAL_S,
            host: String::new(),
            source: Source::Fallback {
                rate_s: rate::FALLBACK_INTERVAL_S,
            },
            at: None,
            unread,
        },
    }
}

/// Which of two rates sets the tick.
///
/// The faster one, and on a tie the one the estate PUBLISHED: a published
/// cadence says where it came from, and a kept rate does not - so a tie
/// must not be settled by a host that merely kept a rate.
const fn faster<'a>(a: &'a HostRate, b: &'a HostRate) -> &'a HostRate {
    if a.rate_s < b.rate_s {
        return a;
    }
    if b.rate_s < a.rate_s {
        return b;
    }
    if published(b) || !published(a) {
        b
    } else {
        a
    }
}

/// Whether this rate came from a cadence the estate published.
const fn published(rate: &HostRate) -> bool {
    matches!(rate.source, Source::Published { .. })
}

/// The agent's tick, from what this client has read.
///
/// The cache holds the answers the last pass kept, and the cadence travels
/// with them (servers ROADMAP E53) - so the tick is read from the estate's
/// own published cadence, and this client carries no rate of its own.
#[must_use]
pub fn tick_for(servers: &[crate::config::Server], cache: &Cache) -> Tick {
    let rates: Vec<HostRate> = servers
        .iter()
        .map(|server| {
            let key = cache::host_key(server);
            let raw = cache.read(&key);
            rate_of(&server.host, raw.as_ref())
        })
        .collect();
    tick_of(&rates)
}

/// How long ago, in the operator's words: "12 s ago", "7 min ago", "2 h ago".
fn ago(secs: i64) -> String {
    if secs < rate::SECS_A_MINUTE {
        format!("{secs} s ago")
    } else if secs < rate::SECS_AN_HOUR {
        format!("{} min ago", secs / rate::SECS_A_MINUTE)
    } else if secs < rate::SECS_A_DAY {
        format!("{} h ago", secs / rate::SECS_AN_HOUR)
    } else {
        format!("{} days ago", secs / rate::SECS_A_DAY)
    }
}
