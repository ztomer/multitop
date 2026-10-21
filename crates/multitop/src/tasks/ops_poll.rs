//! The Ops view's poll.
//!
//! One session per host, kept open across polls (ssh and discovery are paid
//! once), each poll a fresh [`gather`]. A session
//! that breaks is dropped and reopened on the next poll; until then the
//! panel says why, under the last answer it had - never a blank pane, and
//! never old data passed off as current.
//!
//! Generic over how a session is opened, so a test drives it with an
//! in-memory server; `spawn.rs` passes the real ssh opener.

use std::future::Future;
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::Sender;
use tokio::sync::watch;

use crate::app::Msg;
use crate::mcp::client::Client;
use crate::ops::{gather, OpsState, Snapshot};

/// How often a panel in the Ops view asks again.
///
/// **TWICE THE ESTATE'S PUBLICATION RATE, and the number IS the rule.**
/// DECISIONS D15 (owner, 2026-10-20): the estate publishes its health
/// verdict every 15 minutes, so a client that samples at the publication
/// rate can ALIAS - its read lands just before a publication and then waits
/// a whole cycle, and its verdict is two publications behind the truth.
/// Half the publication interval (7.5 minutes) is the fastest a client may
/// go and still never be two publications behind, so that is what this
/// client does.
///
/// This was 30 s, and the 30 s rate is retired. It gathered 2,880 times a
/// day - TEN times what the rule needs - and every gather moved the whole
/// tool catalogue off the host (134,840 B on `.33` alone, 388 MB a day).
/// A verdict published every 15 minutes cannot be fresher than its
/// publication, so ten reads a minute bought nothing and cost the estate
/// the most it spends on anything.
///
/// The interval is DERIVED from the rule, not written next to it: two
/// numbers that must agree cannot drift apart if one of them is the other.
pub const OPS_POLL: Duration = Duration::from_secs(crate::ops::MAX_SAMPLING_INTERVAL_S as u64);
/// How long each request may take.
/// Reason: `alerts.since` reads a day of logs; every other answer is a
/// file read. A host that takes longer than this is reported as slow.
pub const OPS_TIMEOUT: Duration = Duration::from_secs(20);

/// Epoch seconds now.
#[must_use]
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The poll's interval in whole seconds, for the answer that carries it.
///
/// The answer says how often it was sampled, so a client that samples too
/// slowly can say so where its verdict is shown (DECISIONS D15).
fn sampling_interval_s(every: Duration) -> i64 {
    every.as_secs().try_into().unwrap_or(i64::MAX)
}

/// Where a poll's answers go.
pub struct Target {
    pub panel: usize,
    pub gen: u64,
    pub tx: Sender<Msg>,
    pub dims: Arc<watch::Receiver<(u16, u16)>>,
}

impl Target {
    /// Send `state`; false when nobody is listening any more.
    async fn send(&self, state: OpsState) -> bool {
        let dims = *self.dims.borrow();
        self.tx
            .send(Msg::Ops {
                panel: self.panel,
                gen: self.gen,
                state: Box::new(state),
                dims,
            })
            .await
            .is_ok()
    }
}

/// Poll forever (until aborted, or the app stops listening): open with
/// `connect` when there is no session, gather, send, wait `every`.
pub async fn poll<S, R, W, F, Fut>(to: Target, mut connect: F, every: Duration, now: fn() -> i64)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<S, String>>,
    S: DerefMut<Target = Client<R, W>>,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut session: Option<S> = None;
    let mut last: Option<Box<Snapshot>> = None;
    loop {
        if session.is_none() {
            match connect().await {
                Ok(s) => session = Some(s),
                Err(why) => {
                    if !to
                        .send(OpsState::Failed {
                            why,
                            last: last.clone(),
                        })
                        .await
                    {
                        return;
                    }
                    tokio::time::sleep(every).await;
                    continue;
                }
            }
        }
        let Some(s) = session.as_mut() else {
            continue;
        };
        let snap = gather(s, now(), sampling_interval_s(every)).await;
        let broken = s.broken().map(ToString::to_string);
        let state = if let Some(why) = broken {
            session = None;
            OpsState::Failed {
                why,
                last: last.clone(),
            }
        } else {
            let snap = Box::new(snap);
            last = Some(snap.clone());
            OpsState::Ready(snap)
        };
        if !to.send(state).await {
            return;
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
#[path = "ops_poll_tests.rs"]
mod tests;
