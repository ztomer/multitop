//! The Ops view's data (servers ROADMAP 12.17b).
//!
//! What a host's `mcp_host` says about the things the agent does not stream:
//! health verdicts, cron jobs, container health, ZFS pools and the alerts
//! its cron wrapper logged.
//!
//! Five sections, each read from one answer and each able to say why it is
//! not there: a host that lists no `docker.containers` has no Docker (an
//! honest absence, not an error), and one section failing never hides the
//! other four. Health comes from `health://latest` - each check's last
//! verdict, as cron recorded it - never from `health.run_all`, which runs
//! the whole suite (25 s on .33) and must not be a poll.
//!
//! Every section is decided the same way: the tool the host LISTS is asked,
//! and a tool the host does not list is an absence carrying the server's own
//! reason for it (`server/discover`'s `dev.multitop/absent`), never a guess
//! and never a green "none". The ZFS section is no exception: `zfs.health`
//! is the tool that owns the pools' state, so the view reads its answer and
//! never runs `zpool status` itself (servers ROADMAP O82).

pub mod render;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::mcp::client::Client;

/// The resource holding each health check's last recorded verdict.
pub const HEALTH_URI: &str = "health://latest";
/// The tool that answers about a host's ZFS pools: their state, their vdev
/// tree, and what ZFS says to do about them.
pub const ZFS_TOOL: &str = "zfs.health";
/// How far back the alerts section looks.
/// Reason: one day is the window an operator glancing at a board cares
/// about; older events are in `alerts.since` for whoever asks.
pub const ALERT_WINDOW_S: i64 = 86_400;
/// The state a pool or a vdev is in when nothing is wrong with it.
const ONLINE: &str = "ONLINE";

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

/// One section: its data, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part<T> {
    Ready(T),
    /// The host does not offer it (a stated status, not a fault).
    Absent(String),
    /// It was asked for and the answer failed.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub rc: i64,
    pub ts: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub label: String,
    pub last_start: Option<i64>,
    pub last_rc: Option<i64>,
    pub running_since: Option<i64>,
    pub alert_since: Option<i64>,
}

impl Job {
    /// Its last run failed, or it is alerting.
    #[must_use]
    pub fn in_trouble(&self) -> bool {
        self.last_rc.is_some_and(|rc| rc != 0) || self.alert_since.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub name: String,
    pub state: String,
    pub status: String,
    pub health: String,
    pub compose_project: Option<String>,
}

impl Container {
    /// Unhealthy, or not running when it should be. A container outside any
    /// compose project that exited 0 is a finished one-shot (a `docker run`
    /// that did its job), not a stopped service - the same rule as the
    /// routines follower's.
    #[must_use]
    pub fn in_trouble(&self) -> bool {
        if self.health == "unhealthy" {
            return true;
        }
        if self.state == "running" {
            return false;
        }
        !(self.compose_project.is_none() && self.status.starts_with("Exited (0)"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub ts: i64,
    pub subject: String,
}

/// One member of a pool's vdev tree, as `zfs.health` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The vdev's name as the pool knows it (`sdb`, `mirror-0`, ...).
    pub name: String,
    /// `ONLINE`, `DEGRADED`, `REMOVED`, `FAULTED`, `UNAVAIL`, ...
    pub state: String,
}

impl Member {
    /// A member that is not ONLINE.
    #[must_use]
    pub fn is_faulted(&self) -> bool {
        self.state != ONLINE
    }
}

/// One ZFS pool, as `zfs.health` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    /// The pool's name.
    pub name: String,
    /// `ONLINE`, `DEGRADED`, `SUSPENDED`, `UNAVAIL`, ...
    pub state: String,
    /// What ZFS says to do about it, when it says anything.
    pub action: String,
    /// ZFS's explanation, when it has one.
    pub msg: String,
    /// The vdev tree, the pool's children first.
    pub members: Vec<Member>,
}

impl Pool {
    /// The members that are not ONLINE.
    ///
    /// The pool's OWN row is not in the member list (it is the pool, not a
    /// vdev of it), so a DEGRADED `mirror-0` is a faulted member and is
    /// named - it is the level that says the pool lost redundancy.
    #[must_use]
    pub fn faulted_members(&self) -> Vec<&Member> {
        self.members.iter().filter(|m| m.is_faulted()).collect()
    }

