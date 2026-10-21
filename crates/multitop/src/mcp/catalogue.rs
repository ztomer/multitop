//! The tool CATALOGUE: the list of tools a host offers, kept between
//! sessions so that a gather does not pull the whole list every time.
//!
//! servers ROADMAP E61: the Ops gather pulled the **whole catalogue** on
//! every gather - 65,467 B on `.33`, 51,716 B on `.90`, 32,724 B on `.158`,
//! measured - and the list is the SAME list every time: it changes at a
//! RELEASE, not every gather. At the estate's own sampling rate that is
//! the same catalogue, over and over, hundreds of times a day.
//!
//! **THE KEY IS WHAT MAKES THE CACHE SAFE.** The list a host offers is a
//! function of two things, and the client can see both of them cheaply:
//!
//!   * the SERVER's advertised version (`serverInfo.version`) - the binary
//!     that holds the tool table, and the only thing that can add a tool to
//!     it. A release bumps it.
//!   * the set of tools the server says it does NOT offer
//!     (`dev.multitop/absent` in `server/discover`) - the host's own state:
//!     Docker present or not, the tunnel up or not. A host that gains or
//!     loses a tool changes this set.
//!
//! So the kept list is re-checked against the LIVE server on every session:
//! `server/discover` is asked every time (it is a kilobyte), and a
//! different version, or a different absent set, puts the kept list aside
//! and the host is asked again. The cache therefore cannot hide a change -
//! it only saves the download of a list that has not changed.
//!
//! **AND THE SERVER'S OWN WORD BOUNDS IT.** `tools/list` answers with
//! `ttlMs` - an hour, for a list - and that is the answer's owner saying
//! how long the answer may be kept. A kept list that outlived the window
//! the server gave it would be caching something the server never
//! promised; this one does not.
//!
//! **THE CACHE IS AN OPTIMISATION, NEVER A FILTER.** What is kept is the
//! WHOLE list the server gave - every tool of it, every page of it, by
//! name, which is all this client reads of it - so a tool the cache omits
//! cannot vanish from the panel. The key is what makes that safe: a tool
//! can only be added or removed by a release (the version moves) or by the
//! host's own state (the absent set moves).
//!
//! **A STALE LIST IS SAID TO BE STALE.** The client says where the list
//! came from and how old it is, and a kept list past the server's window
//! is not used at all: the server is asked again, and the client says why.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A second in milliseconds: the unit `ttlMs` is in.
pub const MS_PER_S: u64 = 1_000;

/// The tool list, and where this session got it.
///
/// The state travels with the answer: the panel says it, so an operator
/// can see that the list the panel drew came from a kept copy and how old
/// that copy is. A list nobody can date is a list nobody can trust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Catalogue {
    /// The list was taken from the server, just now.
    Listed {
        /// How long the server said the list may be kept, in seconds.
        ttl_s: i64,
    },
    /// The kept list was used: it IS the list the server is offering now.
    Kept {
        /// How old the kept list is, in seconds.
        age_s: i64,
        /// How long the server said the list may be kept, in seconds.
        ttl_s: i64,
    },
    /// The kept list was put aside, and the server was asked again.
    Relisted {
        /// Why the kept list was not used.
        miss: Miss,
        /// How long the server said the list may be kept, in seconds.
        ttl_s: i64,
    },
}

/// Why a kept list was not used.
///
/// Named, because the client says so: "the kept list was for another
/// binary" and "the kept list had expired" are different facts, and an
/// operator reading one of them can act on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Miss {
    /// There is no kept list: this client has never listed this host.
    None,
    /// The server's version moved: a RELEASE.
    Released {
        /// The version the kept list was taken under.
        kept: String,
        /// The version the server advertises now.
        now: String,
    },
    /// The host's own tool set moved: a tool was gained or lost.
    Reshaped {
        /// The absent set the kept list was taken under.
        kept: String,
        /// The absent set the server publishes now.
        now: String,
    },
    /// The kept list is past the window the server gave it.
    Expired {
        /// How old the kept list is, in seconds.
        age_s: i64,
        /// The window the server gave it, in seconds.
        ttl_s: i64,
    },
}

/// One host's list, as it is kept on disk.
///
/// The wire's own names, so the file reads like the answer it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The server's advertised version when the list was taken.
    pub version: String,
    /// The absent set when the list was taken: the other half of the key.
    pub absent: String,
    /// The tools themselves: EVERY tool the server listed.
    pub tools: Vec<String>,
    /// How long the SERVER said this answer may be kept (`ttlMs`).
    pub ttl_ms: u64,
    /// When it was taken (epoch seconds).
    pub fetched_at: i64,
}

impl Entry {
    /// Whether this entry is the list the live server is offering now.
    ///
    /// Both halves of the key: the version (the binary) and the absent set
    /// (the host's own state).
    #[must_use]
    pub fn is_for(&self, version: &str, absent: &str) -> bool {
        self.version == version && self.absent == absent
    }

