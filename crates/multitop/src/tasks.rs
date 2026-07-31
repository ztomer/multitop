use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::app::Msg;
use crate::config::Server;
use crate::fmt::{error_line, header_line, status_line};
use crate::ssh;
use crate::stream::{connect, next_packet};

use multitop_agent::SortBy;

pub fn spawn_fetch(
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

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            if tx
                .send(Msg::Packet {
                    panel: idx,
                    gen,
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

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            if tx
                .send(Msg::Packet {
                    panel: idx,
                    gen,
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

/// One-shot: runs the server's `upgrade_cmd`, streaming its output.
pub fn spawn_upgrade(
    idx: usize,
    gen: u64,
    server: Server,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(command) = server.upgrade_cmd.clone() else {
            return;
        };

        let mut child = match ssh::spawn_command(&server, &command, pass.as_deref()) {
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

        let mut sudo_help = false;
        let mut errbuf = Vec::new();
        loop {
            tokio::select! {
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            for part in line.split('\r') {
                                let clean = part.trim_end_matches('\r');
                                if !clean.is_empty() {
                                    let lower = clean.to_lowercase();
                                    if lower.contains("sudo") && (lower.contains("terminal") || lower.contains("password") || lower.contains("pre-authorized") || lower.contains("tty") || lower.contains("prompt on")) {
                                        sudo_help = true;
                                    }
                                    if tx.send(Msg::AuxLine { panel: idx, gen, line: clean.to_string() }).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        _ => break,
                    }
                }
                Ok(Some(line)) = stderr_lines.next_line() => {
                    for part in line.split('\r') {
                        let clean = part.trim_end_matches('\r');
                        let trimmed = clean.trim();
                        if !trimmed.is_empty() {
                            let lower = trimmed.to_lowercase();
                            if lower.contains("sudo") && (lower.contains("terminal") || lower.contains("password") || lower.contains("pre-authorized") || lower.contains("tty") || lower.contains("prompt on")) {
                                sudo_help = true;
                            }
                            if lower.contains("shared connection to") || (lower.contains("connection to") && lower.contains("closed")) {
                                continue;
                            }
                            if errbuf.len() >= 100 {
                                errbuf.remove(0);
                            }
                            errbuf.push(clean.to_string());
                        }
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
        if sudo_help {
            if pass.is_none() {
                let _ = tx
                    .send(Msg::AuxLine {
                        panel: idx,
                        gen,
                        line: "\x1b[33m\u{2192} Tip: Set password in settings ('e') to allow upgrades\x1b[0m".to_string(),
                    })
                    .await;
            } else {
                let _ = tx
                    .send(Msg::AuxLine {
                        panel: idx,
                        gen,
                        line: "\x1b[33m\u{2192} Tip: Check password in settings ('e') or sudoer permissions\x1b[0m".to_string(),
                    })
                    .await;
            }
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: "\x1b[33m\u{2192} Tip: Add '<user> ALL=(ALL) NOPASSWD: ALL' to /etc/sudoers for passwordless sudo\x1b[0m".to_string(),
                })
                .await;
        }
        let exit_status = child.wait().await;
        let success = exit_status.is_ok_and(|s| s.success());
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: if success {
                    Some(status_line("\u{2500} done"))
                } else {
                    Some(status_line(
                        "\u{26A0} disconnected (upgrade may be incomplete)",
                    ))
                },
                success,
            })
            .await;
    })
}
