//! The machine's HEALTH verdict, as the clients read it (servers ROADMAP
//! E12).
//!
//! The verdict is PUBLISHED, not run: the estate's own cron runs the suite
//! every 15 minutes and publishes what it found, and the clients read that
//! (the `health://latest` resource) instead of running the suite themselves.
//!
//! So a verdict read here is as old as the estate's last run - and the age
//! is what says so. The server's words date it ("13 min ago"), because one
//! formatter dates a verdict everywhere it is shown; a second formatter
//! here would be a second answer to "how old is this".
//!
//! Split from `model.rs` for the house's line cap, along the seam that was
//! already there: the health verdict is its own fact, with its own age and
//! its own suite.

/// One health check's last recorded verdict, as `health://latest` publishes
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    /// The instance (the stack) the check ran in.
    pub instance: Option<String>,
    pub ok: bool,
    pub rc: i64,
    pub ts: i64,
    /// The verdict's age, in the SERVER's own words ("13 min ago").
    ///
    /// The server's words, not this client's: one formatter dates a verdict
    /// everywhere it is shown (the estate's `machine.triage` line and the
    /// resource's rows are the same words), and a second formatter here
    /// would be a second answer to "how old is this".
    pub age: Option<String>,
}

/// The machine's PUBLISHED health verdict: every check's last result, and
/// how old that verdict is.
///
/// The clients read this instead of running the suite themselves (servers
/// ROADMAP E12): the estate's own cron runs the suite every 15 minutes and
/// publishes what it found, so a verdict read here is as old as the estate's
/// last run - and the age is what says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health {
    /// The checks, narrowed to the machine's periodic suite.
    pub checks: Vec<Check>,
    /// The machine's own freshness, in the server's words: the oldest
    /// verdict in its periodic suite.
    pub oldest_age: Option<String>,
    /// The same age in seconds: what the staleness judgment reads.
    pub oldest_age_s: Option<i64>,
}

impl Health {
    /// Whether the verdict is too old to present as a current answer.
    ///
    /// The estate's suite runs every 15 minutes and publishes what it
    /// found, so a verdict read between two of the estate's runs is at most
    /// that old. One interval plus a grace covers a suite that is still
    /// running; past that the estate's own cron did not fire, and the
    /// verdict is not a current answer about the machine.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.oldest_age_s
            .is_some_and(|s| s > super::HEALTH_STALE_AFTER_S)
    }

    /// Keep only the checks the machine's periodic suite runs.
    ///
    /// The published table carries the DAILY checks' verdicts too, and
    /// those are a day old by design - so a verdict that counted them would
    /// pair a number with an age that does not describe it. The suite is
    /// the server's own answer (`health.list`), never this client's guess.
    pub fn narrow_to(&mut self, suite: &Suite) {
        self.checks
            .retain(|c| suite.takes(c.instance.as_deref(), &c.name));
    }
}

/// The machine's periodic suite: the checks `health.list` says are in it.
///
/// The server owns which checks are in the suite; the client asks and never
/// guesses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Suite {
    /// The suite's checks, by (instance, name).
    pub checks: Vec<(Option<String>, String)>,
}

impl Suite {
    /// Whether the suite runs `name` in `instance`.
    ///
    /// The instance must match when both name one; a row that names none is
    /// the machine's own check.
    #[must_use]
    pub fn takes(&self, instance: Option<&str>, name: &str) -> bool {
        self.checks.iter().any(|(i, n)| {
            n == name
                && match (i.as_deref(), instance) {
                    (None, _) | (_, None) => true,
                    (Some(a), Some(b)) => a == b,
                }
        })
    }
}
