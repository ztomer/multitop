//! ONE PASS of the Ops poll: gather every host that is DUE, and keep what
//! it found.
//!
//! **THE POLL IS A JOB, NOT A SCREEN.** servers ROADMAP E36: the Ops poll
//! was bound to the Ops VIEW - it ran while a human was looking at the
//! panel and never otherwise - so the estate's fingerprint tools fired 0.3 a
//! day while the shared tools fired 87. The Ops view was shipped, installed,
//! and nothing asked the estate for it.
//!
//! So the poll runs on its own: `multitop --ops-poll` is one pass, and the
//! house's scheduler runs it (a launchd agent, every poll interval). One
//! pass, not a daemon, because a pass that exits cannot be left half-dead:
//! the next pass opens a fresh session, and a session that breaks is
//! dropped and reopened rather than kept.
//!
//! **WHAT A PASS DOES.** For each host that has an `mcp` command: is it
//! DUE? A host is due when it has no answer, or its answer is older than the
//! poll's interval - the same rule the routines client follows, and the
//! same rule the Ops view follows. A host that is not due is left alone:
//! the estate is not asked twice for one card.

use std::future::Future;
use std::ops::DerefMut;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::config::Server;
use crate::mcp::client::Client;
use crate::ops::cache::{self, Cache};

/// What one pass did to one host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPass {
    /// The host's key in the cache.
    pub key: String,
    /// The host's address, for the operator's log.
    pub host: String,
    /// Whether the host was gathered.
    pub gathered: bool,
    /// Whether the host already had a fresh answer, so it was left alone.
    pub fresh: bool,
    /// Why it was not gathered, when it was not.
    pub why: Option<String>,
    /// When the answer the host now has was gathered.
    pub at: i64,
}

/// What one pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pass {
    pub hosts: Vec<HostPass>,
}

impl Pass {
    /// Whether every host ended up with an answer.
    ///
    /// A pass that could not reach a host is a pass that failed, and the
    /// job says so: a poll that silently does nothing is the defect this
    /// whole round is about.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.hosts.iter().all(|h| h.gathered || h.fresh)
    }

    /// The hosts that were gathered.
    pub fn gathered(&self) -> impl Iterator<Item = &HostPass> {
        self.hosts.iter().filter(|h| h.gathered)
    }
}

/// One pass over `servers`: gather every host that is due, and keep it.
///
/// `now` is the epoch second the pass starts at, and `fallback` is the
/// rate for a host this client has never read: every other host is
/// sampled at half the cadence the server PUBLISHED with its verdict
/// (servers ROADMAP E53), and the due rule follows that rate.
///
/// `open` opens a session for a host. The real opener is [`open_ssh`]; a
/// test opens an in-memory server, the same way the poll's own test does.
pub async fn run<S, R, W, F, Fut>(
    servers: &[Server],
    cache: &Cache,
    now: i64,
    fallback: Duration,
    mut open: F,
) -> Pass
where
    F: FnMut(&Server) -> Fut,
    Fut: Future<Output = Result<S, String>>,
    S: DerefMut<Target = Client<R, W>>,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut pass = Pass::default();
    for server in servers {
        let key = cache::host_key(server);
        let kept = cache.read(&key);
        // The rate this host is sampled at: half the cadence the server
        // published, or the fallback for a host never read.
        let every = kept
            .as_ref()
            .map_or(fallback, crate::ops::cache::RawSnapshot::interval);
        if !cache::due(kept.as_ref(), now, every) {
            pass.hosts.push(HostPass {
                key,
                host: server.host.clone(),
                gathered: false,
                fresh: true,
                why: None,
                at: kept.map_or(0, |r| r.at),
            });
            continue;
        }
        let outcome = gather_one(server, cache, now, kept.as_ref(), &mut open).await;
        pass.hosts.push(outcome);
    }
    pass
}

/// The real opener: the host's `mcp_host`, over ssh.
///
/// The server is cloned so the session's future does not borrow the caller's
/// list: the pass holds the list for the whole pass, and a session that
/// borrowed it could not outlive the iteration that opened it.
///
/// The tool list is taken from the kept one when it is the list this server
/// is offering now (servers ROADMAP E61), so a pass does not pull the whole
/// catalogue off the host every time.
///
/// # Errors
///
/// The host has no `mcp` command, or the session could not be opened.
pub async fn open_ssh(server: Server, cache: &Cache) -> Result<crate::mcp::spawn::Running, String> {
    let Some(source) = crate::ops::Source::of(&server) else {
        return Err("this host has no mcp_host command".into());
    };
    let store = cache.catalogue_store(&cache::host_key(&server));
    crate::mcp::spawn::open(
        &server,
        &source,
        crate::tasks::ops_poll::OPS_TIMEOUT,
        &store,
    )
    .await
    .map_err(|e| e.to_string())
}

/// One pass over `servers`, over ssh.
pub async fn run_real(servers: &[Server], cache: &Cache, now: i64, fallback: Duration) -> Pass {
    run(servers, cache, now, fallback, |server: &Server| {
        let server = server.clone();
        let cache = cache.clone();
        async move { open_ssh(server, &cache).await }
    })
    .await
}

/// One host: open a session, gather, keep the answer.
async fn gather_one<S, R, W, F, Fut>(
    server: &Server,
    cache: &Cache,
    now: i64,
    kept: Option<&cache::RawSnapshot>,
    open: &mut F,
) -> HostPass
where
    F: FnMut(&Server) -> Fut,
    Fut: Future<Output = Result<S, String>>,
    S: DerefMut<Target = Client<R, W>>,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let key = cache::host_key(server);
    let mut outcome = HostPass {
        key: key.clone(),
        host: server.host.clone(),
        gathered: false,
        fresh: false,
        why: None,
        at: now,
    };
    let mut session = match open(server).await {
        Ok(s) => s,
        Err(why) => {
            outcome.why = Some(why);
            return outcome;
        }
    };
    // The rate the answer carries is DERIVED from the cadence the server
    // publishes with the verdict - and the gap since the last answer when
    // this client missed its schedule, which is the truth.
    let prev = kept.map_or_else(
        crate::ops::rate::Previous::default,
        crate::ops::cache::RawSnapshot::previous,
    );
    let raw = crate::ops::gather_raw(&mut session, now, prev).await;
    if let Some(why) = session.broken() {
        outcome.why = Some(why.to_string());
        return outcome;
    }
    outcome.at = raw.at;
    match cache.write(&key, &raw) {
        Ok(()) => outcome.gathered = true,
        Err(e) => outcome.why = Some(e),
    }
    outcome
}
