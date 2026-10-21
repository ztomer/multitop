//! The Ops card's CACHE: what the last gather found, kept on disk so that
//! the poll can run WITHOUT a terminal watching it.
//!
//! **WHY THE ANSWERS ARE KEPT RAW.** The cache holds the SERVER's own
//! answers - the JSON it published, verbatim - and the client parses them
//! when it draws. Not the parsed shape: the parsed shape is this client's
//! drawing of an answer, and a cache of drawings is a cache of opinions. The
//! raw answer is the fact, and the estate can change what a tool means
//! without this client changing what it means.
//!
//! **WHY THERE IS A CACHE AT ALL.** servers ROADMAP E36: the Ops poll was
//! bound to the Ops VIEW - it ran while a human was looking at the panel and
//! never otherwise - so the estate's fingerprint tools fired 0.3 a day while
//! the shared tools fired 87. The poll is a JOB, and a job that runs only
//! when somebody is watching is not a job. The poll runs on its own now
//! (the `multitop --ops-poll` pass), and what it finds is kept here so that
//! the VIEW can show the answer the poll found instead of asking the estate
//! a second time.
//!
//! **ONE WRITER'S WORDS, READ BY EVERYONE.** The due rule is the same rule
//! the routines client follows: a host is due when its last answer is older
//! than the poll's interval. Two pollers that both follow it cannot ask the
//! estate twice for the same answer - the second one finds the answer the
//! first one just kept - which is what keeps the client from paying the
//! estate twice for one card.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Snapshot;

/// One answer, as the server gave it.
///
/// NOT `PartialEq`: `serde_json::Value` is not `Eq` (a number can be a
/// float), and a half-comparable answer is worse than none - the cache's
/// own tests compare what was kept with what was read back as the TEXT the
/// cache holds, which is the thing that has to match.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Raw {
    /// The server answered, and the answer is here verbatim.
    Value(Value),
    /// The answer could not be read, and this is why.
    Failed(String),
    /// The host does not offer the tool, and this is the server's own reason.
    Absent(String),
}

impl Raw {
    /// The answer, when there is one.
    #[must_use]
    pub const fn value(&self) -> Option<&Value> {
        match self {
            Self::Value(v) => Some(v),
            Self::Failed(_) | Self::Absent(_) => None,
        }
    }
}

/// Everything one gather learned, in the server's own words.
///
/// The health section is TWO answers - the published verdict and the suite
/// it belongs to - so it is two fields here too, and they are read together
/// or not at all.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawSnapshot {
    /// `serverInfo` name and version.
    pub server: (String, String),
    /// When it was asked, epoch seconds.
    pub at: i64,
    /// The rate this client KEEPS: the cadence the server published,
    /// halved (servers ROADMAP E53). The rule, and the rate the next
    /// gather is due at.
    pub rule_every_s: i64,
    /// The rate this answer was actually taken at: the rule, raised to the
    /// gap since the last answer when the client missed its schedule.
    ///
    /// The truth, and what the answer says: a client that slept through a
    /// cycle says so, rather than carrying a rate it did not keep.
    pub sampled_every_s: i64,
    /// Where the TOOL LIST came from: the server, or the kept list.
    ///
    /// The list is the panel's shape, so where it came from travels with
    /// the answer the panel draws - a panel that cannot say whether its
    /// shape is today's is a panel that could be showing yesterday's tools
    /// (servers ROADMAP E61).
    pub catalogue: crate::mcp::catalogue::Catalogue,
    pub health: Raw,
    pub health_list: Raw,
    pub vpn: Raw,
    pub cron: Raw,
    pub containers: Raw,
    pub zfs: Raw,
    pub disk: Raw,
    pub facts: Raw,
    pub alerts: Raw,
}

impl RawSnapshot {
    /// The cadence the server published with the verdict, in seconds.
    ///
    /// The server's own number, read from the answer it gave (servers
    /// ROADMAP E53): the client's rate is derived from it, so a server
    /// that publishes faster than its clients read is visible here.
    #[must_use]
    pub fn cadence_s(&self) -> Option<i64> {
        self.health
            .value()?
            .get("cadence_s")
            .and_then(Value::as_i64)
    }

