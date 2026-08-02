use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::app::Msg;
use crate::config::Server;
use crate::fmt::{error_line, header_line, status_line};
use crate::ssh;
use crate::stream::{connect, next_packet};

use multitop_agent::SortBy;

#[must_use]
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
#[must_use]
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

/// How many lines to look at before giving up on the readiness sentinel. A
/// login shell may print a banner first; beyond this something is wrong and the
/// run should proceed (and fail on sudo) rather than hang.
const MAX_SENTINEL_LINES: usize = 50;

#[must_use]
#[allow(
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::expect_used
)]
pub fn spawn_upgrade(
    idx: usize,
    gen: u64,
    server: Server,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Every exit from here must report AuxDone. Returning without it leaves
        // the panel in UpgradeState::STARTED for the rest of the session: it
        // says "running" forever, blocks any further upgrade because
        // `upgrades_in_flight()` never clears, and is recorded as an
        // interrupted run -- all while the stats stream to the same host keeps
        // working, which makes it look like the host went away.
        let Some(command) = server.upgrade_cmd.clone() else {
            let _ = tx
                .send(Msg::AuxDone {
                    panel: idx,
                    gen,
                    note: Some(status_line("\u{26A0} no upgrade_cmd configured")),
                    success: false,
                })
                .await;
            return;
        };

        let ssh::Spawned {
            mut child,
            awaits_password,
        } = match ssh::spawn_command(&server, &command, pass.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(Msg::AuxLine {
                        panel: idx,
                        gen,
                        line: error_line(e),
                    })
                    .await;
                let _ = tx
                    .send(Msg::AuxDone {
                        panel: idx,
                        gen,
                        note: Some(status_line("\u{26A0} could not start the upgrade over SSH")),
                        success: false,
                    })
                    .await;
                return;
            }
        };
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdout_lines = BufReader::new(stdout).lines();
        let mut stderr_lines = BufReader::new(stderr).lines();

        // Hand the sudo password over on stdin, once the remote says it has
        // turned echo off. It is deliberately absent from the command line: argv
        // is not secret, and `/proc/<pid>/cmdline` is world-readable on Linux, so
        // embedding it exposed the password to every user on the monitored host
        // for the length of the run.
        //
        // Waiting for the sentinel is what makes it safe rather than merely
        // moved: `-tt` allocates a pty, and anything arriving before `stty
        // -echo` completes is echoed straight back into this stdout. The
        // sentinel line is consumed here, so it never reaches the panel.
        if let Some(secret) = pass.as_deref().filter(|_| awaits_password) {
            let mut ready = false;
            for _ in 0..MAX_SENTINEL_LINES {
                match stdout_lines.next_line().await {
                    Ok(Some(line)) if line.trim() == ssh::PW_READY_SENTINEL => {
                        ready = true;
                        break;
                    }
                    // Anything else this early is banner noise; keep looking.
                    Ok(Some(_)) => {}
                    _ => break,
                }
            }
            if ready {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(format!("{secret}\n").as_bytes()).await;
                    let _ = stdin.flush().await;
                    // Dropping closes it, so the remote `read` cannot block.
                }
            }
        }

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
                            let line = line.trim_end_matches('\n').trim_end_matches('\r');
                            for part in line.split('\r') {
                                let clean = part.trim_end_matches('\r');
                                if !clean.trim().is_empty() {
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
                    let line = line.trim_end_matches('\n').trim_end_matches('\r');
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
        let success = exit_status
            .as_ref()
            .is_ok_and(std::process::ExitStatus::success);
        // Say what actually happened. Reporting every failure as "disconnected"
        // blamed the network for a command that merely exited non-zero, on a
        // host the stats view was talking to perfectly well at the time.
        let note = match exit_status {
            Ok(s) if s.success() => status_line("\u{2500} done"),
            Ok(s) => s.code().map_or_else(
                || status_line("\u{26A0} upgrade command was killed by a signal"),
                |code| {
                    status_line(format!(
                        "\u{26A0} upgrade command exited {code} \u{2014} host reachable, command failed"
                    ))
                },
            ),
            Err(e) => status_line(format!(
                "\u{26A0} lost the SSH session ({e}) \u{2014} upgrade may be incomplete"
            )),
        };
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(note),
                success,
            })
            .await;
    })
}
