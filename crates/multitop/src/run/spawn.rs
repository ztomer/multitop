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

use super::RECONNECT_BACKOFF;
use multitop_agent::proto::Payload;

/// One status line for a panel, stamped with the generation this task was
/// started for so a later panel list rejects it.
fn frame(idx: usize, epoch: u64, line: String) -> super::Msg {
    super::Msg::Frame {
        panel: idx,
        epoch,
        lines: vec![line],
    }
}

pub fn spawn_monitor(
    idx: usize,
    epoch: u64,
    server: Server,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    sort: super::SortBy,
    tx: tokio::sync::mpsc::Sender<super::Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let local_version = env!("CARGO_PKG_VERSION");
        let mut failures = 0usize;
        loop {
            let status_tx = tx.clone();
            let notify = move |text: String| {
                let _ = status_tx.try_send(frame(idx, epoch, text));
            };

            match stream::connect(&server, Mode::Monitor, sort, notify).await {
                Ok(mut stream) => {
                    failures = 0;
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
                                        .send(super::Msg::Frame {
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
                        if tx
                            .send(super::Msg::Packet {
                                panel: idx,
                                gen: 0,
                                payload,
                                dims,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }

                    let detail = errbuf
                        .last()
                        .cloned()
                        .unwrap_or_else(|| format!("Connection to {} closed", server.host));
                    let _ = tx
                        .send(super::Msg::Frame {
                            panel: idx,
                            epoch,
                            lines: vec![error_line(detail)],
                        })
                        .await;

                    if mismatched {
                        if let Some(arch) = ssh::probe_remote_arch(&server).await {
                            let token = format!("{}", std::process::id());
                            if ssh::upload_agent(&server, arch, &token).await.is_ok() {
                                let _ = tx
                                    .send(super::Msg::Frame {
                                        panel: idx,
                                        epoch,
                                        lines: vec![format!(
                                            "\u{2713} agent replaced on {}",
                                            server.host
                                        )],
                                    })
                                    .await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(frame(idx, epoch, error_line(e))).await;
                }
            }

            let wait = RECONNECT_BACKOFF[failures.min(RECONNECT_BACKOFF.len() - 1)];
            failures += 1;
            sleep(Duration::from_secs(wait)).await;
        }
    })
}