    /// Whether the pool is whole: ONLINE itself, and every member ONLINE.
    ///
    /// The estate's rule, the one `check_zfs_pool` fails on and the tool's
    /// own summary uses: a pool that is not whole is invisible to every
    /// other view of the host (a SUSPENDED pool's I/O does not happen, so
    /// writes BLOCK rather than fail, and `disk.usage` still reports a
    /// plausible size for it).
    #[must_use]
    pub fn is_whole(&self) -> bool {
        self.state == ONLINE && self.faulted_members().is_empty()
    }
}

/// Everything one poll learned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// `serverInfo` name and version.
    pub server: (String, String),
    /// When it was asked, epoch seconds.
    pub at: i64,
    pub health: Part<Vec<Check>>,
    pub cron: Part<Vec<Job>>,
    pub containers: Part<Vec<Container>>,
    pub zfs: Part<Vec<Pool>>,
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
        if let Part::Ready(c) = &self.health {
            t.extend(c.iter().map(|c| c.name.as_str()));
        }
        if let Part::Ready(j) = &self.cron {
            t.extend(j.iter().map(|j| j.label.as_str()));
        }
        if let Part::Ready(c) = &self.containers {
            t.extend(c.iter().flat_map(|c| [c.name.as_str(), c.status.as_str()]));
        }
        if let Part::Ready(p) = &self.zfs {
            t.extend(p.iter().flat_map(|p| {
                std::iter::once(p.name.as_str()).chain(p.members.iter().map(|m| m.name.as_str()))
            }));
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

fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn list<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>, String> {
    v.get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("the answer has no `{k}` list"))
}

/// `health://latest`: `{checks: [{check, ok, rc, ms, ts}]}`.
///
/// # Errors
///
/// The answer has no `checks` list.
pub fn parse_health(v: &Value) -> Result<Vec<Check>, String> {
    Ok(list(v, "checks")?
        .iter()
        .map(|c| Check {
            name: text(c, "check"),
            ok: c.get("ok").and_then(Value::as_bool).unwrap_or(false),
            rc: c.get("rc").and_then(Value::as_i64).unwrap_or_default(),
            ts: c.get("ts").and_then(Value::as_i64).unwrap_or_default(),
        })
        .collect())
}

/// `cron.status`: `{jobs: [{label, last_start, last_rc, running_since, alert_since, ...}]}`.
///
/// # Errors
///
/// The answer has no `jobs` list.
pub fn parse_cron(v: &Value) -> Result<Vec<Job>, String> {
    let int = |j: &Value, k: &str| j.get(k).and_then(Value::as_i64);
    Ok(list(v, "jobs")?
        .iter()
        .map(|j| Job {
            label: text(j, "label"),
            last_start: int(j, "last_start"),
            last_rc: int(j, "last_rc"),
            running_since: int(j, "running_since"),
            alert_since: int(j, "alert_since"),
        })
        .collect())
}

/// `docker.containers`: `{containers: [{name, state, status, health, compose_project, ...}]}`.
///
/// # Errors
///
/// The answer has no `containers` list.
pub fn parse_containers(v: &Value) -> Result<Vec<Container>, String> {
    Ok(list(v, "containers")?
        .iter()
        .map(|c| Container {
            name: text(c, "name"),
            state: text(c, "state"),
            status: text(c, "status"),
            health: text(c, "health"),
            compose_project: c
                .get("compose_project")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
        })
        .collect())
}

/// `alerts.since`: `{events: [{ts, subject, ...}], marker}`. `ts` arrives
/// as a string or a number.
///
/// # Errors
///
/// The answer has no `events` list.
pub fn parse_alerts(v: &Value) -> Result<Vec<Alert>, String> {
    Ok(list(v, "events")?
        .iter()
        .map(|e| Alert {
            ts: e
                .get("ts")
                .and_then(|t| {
                    t.as_i64()
                        .or_else(|| t.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or_default(),
            subject: text(e, "subject"),
        })
        .collect())
}

/// `zfs.health`: `{pools: [{name, state, action, msg, scan, members: [{name,
/// state, read, write, cksum}]}]}`.
///
/// The pool's own state and its vdev tree, as the tool publishes them: the
/// view reads them, and never runs `zpool status` itself.
///
/// The error counters are not read, and that is the estate's rule rather than
/// an omission: a pool is whole when its state and every member's state say
/// so, and `check_zfs_pool` (which owns that verdict) fails on the states,
/// not on a counter over a member that is still ONLINE.
///
/// # Errors
///
/// The answer has no `pools` list, or a pool has no `members` list (a pool
/// whose vdev tree could not be read is not a pool that may be called whole).
pub fn parse_zfs(v: &Value) -> Result<Vec<Pool>, String> {
    list(v, "pools")?
        .iter()
        .map(|p| {
            let members = list(p, "members").map_err(|_| {
                format!(
                    "pool {} has no `members` list",
                    p.get("name").and_then(Value::as_str).unwrap_or_default()
                )
            })?;
            Ok(Pool {
                name: text(p, "name"),
                state: text(p, "state"),
                action: text(p, "action"),
                msg: text(p, "msg"),
                members: members
                    .iter()
                    .map(|m| Member {
                        name: text(m, "name"),
                        state: text(m, "state"),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Ask a connected server for all five sections at `now` (epoch seconds).
pub async fn gather<R, W>(c: &mut Client<R, W>, now: i64) -> Snapshot
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let health = if c.has_tool("health.run") {
        part(c.read(HEALTH_URI).await, parse_health)
    } else {
        Part::Absent(c.absent_reason("health.run", "this host lists no health checks"))
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
        cron,
        containers,
        zfs,
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

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "zfs_tests.rs"]
mod zfs_tests;
