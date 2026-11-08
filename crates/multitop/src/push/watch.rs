//! The transition detector: what turns `--serve`'s live state into alerts.
//!
//! The request model has no events -- a host is just whatever the last packet
//! said -- so this watches each host's coarse state (up, past a threshold,
//! unreachable) and speaks only when it CHANGES, and only once the new state
//! has held for [`CONFIRM_TICKS`] looks in a row: a collector's reconnect or one
//! hot second is not a page at 3 a.m. A steady state, good or bad, says nothing.
//!
//! [`Watcher`] is pure (state in, transitions out); [`spawn`] is the loop that
//! feeds it from the live state and hands each transition to the push hub.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::{Alert, Hub, Kind};
use crate::config::{Config, Server};
use crate::server::{LiveState, SharedState};

/// How often `--serve` looks.
pub const TICK: Duration = Duration::from_secs(15);
/// Looks a new state must hold before it is reported (with [`TICK`], 30 s).
pub const CONFIRM_TICKS: u32 = 2;

/// A host's coarse state, as an alert sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    Up,
    Degraded,
    Down,
}

/// A confirmed change, `from` `None` when the first confirmed state is bad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub host: String,
    pub from: Option<HostState>,
    pub to: HostState,
}

#[derive(Debug, Default)]
struct Track {
    reported: Option<HostState>,
    candidate: Option<HostState>,
    seen: u32,
}

/// Per-host debounce of [`HostState`]s into [`Transition`]s.
#[derive(Debug)]
pub struct Watcher {
    confirm: u32,
    hosts: HashMap<String, Track>,
}

impl Watcher {
    #[must_use]
    pub fn new(confirm: u32) -> Self {
        Self {
            confirm: confirm.max(1),
            hosts: HashMap::new(),
        }
    }

    /// One look at one host. `None` -- not read yet -- neither confirms nor
    /// resets anything: a host the server has not heard from is not news.
    pub fn observe(&mut self, host: &str, state: Option<HostState>) -> Option<Transition> {
        let state = state?;
        let t = self.hosts.entry(host.to_string()).or_default();
        if t.candidate == Some(state) {
            t.seen = t.seen.saturating_add(1);
        } else {
            t.candidate = Some(state);
            t.seen = 1;
        }
        if t.seen < self.confirm || t.reported == Some(state) {
            return None;
        }
        let from = t.reported.replace(state);
        // The first confirmed state is a baseline when it is good: starting the
        // server is not "recovered".
        if from.is_none() && state == HostState::Up {
            return None;
        }
        Some(Transition {
            host: host.to_string(),
            from,
            to: state,
        })
    }
}

/// The state `--serve` holds for `server`: unreachable when its collector has an
/// error, past a threshold when its last snapshot breaches, up otherwise, and
/// `None` before anything was read.
#[must_use]
pub fn host_state(live: &LiveState, server: &Server, cfg: &Config) -> Option<HostState> {
    if live.errors.contains_key(&server.host) {
        return Some(HostState::Down);
    }
    let snap = live.snapshots.get(&server.host)?;
    Some(if crate::health::is_breaching(snap, cfg) {
        HostState::Degraded
    } else {
        HostState::Up
    })
}

/// The alert a transition sends.
#[must_use]
pub fn alert_for(t: &Transition, server: &Server) -> Alert {
    let name = server.name.as_deref().unwrap_or(&server.host);
    let (kind, title, body) = match t.to {
        HostState::Down => (
            Kind::Down,
            format!("{name} is unreachable"),
            format!("multitop lost {} and could not reconnect.", server.host),
        ),
        HostState::Degraded => (
            Kind::Degraded,
            format!("{name} is past a threshold"),
            format!(
                "{} crossed a CPU, memory or disk alert threshold.",
                server.host
            ),
        ),
        HostState::Up => (
            Kind::Recovered,
            format!("{name} recovered"),
            format!(
                "{} is reachable and within its thresholds again.",
                server.host
            ),
        ),
    };
    Alert::new(kind, &server.host, title, body)
}

/// One pass over every host: the alerts to send now.
pub fn pass(
    watcher: &mut Watcher,
    live: &LiveState,
    servers: &[Server],
    cfg: &Config,
) -> Vec<Alert> {
    servers
        .iter()
        .filter_map(|s| {
            watcher
                .observe(&s.host, host_state(live, s, cfg))
                .map(|t| alert_for(&t, s))
        })
        .collect()
}

/// The `--serve` loop: look every [`TICK`], send what changed.
pub fn spawn(live: SharedState, servers: Vec<Server>, cfg: Config, hub: Arc<Hub>) {
    tokio::spawn(async move {
        let mut watcher = Watcher::new(CONFIRM_TICKS);
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            let alerts = {
                let state = live.read().await;
                pass(&mut watcher, &state, &servers, &cfg)
            };
            for alert in alerts {
                let delivery = hub.send(&alert).await;
                if delivery.via.is_none() && !delivery.failed.is_empty() {
                    let why: Vec<String> = delivery
                        .failed
                        .iter()
                        .map(|(t, w)| format!("{}: {w}", t.name()))
                        .collect();
                    eprintln!("[push] {} not delivered ({})", alert.title, why.join("; "));
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
