use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::app::{error_line, header_line, status_line, Msg};
use crate::config::Server;
use crate::run::{connect, next_line, Stream};
use crate::ssh;

/// One-shot: renders the Docker view for a panel.
pub fn spawn_docker(
    idx: usize,
    gen: u64,
    server: Server,
    dims: (u16, u16),
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

        let mut stream = match connect(&server, ssh::Mode::Docker, dims, notify).await {
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
        while let Ok(Some(line)) = next_line(&mut stream, &mut errbuf).await {
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
        let mut stream = Stream {
            _child: child,
            stdout: BufReader::new(stdout).lines(),
            stderr: BufReader::new(stderr).lines(),
            pending: None,
        };

        let header = header_line(format!("Upgrade on {}", server.host));
        let _ = tx
            .send(Msg::AuxBegin {
                panel: idx,
                gen,
                header: Some(header),
            })
            .await;

        let mut errbuf = Vec::new();
        while let Ok(Some(line)) = next_line(&mut stream, &mut errbuf).await {
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
