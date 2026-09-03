use crate::app::Msg;
use crate::config::Server;
use crate::fmt::error_line;
use crate::ssh;
use crate::stream::{connect, next_packet};
use multitop_agent::SortBy;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

/// How long to wait for that sentinel before giving up.
#[must_use]
pub fn spawn_fetch(
    idx: usize,
    gen: u64,
    epoch: u64,
    server: Server,
    dims: (u16, u16),
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let status_tx = tx.clone();
        let notify = move |text: String| {
            let _ = status_tx.try_send(Msg::Status {
                panel: idx,
                gen,
                text,
            });
        };

        let mut stream = match connect(&server, ssh::Mode::Fetch, sort, notify).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(Msg::Status {
                        panel: idx,
                        gen,
                        text: error_line(e),
                    })
                    .await;
                return;
            }
        };

        let mut errbuf = Vec::new();
        let local_version = env!("CARGO_PKG_VERSION");

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            if let multitop_agent::proto::Payload::Hello(hello) = &payload {
                if hello.needs_replacement(local_version) {
                    let _ = tx
                        .send(Msg::AuxLine {
                            panel: idx,
                            gen,
                            line: error_line(format!(
                                "agent version mismatch: remote {} vs local {} — fetch will be retried after replacement",
                                hello.agent_version, local_version
                            )),
                        })
                        .await;
                }
                continue;
            }
            if tx
                .send(Msg::Packet {
                    panel: idx,
                    gen,
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
        for line in errbuf {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(line),
                })
                .await;
        }
    })
}

/// One-shot: renders the Docker view for a panel.
#[must_use]
pub fn spawn_docker(
    idx: usize,
    gen: u64,
    epoch: u64,
    server: Server,
    dims: (u16, u16),
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let status_tx = tx.clone();
        let notify = move |text: String| {
            let _ = status_tx.try_send(Msg::Status {
                panel: idx,
                gen,
                text,
            });
        };

        let mut stream = match connect(&server, ssh::Mode::Docker, sort, notify).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(Msg::Status {
                        panel: idx,
                        gen,
                        text: error_line(e),
                    })
                    .await;
                return;
            }
        };

        let _ = tx
            .send(Msg::AuxBegin {
                panel: idx,
                gen,
                header: None,
            })
            .await;
        let mut errbuf = Vec::new();
        let local_version = env!("CARGO_PKG_VERSION");

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            if let multitop_agent::proto::Payload::Hello(hello) = &payload {
                if hello.needs_replacement(local_version) {
                    let _ = tx
                        .send(Msg::AuxLine {
                            panel: idx,
                            gen,
                            line: error_line(format!(
                                "agent version mismatch: remote {} vs local {} — docker will be retried after replacement",
                                hello.agent_version, local_version
                            )),
                        })
                        .await;
                }
                continue;
            }
            if tx
                .send(Msg::Packet {
                    panel: idx,
                    gen,
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
        for line in errbuf {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(line),
                })
                .await;
        }
    })
}
