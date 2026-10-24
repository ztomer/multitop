//! The Ops view's data (servers ROADMAP 12.17b).
//!
//! What a host's `mcp_host` says about the things the agent does not stream:
//! health verdicts, cron jobs, container health, ZFS pools, the alerts its
//! cron wrapper logged - and, since the machine model (servers ROADMAP O92),
//! the machine's TUNNEL and its CAPACITY.
//!
//! Sections, each read from one answer and each able to say why it is not
//! there: a host that lists no `docker.containers` has no Docker (an honest
//! absence, not an error), and one section failing never hides the others.
//! Health comes from `health://latest` - each check's last verdict, as the
//! estate's own cron recorded it - never from `health.run_all`, which runs
//! the whole suite (25 s on .33) and must not be a poll.
//!
//! **THREE WAYS A SECTION IS NOT READY, AND THEY ARE NOT THE SAME.** A
//! section can be ABSENT (the host does not offer it, and says why), FAILED
//! (the estate's own report that it could not do it), or UNKNOWN (the
//! answer did not arrive: this client did not hear, and knows nothing
//! about the thing it asked - servers ROADMAP E74). The last is a GAP in
//! the reading and never a fault of the estate, and it is never an
//! all-clear either: the section says it is unknown, and says why.
//!
//! **THE VERDICT IS DATED, AND THAT IS THE POINT.** The clients read the
//! published verdict instead of re-running the suite (servers ROADMAP E12),
//! so what a card shows is as old as the estate's last cron run: a reader
//! that cannot date a verdict cannot tell a 3-minute-old answer from a
//! 29-minute-old one - and those are different answers about the same
//! machine. So the age is on the line, in the server's own words, and a
//! verdict past the estate's interval says so and is not green. The suite
//! the verdict belongs to is the server's own answer (`health.list`): the
//! published table also carries the daily checks, whose verdicts are a day
//! old by design, and a count that included them would be paired with an
//! age that does not describe them.
//!
//! Every section is decided the same way: the tool the host LISTS is asked,
//! and a tool the host does not list is an absence carrying the server's own
//! reason for it (`server/discover`'s `dev.multitop/absent`), never a guess
//! and never a green "none". The ZFS section is no exception: `zfs.health`
//! is the tool that owns the pools' state, so the view reads its answer and
//! never runs `zpool status` itself (servers ROADMAP O82). The tunnel is
//! `net.egress`'s the same way: the view reads the verdict, and never probes
//! an exit country itself.
//!
//! A MACHINE may run several roots (`.90` runs the storage root and the
//! solver root), so every row that came from an instance carries the
//! instance's name. The view draws one card per machine and names the stack
//! each row belongs to, which is the whole point of the machine model: a
//! tunnel in the solver root that every view called "fine" (2026-10-19) is
//! a fact nobody could attribute.

pub mod answers;
pub mod cache;
pub mod gather;
pub mod model;
pub mod model_health;
pub mod pass;
pub mod rate;
pub mod render;
pub mod status;
pub mod tick;
pub mod triage;

pub use answers::*;
pub use gather::*;
pub use model::*;
pub use model_health::*;
pub use rate::*;
pub use status::*;
pub use triage::{parse_triage, Triage, TriageItem, TRIAGE_TOOL};

/// The tool list, and where this client got it: the state the panel says.
pub use crate::mcp::catalogue::Catalogue;
/// The reasons a kept tool list was not used.
pub use crate::mcp::catalogue::Miss;
/// How the kept tool list was checked against the server's own word.
pub use crate::mcp::catalogue::Proof;

use std::time::Duration;

/// The resource holding each health check's last recorded verdict.
pub const HEALTH_URI: &str = "health://latest";
/// The tool that says which checks this machine's periodic suite runs.
pub const HEALTH_LIST: &str = "health.list";
/// The estate's publication cadence, in minutes, as this client LAST KNEW
/// it.
///
/// A FALLBACK, not the rule (servers ROADMAP E53): the server publishes
/// its cadence with the verdict (`health://latest`, beside the age), and
/// the client's rate is DERIVED from what it published. This number is
/// used only where there is nothing published to derive from - a host
/// this client has never read, whose server could not read its own
/// cadence.
pub const FALLBACK_CADENCE_MINUTES: i64 = 15;
/// How old a published verdict may be before the client calls it STALE,
/// when the server published no cadence to measure it against: the
/// fallback cadence plus a grace for a run that is still going.
pub const HEALTH_STALE_AFTER_S: i64 = (FALLBACK_CADENCE_MINUTES + 5) * 60;
/// The tool that answers about a host's ZFS pools: their state, their vdev
/// tree, and what ZFS says to do about them.
pub const ZFS_TOOL: &str = "zfs.health";
/// The tool that answers where the machine's tunnels exit: the exit
/// country, the leak verdict, and the stack that declares each tunnel.
pub const EGRESS_TOOL: &str = "net.egress";
/// The tool that answers the machine's filesystems: their size, their free
/// space and the share used.
pub const DISK_TOOL: &str = "disk.usage";
/// The tool that answers the machine's identity and load: its CPUs, its
/// memory, and the instances it declares.
pub const FACTS_TOOL: &str = "host.facts";
/// How far back the alerts section looks.
/// Reason: one day is the window an operator glancing at a board cares
/// about; older events are in `alerts.since` for whoever asks.
pub const ALERT_WINDOW_S: i64 = 86_400;

