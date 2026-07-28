use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::app::{error_line, header_line, status_line, Msg};
use crate::config::Server;
use crate::run::{connect, next_packet};
use crate::ssh;

use multitop_agent::SortBy;

/// One-shot: renders the Docker view for a panel.
pub fn spawn_docker(
    idx: usize,
    gen: u64,
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
        let pal = &multitop_agent::color::ANSI;

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            let lines = match payload {
                multitop_agent::proto::Payload::Docker { host, rows } => {
                    multitop_agent::docker::render(&host, dims.0 as usize, dims.1 as usize, &rows, pal, sort)
                }
                multitop_agent::proto::Payload::Monitor(snap) => {
                    multitop_agent::render::render(&snap, dims.0 as usize, dims.1 as usize, multitop_agent::render::bar_len_for(dims.0 as usize), pal)
                }
            };

            for line in lines {
                if tx
                    .send(Msg::AuxLine {
                        panel: idx,
                        gen,
                        line,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
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

/// One-shot: runs the server's `upgrade_cmd`, streaming its output.
pub fn spawn_upgrade(idx: usize, gen: u64, server: Server, tx: Sender<Msg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(command) = server.upgrade_cmd.clone() else {
            return;
        };

        let mut child = match ssh::spawn_command(&server, &command) {
            Ok(c) => c,
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
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdout_lines = BufReader::new(stdout).lines();
        let mut stderr_lines = BufReader::new(stderr).lines();

        let header = header_line(format!("Upgrade on {}", server.host));
        let _ = tx
            .send(Msg::AuxBegin {
                panel: idx,
                gen,
                header: Some(header),
            })
            .await;

        let mut errbuf = Vec::new();
        loop {
            tokio::select! {
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if tx.send(Msg::AuxLine { panel: idx, gen, line }).await.is_err() {
                                return;
                            }
                        }
                        _ => break,
                    }
                }
                Ok(Some(line)) = stderr_lines.next_line() => {
                    if !line.trim().is_empty() {
                        errbuf.push(line);
                    }
                }
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
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(status_line("\u{2500} done")),
            })
            .await;
    })
}
