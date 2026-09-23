//! The Ops view's data (servers ROADMAP 12.17b).
//!
//! What a host's `mcp_host` says about the things the agent does not stream:
//! health verdicts, cron jobs, container health and the alerts its cron
//! wrapper logged.
//!
//! Four sections, each read from one answer and each able to say why it is
//! not there: a host that lists no `docker.containers` has no Docker (an
//! honest absence, not an error), and one section failing never hides the
//! other three. Health comes from `health://latest` - each check's last
//! verdict, as cron recorded it - never from `health.run_all`, which runs
//! the whole suite (25 s on .33) and must not be a poll.

pub mod render;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::mcp::client::Client;

/// The resource holding each health check's last recorded verdict.
pub const HEALTH_URI: &str = "health://latest";
/// How far back the alerts section looks.
/// Reason: one day is the window an operator glancing at a board cares
/// about; older events are in `alerts.since` for whoever asks.
pub const ALERT_WINDOW_S: i64 = 86_400;

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

/// Ask a connected server for all four sections at `now` (epoch seconds).
pub async fn gather<R, W>(c: &mut Client<R, W>, now: i64) -> Snapshot
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let health = if c.has_tool("health.run") {
        part(c.read(HEALTH_URI).await, parse_health)
    } else {
        Part::Absent("this host lists no health checks".to_string())
    };
    let cron = if c.has_tool("cron.status") {
        part(c.call("cron.status", json!({})).await, parse_cron)
    } else {
        Part::Absent("this host lists no cron.status".to_string())
    };
    let containers = if c.has_tool("docker.containers") {
        part(
            c.call("docker.containers", json!({})).await,
            parse_containers,
        )
    } else {
        Part::Absent("no Docker on this host".to_string())
    };
    let alerts = if c.has_tool("alerts.since") {
        part(
            c.call("alerts.since", json!({ "marker": now - ALERT_WINDOW_S }))
                .await,
            parse_alerts,
        )
    } else {
        Part::Absent("this host lists no alerts.since".to_string())
    };
    Snapshot {
        server: c.server.clone(),
        at: now,
        health,
        cron,
        containers,
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
