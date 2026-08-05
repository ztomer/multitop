use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::config::Server;
use crate::fmt::error_line;
use crate::ssh;
use crate::ssh::Mode;
use crate::stream;

use super::{reconnect_wait, SessionOutcome};
use multitop_agent::proto::Payload;
use multitop_agent::SortBy;

use crate::app::Msg;

/// One status line for a panel, stamped with the generation this task was
/// started for so a later panel list rejects it.
fn frame(idx: usize, epoch: u64, line: String) -> Msg {
    Msg::Frame {
        panel: idx,
        epoch,
        lines: vec![line],
    }
}

/// Replace the agent on a host whose version did not match ours, and say what
/// happened -- either way.
///
/// A `Result` rather than an `Option` and a silent else, because there is
/// exactly one caller and it reports both arms through one send. Nothing here
/// can fail quietly: a mismatch that cannot be repaired repeats on every
/// reconnect for the rest of the session, so the reason it cannot be repaired
/// is the only thing that makes the loop legible.
async fn replace_agent(server: &Server) -> Result<String, String> {
    // A local panel does not run `ssh` at all -- it spawns the agent binary
    // directly -- so there is nothing to upload and nowhere to upload it. The
    // old code sent `ssh` at `localhost:0` and threw the failure away.
    if ssh::is_local(server) {
        return Err(
            "the local agent is a different version and cannot be replaced over SSH -- \
             a stale multitop-agent is ahead of this build on PATH, or beside it; \
             remove it or rebuild with ./build.sh"
                .to_string(),
        );
    }
    let Some(arch) = ssh::probe_remote_arch(server).await else {
        return Err(format!(
            "could not read the architecture of {} to replace its agent -- \
             the version mismatch will repeat on every reconnect until it is replaced by hand",
            server.host
        ));
    };
    let token = format!("{}", std::process::id());
    ssh::upload_agent(server, arch, &token).await?;
    Ok(format!("\u{2713} agent replaced on {}", server.host))
}

#[must_use]
pub fn spawn_monitor(
    idx: usize,
    epoch: u64,
    server: Server,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    sort: SortBy,
    tx: tokio::sync::mpsc::Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let local_version = env!("CARGO_PKG_VERSION");
        let mut failures = 0usize;
        loop {
            let status_tx = tx.clone();
            let notify = move |text: String| {
                let _ = status_tx.try_send(frame(idx, epoch, text));
            };

            let outcome = match stream::connect(&server, Mode::Monitor, sort, notify).await {
                Ok(mut stream) => {
                    // Not `failures = 0` here. Connecting is not progress: the
                    // session has delivered nothing yet, and the failures this
                    // backoff exists for are exactly the ones that happen after
                    // the connection is accepted.
                    let mut delivered = false;
                    let mut errbuf = Vec::new();
                    let mut version_checked = false;
                    let mut mismatched = false;
                    while let Ok(Some(payload)) =
                        stream::next_packet(&mut stream, &mut errbuf).await
                    {
                        if let Payload::Monitor(snap) = &payload {
                            if !version_checked {
                                version_checked = true;
                                if !snap.agent_version.is_empty()
                                    && snap.agent_version != local_version
                                {
                                    let _ = tx
                                        .send(Msg::Frame {
                                            panel: idx,
                                            epoch,
                                            lines: vec![format!(
                                                "\u{2192} agent version mismatch: \
                             remote {} vs local {}, replacing...",
                                                snap.agent_version, local_version
                                            )],
                                        })
                                        .await;
                                    mismatched = true;
                                    break;
                                }
                            }
                        }
                        let dims = *dims_rx.borrow();
                        delivered = true;
                        if tx
                            .send(Msg::Packet {
                                panel: idx,
                                gen: 0,
                                epoch,
                                payload,
                                dims,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }

                    // Not when the break above was ours. The version-mismatch
                    // branch ends the session on purpose, and reporting that as
                    // `Connection to <host> closed` put a failure the host never
                    // had between "replacing..." and "agent replaced" -- the one
                    // line in the sequence that is not true.
                    if !mismatched {
                        let detail = errbuf
                            .last()
                            .cloned()
                            .unwrap_or_else(|| format!("Connection to {} closed", server.host));
                        let _ = tx
                            .send(Msg::Frame {
                                panel: idx,
                                epoch,
                                lines: vec![error_line(detail)],
                            })
                            .await;
                    }

                    if mismatched {
                        // One send, whatever happened. Both failure paths used
                        // to be silent -- `probe_remote_arch` returning `None`
                        // and `upload_agent` returning `Err` were both swallowed
                        // by an `if ... .is_ok()` -- so a mismatch that could not
                        // be repaired left the panel saying "replacing..." and
                        // then nothing, forever, once every backoff interval.
                        // `upload_agent`'s own message ("No aarch64 agent was
                        // built into this binary. Rebuild with ./build.sh") is
                        // written to be acted on, and was the message being
                        // thrown away.
                        let line = match replace_agent(&server).await {
                            Ok(note) => note,
                            Err(reason) => error_line(reason),
                        };
                        let _ = tx
                            .send(Msg::Frame {
                                panel: idx,
                                epoch,
                                lines: vec![line],
                            })
                            .await;
                    }
                    if delivered {
                        SessionOutcome::Delivered
                    } else {
                        SessionOutcome::NoData
                    }
                }
                Err(e) => {
                    let _ = tx.send(frame(idx, epoch, error_line(e))).await;
                    SessionOutcome::NeverConnected
                }
            };

            let wait = reconnect_wait(outcome, &mut failures);
            sleep(Duration::from_secs(wait)).await;
        }
    })
}

#[cfg(test)]
mod replace_agent_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::replace_agent;
    use crate::config::Server;

    fn server(host: &str, port: u16) -> Server {
        Server {
            host: host.to_string(),
            port,
            user: "admin".to_string(),
            upgrade_cmd: None,
        }
    }

    /// A local panel has no SSH session to replace an agent over, and saying
    /// nothing left the panel repeating "agent version mismatch ... replacing"
    /// once per backoff interval for the rest of the session with no hint that
    /// the replacement could never happen.
    ///
    /// This arm reaches no network, which is the whole reason it is the one
    /// under test: the other two need a host.
    #[tokio::test]
    async fn a_local_panel_says_why_it_cannot_replace_its_agent() {
        let reason = replace_agent(&server("localhost", 0))
            .await
            .expect_err("a local panel cannot be replaced over SSH");
        assert!(
            reason.contains("multitop-agent"),
            "the message must name the binary to remove: {reason}"
        );
        assert!(
            !reason.is_empty(),
            "and must exist at all -- silence is the defect"
        );
    }
}
