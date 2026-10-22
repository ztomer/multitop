//! THE AGENT: the Ops poll, in a loop, paced by the cadence the estate
//! publishes.
//!
//! servers ROADMAP E53: the installed agent's tick was a number this repo
//! carried, and launchd fixes an interval when the job is loaded - so the
//! tick could not follow the estate, and a publication that moved left the
//! agent reading too slowly, with the client's own warning firing on every
//! tick and nothing anyone could do about it but reinstall.
//!
//! So the agent paces ITSELF. Every cycle it runs one pass - a fresh
//! session per host, dropped rather than kept, the way the pass has always
//! worked - and then waits the tick that the cadence the estate published
//! implies ([`crate::ops::tick`]). The install template therefore installs
//! no interval at all: there is no rate in this repo for the estate's
//! cadence to move under.
//!
//! **A PASS IS STILL A PASS.** The loop schedules passes; it does not keep
//! one open. The property the pass was built for - "a pass that exits
//! cannot be left half-dead" - is the agent's too: each cycle opens a fresh
//! session, and a session that breaks is dropped rather than kept.
//!
//! **THE TICK IS STATED, AND SO IS A CADENCE THAT COULD NOT BE READ.** The
//! log says how long the agent will wait and where that number came from,
//! and a host whose cadence could not be read is named, with the rate the
//! agent is keeping for it. Nothing is silently replaced by a number this
//! repo carries, and nothing is called a published rate that was not
//! published.

use std::future::Future;
use std::ops::DerefMut;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::config::Server;
use crate::mcp::client::Client;
use crate::ops::cache::Cache;
use crate::ops::pass::{self, Pass};
use crate::ops::tick::{self, Tick};

/// One cycle: the pass, and the tick that follows it.
#[derive(Clone, Debug)]
pub struct Cycle {
    /// What the pass found.
    pub pass: Pass,
    /// The tick: the period the agent keeps.
    pub tick: Tick,
    /// How long the pass took.
    pub took: Duration,
    /// How long the agent waits before the next pass.
    ///
    /// The tick is the PERIOD between passes, so the pass's own time is
    /// taken off the wait. A pass that took longer than the tick waits not
    /// at all - the agent is behind, and the log says so in the rate the
    /// answer carries.
    pub wait: Duration,
}

/// One cycle: the pass, then the tick.
pub async fn cycle<S, R, W, F, Fut>(
    servers: &[Server],
    cache: &Cache,
    now: fn() -> i64,
    open: &mut F,
) -> Cycle
where
    F: FnMut(&Server) -> Fut,
    Fut: Future<Output = Result<S, String>>,
    S: DerefMut<Target = Client<R, W>>,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let started = Instant::now();
    let pass = pass::run(
        servers,
        cache,
        now(),
        crate::tasks::ops_poll::OPS_POLL,
        open,
    )
    .await;
    let took = started.elapsed();
    let tick = tick::tick_for(servers, cache);
    let spent = i64::try_from(took.as_secs()).unwrap_or(i64::MAX);
    let wait = Duration::from_secs(u64::try_from(tick.secs.saturating_sub(spent)).unwrap_or(0));
    Cycle {
        pass,
        tick,
        took,
        wait,
    }
}

/// What the agent says about the cycle it just ran.
///
/// The pass's own lines first - what each host answered - then the hosts
/// whose cadence could not be read, and last the tick: how long until the
/// next pass, and where that number came from.
#[must_use]
pub fn lines(cycle: &Cycle, now: i64) -> Vec<String> {
    let mut out: Vec<String> = cycle
        .pass
        .hosts
        .iter()
        .map(|host| format!("{} · {}", host.host, pass::said(host)))
        .collect();
    for host in &cycle.tick.unread {
        out.push(format!("{} · {}", host.host, host.note()));
    }
    out.push(cycle.tick.line(now));
    out
}

/// The agent's loop: a pass, the tick's wait, and again.
///
/// It does not return while the agent is healthy: the agent runs until it is
/// stopped. It returns when the installed binary has been replaced under it
/// and the new one could not be started - the one failure the agent cannot
/// go on with.
pub async fn run<S, R, W, F, Fut>(
    servers: &[Server],
    cache: &Cache,
    now: fn() -> i64,
    open: &mut F,
) -> ExitCode
where
    F: FnMut(&Server) -> Fut,
    Fut: Future<Output = Result<S, String>>,
    S: DerefMut<Target = Client<R, W>>,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let binary = std::env::current_exe().ok();
    let started = binary.as_deref().and_then(stamp_of);
    loop {
        let cycle = cycle(servers, cache, now, open).await;
        for line in lines(&cycle, now()) {
            println!("{line}");
        }
        // The binary was replaced under this agent: hand over to the new
        // one, so the estate is read by the binary that is installed.
        if let Some(binary) = binary.as_deref() {
            if replaced(binary, started) {
                println!("the installed binary was replaced: this agent is now the new one");
                return hand_over(binary);
            }
        }
        tokio::time::sleep(cycle.wait).await;
    }
}

/// When the binary on disk was last changed.
fn stamp_of(binary: &Path) -> Option<SystemTime> {
    std::fs::metadata(binary).and_then(|m| m.modified()).ok()
}

/// Whether the binary on disk is not the one this agent started from.
///
/// The agent is a long-lived process - that is what lets it pace itself by
/// the published cadence - so an upgraded binary would otherwise leave it
/// running the old one, and the old one keeps the old rule. The agent
/// re-execs itself when the binary changes, so the estate is read by the
/// binary that is installed.
#[must_use]
pub fn replaced(binary: &Path, started: Option<SystemTime>) -> bool {
    let Some(started) = started else {
        return false;
    };
    stamp_of(binary).is_some_and(|now| now != started)
}

/// Hand this process over to the binary that is now installed.
///
/// `exec` REPLACES this process, so the launchd job keeps its identity and
/// the estate is read by the new binary from the next tick. `exec` only
/// returns when it failed - and then the agent cannot go on as the thing
/// the operator installed, so it says so and lets launchd try again.
fn hand_over(binary: &Path) -> ExitCode {
    let err = std::process::Command::new(binary).arg("--ops-agent").exec();
    eprintln!("[Error] the new binary could not be run: {err}");
    ExitCode::FAILURE
}

/// The agent, over ssh.
pub async fn run_real(servers: &[Server], cache: &Cache) -> ExitCode {
    let mut open = |server: &Server| {
        let server = server.clone();
        let cache = cache.clone();
        async move { pass::open_ssh(server, &cache).await }
    };
    run(servers, cache, crate::tasks::ops_poll::now_epoch, &mut open).await
}

#[cfg(test)]
#[path = "ops_agent_tests.rs"]
mod tests;