    /// Whether the server's own window has not closed.
    ///
    /// A window of zero is the protocol's "a live fact": there is nothing to
    /// keep, so there is nothing to use.
    #[must_use]
    pub fn is_fresh(&self, now: i64) -> bool {
        self.ttl_ms > 0 && now.saturating_sub(self.fetched_at) < self.window_s()
    }

    /// The window the server gave, in seconds.
    #[must_use]
    pub fn window_s(&self) -> i64 {
        i64::try_from(self.ttl_ms / MS_PER_S).unwrap_or(i64::MAX)
    }
}

/// The absent set as one string: the names, sorted, joined.
///
/// Sorted, because the set is a SET and the order the server published it
/// in is not part of what it says. The REASONS are not part of the key
/// either: they are the server's own words, read live on every session,
/// and they do not change which list the server is offering.
#[must_use]
pub fn fingerprint<S: std::hash::BuildHasher>(absent: &HashMap<String, String, S>) -> String {
    let mut names: Vec<&str> = absent.keys().map(String::as_str).collect();
    names.sort_unstable();
    names.join(",")
}

/// What the kept list turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The kept list IS the list the server is offering now.
    Current {
        /// The tools, every one of them.
        tools: Vec<String>,
        /// How old the kept list is, in seconds.
        age_s: i64,
        /// The window the server gave it, in seconds.
        ttl_s: i64,
    },
    /// The kept list was not used.
    Missed(Miss),
}

/// Where one host's list is kept.
///
/// A store with no path keeps nothing: there is nowhere to keep it, and a
/// client with nowhere to keep it asks the server every time - which is
/// what this client did before there was a cache at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    path: Option<PathBuf>,
}

impl Store {
    /// The store that keeps a list in `path`.
    #[must_use]
    pub const fn at(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// The store that keeps nothing.
    #[must_use]
    pub const fn nowhere() -> Self {
        Self { path: None }
    }

    /// The kept list, and how it stands against the list the server is
    /// offering now.
    ///
    /// The check is here rather than at the call site so that there is
    /// exactly one answer to "is this list still true", and it is answered
    /// by the LIVE server's two facts and by the server's own window - not
    /// by the age of a file.
    #[must_use]
    pub fn hit(&self, version: &str, absent: &str) -> Hit {
        let Some(path) = &self.path else {
            return Hit::Missed(Miss::None);
        };
        // A file that cannot be read is NO list: it is not a wrong list and
        // it is not a crash. The list is gone, and the server is asked again.
        let Ok(text) = std::fs::read_to_string(path) else {
            return Hit::Missed(Miss::None);
        };
        let Ok(entry) = serde_json::from_str::<Entry>(&text) else {
            return Hit::Missed(Miss::None);
        };
        let now = crate::clock::now_epoch();
        let age_s = now.saturating_sub(entry.fetched_at);
        // The kept list stands against the list the server is offering now:
        // a RELEASE (the version moved), a host whose own tool set moved
        // (the absent set), or the window the server gave having closed.
        let miss = if !entry.is_for(version, absent) {
            if entry.version == version {
                // The version is the same, so what moved is the host's own
                // tool set: a tool was gained or lost.
                Miss::Reshaped {
                    kept: entry.absent,
                    now: absent.to_string(),
                }
            } else {
                // The version moved: this is a RELEASE.
                Miss::Released {
                    kept: entry.version,
                    now: version.to_string(),
                }
            }
        } else if !entry.is_fresh(now) {
            Miss::Expired {
                age_s,
                ttl_s: entry.window_s(),
            }
        } else {
            let ttl_s = entry.window_s();
            return Hit::Current {
                tools: entry.tools,
                age_s,
                ttl_s,
            };
        };
        Hit::Missed(miss)
    }

    /// Keep this list, under the key it was taken under.
    ///
    /// A window of zero is the server saying this answer is a live fact:
    /// there is no window to keep it in, so nothing is written. A server
    /// that names no version is not keyed at all, so nothing is written
    /// either - the version is the only thing that says which binary holds
    /// the tool table.
    ///
    /// A write that fails is not an error the caller can act on: the list
    /// was read either way, and the next session will fetch it again.
    ///
    /// # Errors
    ///
    /// The file could not be written.
    pub fn store(
        &self,
        version: &str,
        absent: &str,
        tools: &[String],
        ttl_ms: u64,
    ) -> Result<(), String> {
        let Some(path) = &self.path else {
            // Nowhere to keep it is not a failure: it is a client with no
            // catalogue kept, which asks the server every time.
            return Ok(());
        };
        if ttl_ms == 0 || version.is_empty() {
            return Ok(());
        }
        let entry = Entry {
            version: version.to_string(),
            absent: absent.to_string(),
            tools: tools.to_vec(),
            ttl_ms,
            fetched_at: crate::clock::now_epoch(),
        };
        let text = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::state::write_atomic(path, &text)
    }
}