    /// The rate this client samples this host at, from what this answer
    /// says: half the cadence the server published, or the rate this
    /// client was keeping.
    ///
    /// The RULE, not the gap a missed cycle raised it to: a missed cycle
    /// is not a new rate, and a client that missed one must not keep
    /// sampling at half the rate for ever.
    #[must_use]
    pub fn interval(&self) -> Duration {
        super::rate::interval_for(self.cadence_s(), Some(self.rule_every_s))
    }

    /// What this client knew before the next gather: when this answer was
    /// taken, and the rate this client was keeping.
    #[must_use]
    pub const fn previous(&self) -> super::rate::Previous {
        super::rate::Previous {
            at: Some(self.at),
            rate_s: Some(self.rule_every_s),
        }
    }
}

/// The host's key in the cache: `user@host:port`, the same shape the state
/// file uses, so one host is one key everywhere.
#[must_use]
pub fn host_key(server: &crate::config::Server) -> String {
    let user = if server.user.is_empty() {
        "default"
    } else {
        server.user.as_str()
    };
    format!("{user}@{}:{}", server.host, server.port)
}

/// The file name a host's answers live under.
///
/// The key holds a colon, which is legal in a file name on macOS but is not
/// legal everywhere the bytes might travel, and it reads badly in a shell.
/// Every character that is not a letter, a digit, a dot, a dash or an `@`
/// becomes an underscore; two different keys cannot collide, because the
/// characters that would have to differ are the ones that are replaced -
/// and the key itself is in the file, so a collision would be visible.
#[must_use]
pub fn file_name(key: &str) -> String {
    format!("{}.json", sanitised(key))
}

/// The file name a host's TOOL LIST lives under.
///
/// The same key, sanitised the same way, so the file of one host cannot
/// collide with another's.
#[must_use]
pub fn catalogue_file_name(key: &str) -> String {
    format!("{}.catalogue.json", sanitised(key))
}

/// The directory, inside the cache directory, that holds the TOOL LISTS.
///
/// **A DIRECTORY, NOT A SECOND FILE BESIDE EACH ANSWER.** The routines
/// client reads this cache directory to find what the monitor knows about a
/// host, and it takes the FIRST file it finds whose name holds the host -
/// so a second file beside the answer, named for the same host, would be
/// taken for the answer itself, and the parity check would read a tool list
/// where it expects the estate's own answers.
///
/// A directory is out of that reader's way: it does not descend into it,
/// and a directory's name is not a file name ending in `.json`. The lists
/// therefore live where that reader never looks (servers ROADMAP E61).
pub const CATALOGUE_DIR: &str = "catalogue";
/// The key with every character that is not safe in a file name replaced.
fn sanitised(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '@' | '_') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

/// Where the cached answers live, and what is in them.
///
/// The cache is beside the config file: the config is the operator's, the
/// answers are about the estate, and the estate's answers are as private as
/// the config's directory. A cache with nowhere to live keeps nothing - a
/// client with no config has no estate to poll either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cache {
    dir: Option<PathBuf>,
}

impl Cache {
    /// The cache beside `config_path`.
    #[must_use]
    pub fn beside(config_path: &Path) -> Self {
        let dir = config_path
            .parent()
            .map_or_else(|| PathBuf::from("ops"), |p| p.join("ops"));
        Self { dir: Some(dir) }
    }

    /// A cache that keeps nothing: there is nowhere to keep it.
    #[must_use]
    pub const fn nowhere() -> Self {
        Self { dir: None }
    }

