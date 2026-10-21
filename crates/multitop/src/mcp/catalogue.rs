//! The tool CATALOGUE: the list of tools a host offers, kept between
//! sessions so that a gather does not pull the whole list every time.
//!
//! servers ROADMAP E61: the Ops gather pulled the **whole catalogue** on
//! every gather - 65,467 B on `.33`, 51,716 B on `.90`, 32,724 B on `.158`,
//! measured - and the list is the SAME list every time: it changes at a
//! RELEASE, not every gather. At the estate's own sampling rate that is
//! the same catalogue, over and over, hundreds of times a day.
//!
//! **THE KEY IS THE SERVER'S OWN WORD.** The list a host offers is a
//! function of the binary that holds the tool table and of the host's own
//! state, and the server knows BOTH of them exactly: it publishes the
//! **DIGEST** of the exact list `tools/list` answers, in `server/discover`,
//! beside the map of the tools it does not offer (`dev.multitop/absent`).
//! The digest is that list's SHA-256, so the client's kept copy is checked
//! against the server's own word - not against a GUESS about it.
//!
//! **THE GUESS WAS WRONG OFTEN ENOUGH TO MEASURE.** The older key was the
//! advertised version plus the absent set, and it made the client re-list
//! on every release - including the releases that touched no tool table at
//! all (four of the last ten: v1.99.13-14, 15-16, 16-17, 17-18). Each of
//! those cost the estate 149,907 B a gather for a list that had not
//! changed, and each of them could have missed a table that moved without
//! a release (servers ROADMAP E72).
//!
//! **THE DIGEST IS REMEMBERED, NOT COMPUTED.** It is of the WHOLE list -
//! names AND schemas - and this client keeps only the names, so it cannot
//! compute the digest again: it keeps the digest it was HANDED, beside the
//! list the word was given for. A digest over the names alone would be a
//! smaller fact and a weaker one - a schema change would go unseen - and
//! the server publishes the stronger one.
//!
//! **A DIGEST THAT IS ABSENT IS SAID TO BE ABSENT.** A server that
//! publishes no digest - one too old to know the field, or one that
//! publishes something that is not a digest - has said nothing about its
//! list. The kept copy is then checked the OLDER way (the version and the
//! absent set), the panel says the digest was absent, and the client does
//! NOT start re-listing every session: that is the defect this closes, not
//! a new one.
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
//! cannot vanish from the panel.
//!
//! **A STALE LIST IS SAID TO BE STALE.** The client says where the list
//! came from and how old it is, and a kept list past the server's window
//! is not used at all: the server is asked again, and the client says why.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A second in milliseconds: the unit `ttlMs` is in.
pub const MS_PER_S: u64 = 1_000;

/// The `_meta` key under which a server publishes the DIGEST of the list
/// `tools/list` answers.
///
/// The estate's key, beside the absent map this client already reads
/// (servers ROADMAP E70): the server's own word about the list it serves,
/// so a client that keeps the list can CHECK its copy instead of inferring
/// it from the version and the absent set.
pub const META_DIGEST: &str = "dev.multitop/catalogueDigest";

/// The algorithm, in front of the digest.
///
/// The estate's spelling (the registry and the update check write it the
/// same way): a digest without its algorithm is a number nobody can check.
pub const DIGEST_PREFIX: &str = "sha256:";

/// How many hex digits a digest carries.
const DIGEST_HEX: usize = 64;

