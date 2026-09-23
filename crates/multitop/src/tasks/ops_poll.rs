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
/// Reason: the fastest thing it shows moves on cron's clock (health every
/// 30 minutes, `vpn_watchdog` every 3); a poll every 30 s shows a finished
/// job within half a minute and costs the host four small reads.
pub const OPS_POLL: Duration = Duration::from_secs(30);
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
        let snap = gather(s, now()).await;
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
