#![allow(
    clippy::expect_used,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;

use crate::app::Msg;
use crate::config::Server;
use crate::fmt::{error_line, header_line, status_line};
use crate::ssh;
use crate::tasks::painted::{is_sudo_help, marker, painted_states, Marker, Painter};
use crate::tasks::spawn::deliver_sudo_password;
use crate::tasks::spawn::SENTINEL_TIMEOUT;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

const CHUNK_READ_BUFFER_SIZE: usize = 4096;
const PROMPT_FLUSH_TIMEOUT_MS: u64 = 100;

#[allow(clippy::too_many_lines)]
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
        let stderr_lines = BufReader::new(stderr).lines();

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
            let handed = deliver_sudo_password(
                &mut stdout_lines,
                child.stdin.take(),
                secret,
                SENTINEL_TIMEOUT,
            )
            .await;
            if !handed {
                // Say so rather than letting it surface as a network error.
                // Silence here is what made a sudo-handshake failure
                // indistinguishable from an unreachable host.
                let _ = tx
                    .send(Msg::AuxLine {
                        panel: idx,
                        gen,
                        line: error_line(
                            "sudo handshake did not complete: the remote never signalled that it \
                             was ready for the password. The host is reachable; the upgrade \
                             continued without sudo pre-authorisation.",
                        ),
                    })
                    .await;
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

        // The screen the remote thinks it is drawing on, for tools that repaint
        // several lines rather than one.
        let mut painter = Painter::new();
        let mut sudo_help = false;
        // Set when the remote says sudo refused the password we handed it.
        let mut sudo_rejected = false;
        // Set when the remote says another run already holds the upgrade lock.
        let mut lock_held = false;
        let mut errbuf = Vec::new();
        // Both streams are read to their own end, rather than stopping the
        // moment stdout finishes. They close together when the child exits, so
        // whichever `select!` happened to poll first decided whether the
        // contents of the stderr pipe were read or thrown away -- and stderr is
        // where the reason lives: apt's actual complaint, the sudo-help shapes,
        // the held-lock sentinel. A run that failed for a nameable reason
        // reported "exited 1" about half the time.
        let mut stdout_open = true;
        let mut stderr_open = true;
        let mut stdout_reader = stdout_lines.into_inner();
        let mut stderr_reader = stderr_lines.into_inner();
        let mut stdout_buf = [0u8; CHUNK_READ_BUFFER_SIZE];
        let mut stderr_buf = [0u8; CHUNK_READ_BUFFER_SIZE];
        let mut stdout_str = String::new();
        let mut stderr_str = String::new();

        while stdout_open || stderr_open {
            tokio::select! {
                res = tokio::time::timeout(std::time::Duration::from_millis(PROMPT_FLUSH_TIMEOUT_MS), stdout_reader.read(&mut stdout_buf)), if stdout_open => {
                    match res {
                        Ok(Ok(0) | Err(_)) => {
                            stdout_open = false;
                            if !stdout_str.is_empty() {
                                if let Some(paint) = painter.feed(&stdout_str, true) {
                                    let msg = if paint.back == 0 && paint.erase_below == 0 {
                                        Msg::AuxLine { panel: idx, gen, line: paint.text }
                                    } else {
                                        Msg::AuxRepaint { panel: idx, gen, line: paint.text, back: paint.back, erase_below: paint.erase_below }
                                    };
                                    let _ = tx.send(msg).await;
                                }
                                stdout_str.clear();
                            }
                        }
                        Ok(Ok(n)) => {
                            let chunk = String::from_utf8_lossy(&stdout_buf[..n]);
                            for c in chunk.chars() {
                                stdout_str.push(c);
                                if c == '\n' || c == '\r' {
                                    let mut is_marker = false;
                                    for state in painted_states(&stdout_str) {
                                        let trimmed = state.trim();
                                        match marker(trimmed) {
                                            Some(Marker::SudoFailed) => { sudo_rejected = true; is_marker = true; continue; }
                                            Some(Marker::LockHeld) => { lock_held = true; is_marker = true; continue; }
                                            None => {}
                                        }
                                        if !trimmed.is_empty() && is_sudo_help(&trimmed.to_lowercase()) {
                                            sudo_help = true;
                                        }
                                    }
                                    if !is_marker {
                                        if let Some(paint) = painter.feed(&stdout_str, c == '\n') {
                                            let msg = if paint.back == 0 && paint.erase_below == 0 {
                                                Msg::AuxLine { panel: idx, gen, line: paint.text }
                                            } else {
                                                Msg::AuxRepaint { panel: idx, gen, line: paint.text, back: paint.back, erase_below: paint.erase_below }
                                            };
                                            if tx.send(msg).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                    stdout_str.clear();
                                }
                            }
                        }
                        Err(_) => { // Timeout
                            if !stdout_str.is_empty() {
                                if let Some(paint) = painter.feed(&stdout_str, false) {
                                    let msg = if paint.back == 0 && paint.erase_below == 0 {
                                        Msg::AuxLine { panel: idx, gen, line: paint.text }
                                    } else {
                                        Msg::AuxRepaint { panel: idx, gen, line: paint.text, back: paint.back, erase_below: paint.erase_below }
                                    };
                                    if tx.send(msg).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                res = tokio::time::timeout(std::time::Duration::from_millis(PROMPT_FLUSH_TIMEOUT_MS), stderr_reader.read(&mut stderr_buf)), if stderr_open => {
                    let process_stderr_line = |line: &str, errbuf: &mut Vec<String>, sudo_rejected: &mut bool, lock_held: &mut bool, sudo_help: &mut bool| {
                        let mut visible = None;
                        for state in painted_states(line) {
                            let trimmed = state.trim();
                            match marker(trimmed) {
                                Some(Marker::SudoFailed) => {
                                    *sudo_rejected = true;
                                    continue;
                                }
                                Some(Marker::LockHeld) => {
                                    *lock_held = true;
                                    continue;
                                }
                                None => {}
                            }
                            if trimmed.is_empty() {
                                continue;
                            }
                            let lower = trimmed.to_lowercase();
                            if is_sudo_help(&lower) {
                                *sudo_help = true;
                            }
                            if lower.contains("shared connection to") || (lower.contains("connection to") && lower.contains("closed")) {
                                continue;
                            }
                            visible = Some(state);
                        }
                        if let Some(state) = visible {
                            if errbuf.len() >= crate::consts::MAX_UPGRADE_ERR_LINES {
                                errbuf.remove(0);
                            }
                            errbuf.push(state.to_string());
                        }
                    };

                    match res {
                        Ok(Ok(0) | Err(_)) => {
                            stderr_open = false;
                            if !stderr_str.is_empty() {
                                process_stderr_line(&stderr_str, &mut errbuf, &mut sudo_rejected, &mut lock_held, &mut sudo_help);
                                stderr_str.clear();
                            }
                        }
                        Ok(Ok(n)) => {
                            let chunk = String::from_utf8_lossy(&stderr_buf[..n]);
                            for c in chunk.chars() {
                                stderr_str.push(c);
                                if c == '\n' || c == '\r' {
                                    process_stderr_line(&stderr_str, &mut errbuf, &mut sudo_rejected, &mut lock_held, &mut sudo_help);
                                    stderr_str.clear();
                                }
                            }
                        }
                        Err(_) => {}
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
            // A rejected sudo password is not a failing upgrade command, and
            // saying so sent the user to read their upgrade script when the
            // problem was the password. The command never ran at all.
            Ok(s) if sudo_rejected || s.code() == Some(ssh::SUDO_FAILED_CODE) => status_line(
                format!(
                    "\u{26A0} sudo refused the stored password on {} \u{2014} the upgrade did not run. \
                     Set this host's password with {} in Settings.",
                    server.host,
                    crate::consts::SETTINGS_KEY
                ),
            ),
            // A held lock is not a failing command either: the command never ran.
            // The lock lives at `~/.cache/multitop/upgrade.lock` and is only
            // broken automatically after six hours, so naming it is the whole
            // fix -- a leftover from a killed run needs removing by hand.
            Ok(s) if lock_held || s.code() == Some(ssh::LOCK_HELD_CODE) => status_line(
                format!(
                    "\u{26A0} another upgrade holds the lock on {} \u{2014} this one never ran. \
                     If no other run is active, remove ~/.cache/multitop/upgrade.lock.",
                    server.host
                ),
            ),
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