/// How long the egress answer may take.
///
/// Reason: `net.egress` measures the tunnel's exit country with the check's
/// own probe - three sources, about half a minute on a machine whose tunnel
/// is up. A timeout BREAKS the session (a late reply could be mistaken for
/// the next one's), so a deadline shorter than the probe would fail the
/// whole poll instead of one section.
pub const EGRESS_TIMEOUT: Duration = Duration::from_secs(75);

/// Where a host's Ops answers come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The host's `mcp` command: `mcp_host` over ssh, or any command locally.
    Command(String),
    /// This machine, with no `mcp` configured: this build's own agent in its
    /// `mcp` mode. A Mac has no `mcp_host`, and the hint to configure one was
    /// a dead end -- there was no command worth adding.
    LocalAgent,
}

impl Source {
    /// The source for `server`: its `mcp` command when it names one (which
    /// always wins), the local agent for this machine, and nothing for a
    /// remote host without one -- that is `OpsState::NotConfigured`.
    #[must_use]
    pub fn of(server: &crate::config::Server) -> Option<Self> {
        server
            .mcp
            .clone()
            .map(Self::Command)
            .or_else(|| crate::ssh::is_local(server).then_some(Self::LocalAgent))
    }
}

/// The machine's banner: its name and its address.
///
/// The name is the one the operator gave the machine in the config; the
/// address is beside it, because a name alone does not say which box this
/// is. A machine with no name shows its address alone, as it always did.
#[must_use]
pub fn banner(server: &crate::config::Server) -> String {
    let name = server
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    name.map_or_else(|| server.host.clone(), |n| format!("{n} ({})", server.host))
}

/// One section: its data, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part<T> {
    Ready(T),
    /// The host does not offer it (a stated status, not a fault).
    Absent(String),
    /// It was asked for, and the ESTATE reported a failure: its own words.
    Failed(String),
    /// It was asked for, and the ANSWER DID NOT ARRIVE: the fact is
    /// unknown, and this is why it is unknown.
    ///
    /// A gap in the reading, NOT a finding about the estate: a deadline
    /// missed says the client did not hear, not that the estate is
    /// broken. It is never an all-clear either (servers ROADMAP E74).
    Unknown(String),
}

/// Everything one poll learned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// `serverInfo` name and version.
    pub server: (String, String),
    /// When it was asked, epoch seconds.
    pub at: i64,
    /// How often THIS client samples the published verdict, in seconds.
    ///
    /// The answer carries the client's own rate, because the rate is what
    /// decides whether the verdict can be two publications behind the truth
    /// (DECISIONS D15) - and the age on the line cannot say that: a verdict
    /// two publications behind looks exactly like a fresh one. So the answer
    /// says it, where the verdict is shown, rather than in silence.
    pub sampled_every_s: i64,
    /// Where the TOOL LIST came from: the server, or the kept list.
    ///
    /// The list is this panel's SHAPE - a tool the list does not name has no
    /// section - so a panel that cannot say where its shape came from could
    /// be showing yesterday's tools as today's (servers ROADMAP E61).
    pub catalogue: Catalogue,
    pub health: Part<Health>,
    /// The machine's tunnels.
    pub vpn: Part<Egress>,
    pub cron: Part<Vec<Job>>,
    pub containers: Part<Vec<Container>>,
    pub zfs: Part<Vec<Pool>>,
    /// The machine's filesystems.
    pub disk: Part<Disk>,
    /// The machine's identity and capacity.
    pub facts: Part<Facts>,
    pub alerts: Part<Vec<Alert>>,
    /// The machine's one verdict (`machine.triage`, servers ROADMAP O92).
    pub triage: Part<Triage>,
}

/// What a panel in the Ops view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpsState {
    /// The host's `[[servers]]` entry names no `mcp` command.
    NotConfigured,
    /// Asked, no answer yet.
    Asking,
    /// The session could not be opened or broke; the last good answer, if
    /// any, is kept and shown as such.
    Failed {
        why: String,
        last: Option<Box<Snapshot>>,
    },
    Ready(Box<Snapshot>),
}

