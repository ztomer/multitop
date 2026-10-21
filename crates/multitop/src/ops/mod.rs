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
pub mod model;
pub mod model_health;
pub mod render;
pub mod status;

pub use answers::*;
pub use model::*;
pub use model_health::*;
pub use status::*;

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::mcp::client::Client;

/// The resource holding each health check's last recorded verdict.
pub const HEALTH_URI: &str = "health://latest";
/// The tool that says which checks this machine's periodic suite runs.
pub const HEALTH_LIST: &str = "health.list";
/// The estate's suite cadence, in minutes: the suite runs every 15 (servers
/// ROADMAP E12; `media_server` v1.99.11's `*/15` cron, deployed).
pub const SUITE_INTERVAL_MINUTES: i64 = 15;
/// How old a published verdict may be before the client calls it STALE: the
/// suite's interval plus a grace for a run that is still going.
///
/// A verdict read between two of the estate's own runs is at most the
/// interval old. Past that, the estate's cron did not fire, and the
/// verdict is not a current answer about the machine - which is the whole
/// reason the age is published.
pub const HEALTH_STALE_AFTER_S: i64 = (SUITE_INTERVAL_MINUTES + 5) * 60;
/// The longest a client may go between reads before its verdict can be two
/// publications behind: half the publication interval, in seconds.
///
/// DECISIONS D15 (owner, 2026-10-20): a client that samples at the rate
/// the estate publishes can ALIAS - its read can land just before a
/// publication and then wait a whole cycle, so its verdict is two
/// publications behind the truth. A client that reads slower than this
/// says so in its answer rather than in silence.
pub const MAX_SAMPLING_INTERVAL_S: i64 = (SUITE_INTERVAL_MINUTES * 60) / 2;

/// The warning the health section carries when this client samples the
/// published verdict too slowly.
///
/// The age on the line cannot say it: a verdict that is two publications
/// behind looks exactly like one that is fresh.
#[must_use]
pub fn sampling_warning(interval_s: i64) -> Option<String> {
    (interval_s > MAX_SAMPLING_INTERVAL_S).then(|| {
        format!(
            "sampled every {} min: the verdict can be two publications behind",
            interval_s / 60
        )
    })
}
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
    /// It was asked for and the answer failed.
    Failed(String),
}

/// Everything one poll learned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// `serverInfo` name and version.
    pub server: (String, String),
    /// When it was asked, epoch seconds.
    pub at: i64,
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

/// Ask a connected server for every section at `now` (epoch seconds).
pub async fn gather<R, W>(c: &mut Client<R, W>, now: i64) -> Snapshot
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let health = if c.has_tool(HEALTH_LIST) {
        health_part(c).await
    } else {
        Part::Absent(c.absent_reason(HEALTH_LIST, "this host lists no health checks"))
    };
    let vpn = if c.has_tool(EGRESS_TOOL) {
        part(
            c.call_within(EGRESS_TOOL, json!({}), EGRESS_TIMEOUT).await,
            parse_egress,
        )
    } else {
        Part::Absent(c.absent_reason(EGRESS_TOOL, "this host lists no net.egress"))
    };
    let cron = if c.has_tool("cron.status") {
        part(c.call("cron.status", json!({})).await, parse_cron)
    } else {
        Part::Absent(c.absent_reason("cron.status", "this host lists no cron.status"))
    };
    let containers = if c.has_tool("docker.containers") {
        part(
            c.call("docker.containers", json!({})).await,
            parse_containers,
        )
    } else {
        Part::Absent(c.absent_reason("docker.containers", "no Docker on this host"))
    };
    let zfs = if c.has_tool(ZFS_TOOL) {
        part(c.call(ZFS_TOOL, json!({})).await, parse_zfs)
    } else {
        Part::Absent(c.absent_reason(ZFS_TOOL, "this host lists no zfs.health"))
    };
    let disk = if c.has_tool(DISK_TOOL) {
        part(c.call(DISK_TOOL, json!({})).await, parse_disk)
    } else {
        Part::Absent(c.absent_reason(DISK_TOOL, "this host lists no disk.usage"))
    };
    let facts = if c.has_tool(FACTS_TOOL) {
        part(c.call(FACTS_TOOL, json!({})).await, parse_facts)
    } else {
        Part::Absent(c.absent_reason(FACTS_TOOL, "this host lists no host.facts"))
    };
    let alerts = if c.has_tool("alerts.since") {
        part(
            c.call("alerts.since", json!({ "marker": now - ALERT_WINDOW_S }))
                .await,
            parse_alerts,
        )
    } else {
        Part::Absent(c.absent_reason("alerts.since", "this host lists no alerts.since"))
    };
    Snapshot {
        server: c.server.clone(),
        at: now,
        health,
        vpn,
        cron,
        containers,
        zfs,
        disk,
        facts,
        alerts,
    }
}

fn part<T>(
    answer: Result<Value, crate::mcp::client::Error>,
    parse: fn(&Value) -> Result<T, String>,
) -> Part<T> {
    match answer {
        Ok(v) => parse(&v).map_or_else(Part::Failed, Part::Ready),
        Err(e) => Part::Failed(e.to_string()),
    }
}

/// The health section: the machine's PUBLISHED verdict, narrowed to its
/// periodic suite.
///
/// The verdict is READ, never run (servers ROADMAP E12): the estate's own
/// cron runs the suite every 15 minutes and publishes what it found, and
/// running it here would pay the suite's whole cost for an answer the
/// estate already has. The suite's own list is asked too, so the verdict
/// is the suite's - the published table also carries the daily checks,
/// whose verdicts are a day old by design and would pair a count with an
/// age that does not describe it.
///
/// A section that cannot be read says why: an absent verdict is never an
/// all-clear.
async fn health_part<R, W>(c: &mut Client<R, W>) -> Part<Health>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let published = match c.read(HEALTH_URI).await {
        Ok(v) => v,
        Err(e) => return Part::Failed(e.to_string()),
    };
    let listed = match c.call(HEALTH_LIST, json!({})).await {
        Ok(v) => v,
        Err(e) => return Part::Failed(e.to_string()),
    };
    let mut health = match parse_health(&published) {
        Ok(h) => h,
        Err(why) => return Part::Failed(why),
    };
    let suite = match parse_suite(&listed) {
        Ok(s) => s,
        Err(why) => return Part::Failed(why),
    };
    health.narrow_to(&suite);
    Part::Ready(health)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

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