    /// Where one host's answers live.
    #[must_use]
    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(file_name(key)))
    }

    /// Where one host's TOOL LIST lives.
    ///
    /// The list is kept in its own directory inside the cache directory:
    /// they are two halves of what this client knows about one machine, and
    /// a host that moved has moved both - but the list is not an answer,
    /// and it must not be mistaken for one (see [`CATALOGUE_DIR`]).
    #[must_use]
    pub fn catalogue_path(&self, key: &str) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(CATALOGUE_DIR).join(catalogue_file_name(key)))
    }

    /// The store that keeps one host's tool list.
    ///
    /// servers ROADMAP E61: the list is the biggest thing this client pulls
    /// off a host, and it changes at a RELEASE - so it is kept, keyed to the
    /// server's advertised version and to the set of tools it says it does
    /// not offer, and bounded by the server's own `ttlMs`.
    #[must_use]
    pub fn catalogue_store(&self, key: &str) -> crate::mcp::catalogue::Store {
        crate::mcp::catalogue::Store::at(self.catalogue_path(key))
    }

    /// What the last gather found for `key`, if it is still there.
    ///
    /// A file that cannot be read - truncated by a crash, written by an
    /// older build - reads as NOTHING rather than as an error: the answer
    /// is gone, and the poll will gather it again. A cache that refuses to
    /// be read would leave the card blank for ever.
    #[must_use]
    pub fn read(&self, key: &str) -> Option<RawSnapshot> {
        let path = self.path(key)?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Keep what the last gather found for `key`.
    ///
    /// An answer that is OLDER than the one already kept is not kept: the
    /// poll writes what it found, and a write that lands late must not
    /// replace a newer answer with an older one.
    ///
    /// # Errors
    ///
    /// The file could not be written.
    pub fn write(&self, key: &str, raw: &RawSnapshot) -> Result<(), String> {
        let Some(path) = self.path(key) else {
            return Err("there is nowhere to keep the answer".into());
        };
        if let Some(kept) = self.read(key) {
            if kept.at > raw.at {
                return Ok(());
            }
        }
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(raw).map_err(|e| e.to_string())?;
        crate::state::write_atomic(&path, &text)
    }

    /// The answer for `key`, if it is younger than `every`.
    ///
    /// Fresh means: younger than the poll's own interval. An answer older
    /// than that is not an answer about the machine any more - the estate
    /// has published since - so it is not shown as one.
    #[must_use]
    pub fn fresh(&self, key: &str, now: i64, every: Duration) -> Option<RawSnapshot> {
        let raw = self.read(key)?;
        is_fresh(&raw, now, every).then_some(raw)
    }
}

/// Whether an answer is young enough to be shown as the machine's answer.
#[must_use]
pub fn is_fresh(raw: &RawSnapshot, now: i64, every: Duration) -> bool {
    let age = now.saturating_sub(raw.at);
    age < crate::tasks::ops_poll::interval_s(every)
}

/// How much EARLIER than the poll's interval a host may be gathered.
///
/// The scheduler that runs the pass ticks on its own clock, and launchd's
/// timer wheel has a granularity of its own (measured on this Mac on
/// 2026-10-21: a 420 s `StartInterval` fired 425 to 470 s after the last
/// run). A tick that lands a second early must not SKIP the gather: the
/// pass would then wait a whole extra cycle, and the client's read rate
/// would halve without anything saying so. The slack is a minute and a
/// half, which is more than the wheel's own spread - so every tick of the
/// scheduler's own clock finds a gather DUE.
///
/// The slack is only ever taken OFF the interval - the client reads MORE
/// often than the rule asks, never less - so it cannot weaken the rule it
/// serves (DECISIONS D15: a client must read at least twice as often as the
/// estate publishes).
pub const DUE_SLACK_S: i64 = 90;

/// Whether a host is DUE: it has no answer, or its answer is older than the
/// poll's interval.
///
/// The same rule the routines client follows (servers ROADMAP E12): the
/// client reads the estate's published verdict, and it reads it twice as
/// often as the estate publishes it. A host that was read inside the
/// interval is not read again - not by this client, and not by the other
/// poller of the same client, which is what keeps one card from being paid
/// for twice.
#[must_use]
pub fn due(raw: Option<&RawSnapshot>, now: i64, every: Duration) -> bool {
    let threshold = crate::tasks::ops_poll::interval_s(every) - DUE_SLACK_S;
    raw.is_none_or(|r| now.saturating_sub(r.at) >= threshold)
}

/// The answer as the view draws it: the raw answers, parsed.
#[must_use]
pub fn to_snapshot(raw: &RawSnapshot) -> Snapshot {
    super::parse_snapshot(raw)
}