impl Snapshot {
    /// The sections whose answer did not arrive, with the reason.
    ///
    /// A gap in the reading, NAMED: the operator has to be able to see
    /// which fact is missing, and not only that something is (servers
    /// ROADMAP E74).
    #[must_use]
    pub fn unknowns(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = Vec::new();
        gap("health", &self.health, &mut out);
        gap("vpn", &self.vpn, &mut out);
        gap("cron", &self.cron, &mut out);
        gap("containers", &self.containers, &mut out);
        gap("zfs", &self.zfs, &mut out);
        gap("disk", &self.disk, &mut out);
        gap("facts", &self.facts, &mut out);
        gap("alerts", &self.alerts, &mut out);
        gap("triage", &self.triage, &mut out);
        out
    }

    /// Every name and subject it shows, for `/` to search.
    fn terms(&self) -> Vec<&str> {
        let mut t = Vec::new();
        if let Part::Ready(h) = &self.health {
            t.extend(h.checks.iter().map(|c| c.name.as_str()));
        }
        if let Part::Ready(e) = &self.vpn {
            t.extend(e.tunnels.iter().flat_map(|t| {
                [
                    t.container.as_str(),
                    t.leak.as_str(),
                    t.instance.as_deref().unwrap_or_default(),
                ]
            }));
        }
        if let Part::Ready(j) = &self.cron {
            t.extend(
                j.iter()
                    .flat_map(|j| [j.label.as_str(), j.instance.as_deref().unwrap_or_default()]),
            );
        }
        if let Part::Ready(c) = &self.containers {
            t.extend(
                c.iter()
                    .flat_map(|c| [c.name.as_str(), c.status.as_str(), c.stack()]),
            );
        }
        if let Part::Ready(p) = &self.zfs {
            t.extend(p.iter().flat_map(|p| {
                std::iter::once(p.name.as_str()).chain(p.members.iter().map(|m| m.name.as_str()))
            }));
        }
        if let Part::Ready(d) = &self.disk {
            t.extend(d.mounts.iter().map(|m| m.mount.as_str()));
        }
        if let Part::Ready(f) = &self.facts {
            if let Some(i) = &f.instances {
                t.extend(i.rows.iter().map(|r| r.instance.as_str()));
                // The server's own reason for a row that is not an instance
                // is searchable: `/declaration` finds the machine whose
                // declaration could not be read.
                t.extend(i.rows.iter().filter_map(|r| r.problem.as_deref()));
            }
        }
        if let Part::Ready(a) = &self.alerts {
            t.extend(a.iter().map(|a| a.subject.as_str()));
        }
        if let Part::Ready(tr) = &self.triage {
            t.extend(
                tr.items
                    .iter()
                    .flat_map(|i| [i.instance.as_str(), i.what.as_str()]),
            );
        }
        // A gap in the reading is searchable too: `/no answer` finds the
        // host whose docker call missed its deadline.
        t.extend(self.unknowns().into_iter().map(|(_, why)| why));
        t
    }
}

impl OpsState {
    /// What `/` searches in the Ops view: the names and subjects shown,
    /// and a failure's reason (so `/refused` finds the hosts that refused).
    #[must_use]
    pub fn terms(&self) -> Vec<&str> {
        match self {
            Self::NotConfigured | Self::Asking => Vec::new(),
            Self::Ready(s) => s.terms(),
            Self::Failed { why, last } => {
                let mut t = vec![why.as_str()];
                if let Some(s) = last {
                    t.extend(s.terms());
                }
                t
            }
        }
    }
}

/// One section's gap, if it has one: the section, and why it is unknown.
fn gap<'a, T>(name: &'static str, part: &'a Part<T>, out: &mut Vec<(&'static str, &'a str)>) {
    if let Part::Unknown(why) = part {
        out.push((name, why.as_str()));
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
#[path = "pass_tests.rs"]
mod pass_tests;

#[cfg(test)]
#[path = "zfs_tests.rs"]
mod zfs_tests;

#[cfg(test)]
#[path = "machine_tests.rs"]
mod machine_tests;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "health_tests.rs"]
mod health_tests;

#[cfg(test)]
#[path = "rate_tests.rs"]
mod rate_tests;

#[cfg(test)]
#[path = "wording_tests.rs"]
mod wording_tests;

#[cfg(test)]
#[path = "triage_tests.rs"]
mod triage_tests;

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod catalogue_tests;

#[cfg(test)]
#[path = "unknown_tests.rs"]
mod unknown_tests;

#[cfg(test)]
#[path = "tick_tests.rs"]
mod tick_tests;
