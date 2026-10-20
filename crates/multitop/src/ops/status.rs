//! The estate's STATUS vocabularies: the words the server publishes, held
//! as ENUMS.
//!
//! WHY (DECISIONS D14, owner 2026-10-20): *"V is a visual representation
//! of a status. we should use enums, not utf symbols. it shouldn't be
//! something we transport around."*
//!
//! A status that travels as a `String` is compared as TEXT - `"ONLINE" ==
//! "ONLINE"` - and a word that lies survives every comparison: the pool
//! that is SUSPENDED, the member that is REMOVED, the "ok" that is not ok.
//! An enum cannot be compared to a word it does not have.
//!
//! So the model holds a STATUS, and the wire carries the server's WORD
//! (unchanged - `confirmed`, `ONLINE`, `running`), and the PICTURE of a
//! verdict is the renderer's own: this module has no glyph in it, and
//! neither has the model. `render::Tone` draws the verdict, this client's
//! way.
//!
//! THE WORDS ARE THE ESTATE'S, and they are SHARED: the routines client
//! holds the same vocabularies, with the same words, so a fact cannot come
//! back as `"ONLINE"` to one client and `"online"` to the other. A client
//! that meets a word its enum does not have GROWS THE ENUM - not the
//! string, and not the glyph.
//!
//! TWO KINDS OF VOCABULARY:
//!
//! - **OURS** - the estate's own words (the egress verdict). The set is
//!   closed, so a word outside it is a fault and not a state.
//! - **THEIRS** - ZFS's and Docker's words. The set belongs to somebody
//!   else and is not ours to close: a word ZFS adds must still be shown
//!   as it stands, so those enums keep an `Other` arm that carries the
//!   foreign word unchanged, and the estate's own question - "is this
//!   ONLINE?" - is asked of the enum.

/// The egress verdict, as `net.egress` publishes it.
///
/// The GOOD case, and the word that would read as the opposite beside the
/// word "leak" - which is why the row's tone carries the verdict and the
/// detail says which of the two it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leak {
    /// The exit is where it belongs.
    Confirmed,
    /// Every source that answered agrees the exit is elsewhere.
    Leaked,
    /// The sources disagree, so the geolocation cannot settle it.
    Disagreement,
    /// Too few sources answered for a verdict.
    Unverified,
    /// No verdict at all: the tunnel was not running, so nothing was
    /// probed.
    Unknown,
}

impl Leak {
    /// Every verdict, in the order the server's schema lists them.
    pub const ALL: [Self; 5] = [
        Self::Confirmed,
        Self::Leaked,
        Self::Disagreement,
        Self::Unverified,
        Self::Unknown,
    ];

    /// The wire word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Leaked => "leaked",
            Self::Disagreement => "disagreement",
            Self::Unverified => "unverified",
            Self::Unknown => "unknown",
        }
    }

    /// The wire words, in the order the server's schema lists them.
    #[must_use]
    pub fn words() -> Vec<&'static str> {
        Self::ALL.iter().map(|s| s.as_str()).collect()
    }

    /// The verdict a word names, when this vocabulary has it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == word)
    }
}

/// How bad a verdict is, as the view's tone bands.
///
/// A verdict is a STATUS, so its severity is one too - a number compared
/// with `max_by_key` was a status wearing a number's clothes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The exit is where it belongs.
    Ok,
    /// Neither settled nor wrong: a disagreement, or an unverifiable
    /// answer.
    Unsettled,
    /// The exit is somewhere else, or nothing was measured.
    Bad,
}

/// The estate's own verdict about a pool: WHOLE, or not.
///
/// "2 pools whole: pool0, pool1" is OURS - computed from the vdev tree,
/// not read from ZFS - and it is a DIFFERENT fact from ZFS's own word. A
/// pool that says ONLINE with a member REMOVED is whole in name only, and
/// the word that lies is the word this verdict is not.
///
/// The WORD is the renderer's: this enum carries the verdict, and the frame
/// says it in the house's own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The pool is whole: ONLINE, and every member ONLINE.
    Whole,
    /// The pool is not whole.
    NotWhole,
}

impl Verdict {
    /// The estate's word for the verdict.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Whole => "whole",
            Self::NotWhole => "not whole",
        }
    }
}