/// Whether `text` is a digest this client can check.
///
/// The algorithm, a colon, and the hex - the estate's spelling. A digest
/// in any other shape cannot be checked against anything, so it is not a
/// digest: the client says the fact is ABSENT rather than comparing
/// strings it does not understand (servers ROADMAP E72).
#[must_use]
pub fn is_digest(text: &str) -> bool {
    text.strip_prefix(DIGEST_PREFIX).is_some_and(|hex| {
        hex.len() == DIGEST_HEX
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

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
        /// How the kept list was checked against the server's own word.
        proof: Proof,
    },
    /// The kept list was put aside, and the server was asked again.
    Relisted {
        /// Why the kept list was not used.
        miss: Miss,
        /// How long the server said the list may be kept, in seconds.
        ttl_s: i64,
    },
}

/// How the kept list was checked against the server's own word.
///
/// The two states, and the difference is the whole point (servers ROADMAP
/// E72): a digest that MATCHED is the server saying the kept copy is the
/// list it serves, and a digest that was ABSENT is a fact this client
/// could not check - which is not the same thing as saying the copy is
/// fine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Proof {
    /// The server published a digest, and it MATCHED the kept list's.
    Digest {
        /// The digest the server publishes now.
        digest: String,
    },
    /// The server published NO digest: the fact is absent, and the kept
    /// list was checked the older way - the version and the absent set.
    Absent,
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
    /// The server's catalogue digest is not the one the kept list was taken
    /// under: the tool table moved.
    Changed {
        /// The digest the kept list was taken under.
        kept: String,
        /// The digest the server publishes now.
        now: String,
    },
    /// The kept list carries NO digest, so it cannot be checked against the
    /// server's own word.
    Uncheckable,
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
    /// The digest the SERVER published when the list was taken.
    ///
    /// Remembered, not computed: the digest is of the WHOLE list - names
    /// AND schemas - and this client keeps only the names, so it cannot
    /// compute it again. It keeps the server's word (servers ROADMAP E72).
    ///
    /// `None` for a list taken before the server published one, and for a
    /// server that publishes none: the fact is absent, and the client says
    /// so.
    #[serde(default)]
    pub digest: Option<String>,
    /// The tools themselves: EVERY tool the server listed.
    pub tools: Vec<String>,
    /// How long the SERVER said this answer may be kept (`ttlMs`).
    pub ttl_ms: u64,
    /// When it was taken (epoch seconds).
    pub fetched_at: i64,
}

impl Entry {
    /// Whether this entry is the list the live server is offering now, by
    /// the OLDER key.
    ///
    /// The version (the binary) and the absent set (the host's own state) -
    /// a GUESS about the list, used only when the server publishes no
    /// digest of its own (servers ROADMAP E72).
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
        /// How the kept list was checked against the server's own word.
        proof: Proof,
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
    /// exactly one answer to "is this list still true".
    ///
    /// **THE SERVER'S OWN WORD COMES FIRST.** `digest` is what the server
    /// published about the list it serves, read live on every session. A
    /// server that publishes one has SAID what its list is: the kept copy
    /// is either that list or it is not, and the version and the absent set
    /// are only a guess about it. So a digest that matches keeps the list -
    /// even across a release that touched no tool table - and a digest that
    /// does not match puts it aside.
    ///
    /// **NO DIGEST IS NOT A CHECK.** A server that publishes none has said
    /// nothing about its list: the kept copy is checked the OLDER way (the
    /// version and the absent set), and the answer says so, because an
    /// absent fact is not an all-clear (servers ROADMAP E72).
    #[must_use]
    pub fn hit(&self, version: &str, absent: &str, digest: Option<&str>) -> Hit {
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
        let ttl_s = entry.window_s();
        let fresh = entry.is_fresh(now);
        // The server's own word, when it published one.
        if let Some(digest) = digest {
            return match entry.digest.as_deref() {
                Some(kept) if kept == digest => {
                    if fresh {
                        Hit::Current {
                            tools: entry.tools,
                            age_s,
                            ttl_s,
                            proof: Proof::Digest {
                                digest: digest.to_string(),
                            },
                        }
                    } else {
                        Hit::Missed(Miss::Expired { age_s, ttl_s })
                    }
                }
                Some(kept) => Hit::Missed(Miss::Changed {
                    kept: kept.to_string(),
                    now: digest.to_string(),
                }),
                // The kept list carries no digest, so it cannot be checked
                // against the server's word: it is put aside and the server
                // is asked again - once, and then the digest is remembered
                // beside the list it was given for.
                None => Hit::Missed(Miss::Uncheckable),
            };
        }
        // No digest: the fact is ABSENT. The kept list is checked the older
        // way - a RELEASE (the version moved), a host whose own tool set
        // moved (the absent set), or the window the server gave having
        // closed - and the answer says the check was the weaker one.
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
        } else if !fresh {
            Miss::Expired { age_s, ttl_s }
        } else {
            return Hit::Current {
                tools: entry.tools,
                age_s,
                ttl_s,
                proof: Proof::Absent,
            };
        };
        Hit::Missed(miss)
    }

    /// Keep this list, under the key it was taken under.
    ///
    /// The digest is kept BESIDE the list, exactly as the server handed it:
    /// it is the server's word about that list, and the word is the only
    /// way to check the list later (servers ROADMAP E72).
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
        digest: Option<&str>,
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
            digest: digest.map(str::to_string),
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