/// ZFS's own state words - `ONLINE`, `DEGRADED`, `SUSPENDED`, ...
///
/// ZFS owns this vocabulary, so an unknown word is kept VERBATIM in
/// [`ZfsState::Other`] rather than dropped: the view shows what the
/// server published, and a paraphrase would show something else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZfsState {
    /// The pool or vdev is whole.
    Online,
    /// The pool or vdev lost redundancy.
    Degraded,
    /// The pool or vdev has an error the operator must act on.
    Faulted,
    /// The pool or vdev was taken offline by the administrator.
    Offline,
    /// The device was physically removed.
    Removed,
    /// The pool or vdev is not available.
    Unavail,
    /// The pool's I/O is suspended: writes BLOCK rather than fail.
    Suspended,
    /// A word this vocabulary does not name: ZFS's, kept verbatim.
    Other(String),
}

impl ZfsState {
    /// Every state this vocabulary names, with its word.
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::Online, "ONLINE"),
        (Self::Degraded, "DEGRADED"),
        (Self::Faulted, "FAULTED"),
        (Self::Offline, "OFFLINE"),
        (Self::Removed, "REMOVED"),
        (Self::Unavail, "UNAVAIL"),
        (Self::Suspended, "SUSPENDED"),
    ];

    /// The wire word - ZFS's own, unchanged.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Online => "ONLINE",
            Self::Degraded => "DEGRADED",
            Self::Faulted => "FAULTED",
            Self::Offline => "OFFLINE",
            Self::Removed => "REMOVED",
            Self::Unavail => "UNAVAIL",
            Self::Suspended => "SUSPENDED",
            Self::Other(w) => w,
        }
    }

    /// The wire words this vocabulary names.
    #[must_use]
    pub fn words() -> Vec<&'static str> {
        Self::ALL.iter().map(|(_, w)| *w).collect()
    }

    /// The state a word names, when this vocabulary has it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, w)| *w == word)
            .map(|(s, _)| s.clone())
    }

    /// The state a word names. A word this vocabulary does not name is kept
    /// verbatim.
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        Self::parse(word).unwrap_or_else(|| Self::Other(word.to_owned()))
    }

    /// Whether the state is ONLINE - the estate's one question about a pool
    /// or a vdev, asked of the enum and never of a string.
    #[must_use]
    pub const fn is_online(&self) -> bool {
        matches!(self, Self::Online)
    }
}

/// Docker's own container states - `running`, `exited`, `paused`, ...
///
/// Docker owns this vocabulary, so an unknown word is kept VERBATIM in
/// [`DockerState::Other`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockerState {
    /// The container is up.
    Running,
    /// The container ran and stopped.
    Exited,
    /// The container is paused.
    Paused,
    /// The container was created and never started.
    Created,
    /// The container is restarting.
    Restarting,
    /// The container is being removed.
    Removing,
    /// The container is dead: it could not be removed.
    Dead,
    /// A word this vocabulary does not name: Docker's, kept verbatim.
    Other(String),
}

impl DockerState {
    /// Every state this vocabulary names, with its word.
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::Running, "running"),
        (Self::Exited, "exited"),
        (Self::Paused, "paused"),
        (Self::Created, "created"),
        (Self::Restarting, "restarting"),
        (Self::Removing, "removing"),
        (Self::Dead, "dead"),
    ];

    /// The wire word - Docker's own, unchanged.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Paused => "paused",
            Self::Created => "created",
            Self::Restarting => "restarting",
            Self::Removing => "removing",
            Self::Dead => "dead",
            Self::Other(w) => w,
        }
    }

    /// The wire words this vocabulary names.
    #[must_use]
    pub fn words() -> Vec<&'static str> {
        Self::ALL.iter().map(|(_, w)| *w).collect()
    }

    /// The state a word names, when this vocabulary has it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, w)| *w == word)
            .map(|(s, _)| s.clone())
    }

    /// The state a word names. A word this vocabulary does not name is kept
    /// verbatim.
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        Self::parse(word).unwrap_or_else(|| Self::Other(word.to_owned()))
    }
}

/// Docker's own container health - `healthy`, `unhealthy`, `starting`, or
/// `none` when the container declares no health check at all.
///
/// Docker owns this vocabulary, so an unknown word is kept VERBATIM in
/// [`DockerHealth::Other`]. [`DockerHealth::Unset`] is the empty word: the
/// answer carried no health field at all, which is a stated absence rather
/// than a health check that does not exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockerHealth {
    /// The container's own health check passes.
    Healthy,
    /// The container's own health check fails.
    Unhealthy,
    /// The container's own health check has not settled yet.
    Starting,
    /// The container declares no health check (`none`).
    NoCheck,
    /// The answer carried no health field (empty).
    Unset,
    /// A word this vocabulary does not name: Docker's, kept verbatim.
    Other(String),
}

impl DockerHealth {
    /// Every state this vocabulary names, with its word.
    pub const ALL: [(Self, &'static str); 5] = [
        (Self::Healthy, "healthy"),
        (Self::Unhealthy, "unhealthy"),
        (Self::Starting, "starting"),
        (Self::NoCheck, "none"),
        (Self::Unset, ""),
    ];

    /// The wire word - Docker's own, unchanged.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Healthy => "healthy",
            Self::Unhealthy => "unhealthy",
            Self::Starting => "starting",
            Self::NoCheck => "none",
            Self::Unset => "",
            Self::Other(w) => w,
        }
    }

    /// The wire words this vocabulary names.
    #[must_use]
    pub fn words() -> Vec<&'static str> {
        Self::ALL.iter().map(|(_, w)| *w).collect()
    }

    /// The health a word names, when this vocabulary has it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, w)| *w == word)
            .map(|(s, _)| s.clone())
    }

    /// The health a word names. A word this vocabulary does not name is kept
    /// verbatim.
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        Self::parse(word).unwrap_or_else(|| Self::Other(word.to_owned()))
    }

    /// Whether the container declares NO health check: a GAP, not a fault -
    /// nothing is down, but nothing would notice it hang.
    #[must_use]
    pub const fn is_unchecked(&self) -> bool {
        matches!(self, Self::NoCheck | Self::Unset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed vocabulary refuses a word it does not have: that is the
    /// whole point of an enum, and the reason a tool cannot publish a
    /// word it does not have.
    #[test]
    fn a_closed_vocabulary_refuses_a_word_it_does_not_have() {
        assert_eq!(Leak::parse("ONLINE"), None);
        assert_eq!(Leak::parse("leak"), None);
        assert_eq!(Leak::parse("confirmed"), Some(Leak::Confirmed));
    }

    /// Every word the closed vocabulary publishes round-trips, and the list
    /// is the estate's - the same words the routines client holds.
    #[test]
    fn the_closed_vocabulary_publishes_the_estates_words() {
        assert_eq!(
            Leak::words(),
            vec![
                "confirmed",
                "leaked",
                "disagreement",
                "unverified",
                "unknown"
            ]
        );
        for w in Leak::words() {
            assert_eq!(Leak::parse(w).map(Leak::as_str), Some(w));
        }
    }

    /// A FOREIGN word is kept verbatim: the view shows what the server
    /// published, and a paraphrase would show something else.
    #[test]
    fn a_foreign_word_is_kept_verbatim() {
        assert_eq!(ZfsState::from_word("REPLACED").as_str(), "REPLACED");
        assert!(!ZfsState::from_word("REPLACED").is_online());
        assert!(ZfsState::from_word("ONLINE").is_online());
        assert_eq!(DockerState::from_word("running").as_str(), "running");
        assert_eq!(DockerHealth::from_word("none").as_str(), "none");
        assert_eq!(DockerHealth::from_word("").as_str(), "");
    }

    /// The severity bands are ordered, so the worst verdict is the one the
    /// view draws.
    #[test]
    fn the_severity_bands_are_ordered() {
        assert!(Severity::Bad > Severity::Unsettled);
        assert!(Severity::Unsettled > Severity::Ok);
    }

    /// The wire words are the ESTATE's, and they are the server's: a fact
    /// cannot come back as `ONLINE` to one client and `online` to the other.
    /// The routines client holds the same lists.
    #[test]
    fn the_wire_words_are_the_estates() {
        assert_eq!(
            Leak::words(),
            vec![
                "confirmed",
                "leaked",
                "disagreement",
                "unverified",
                "unknown"
            ]
        );
        assert_eq!(
            ZfsState::words(),
            vec![
                "ONLINE",
                "DEGRADED",
                "FAULTED",
                "OFFLINE",
                "REMOVED",
                "UNAVAIL",
                "SUSPENDED"
            ]
        );
        assert_eq!(
            DockerState::words(),
            vec![
                "running",
                "exited",
                "paused",
                "created",
                "restarting",
                "removing",
                "dead"
            ]
        );
    }

    /// The estate's verdict about a pool is OURS, and it is a different
    /// fact from ZFS's word: the word that lies is the word the verdict is
    /// not.
    #[test]
    fn the_pools_verdict_is_ours_and_has_its_own_words() {
        assert_eq!(Verdict::Whole.as_str(), "whole");
        assert_eq!(Verdict::NotWhole.as_str(), "not whole");
        assert_ne!(Verdict::Whole, Verdict::NotWhole);
    }
}
