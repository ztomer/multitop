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

/// Hand the sudo password to a spawned command, and close its stdin.
///
/// Returns whether the remote signalled that it was ready. Extracted from
/// `spawn_upgrade` so the failure path can be tested: inline, it was reachable
/// only through a real SSH session that had to misbehave on cue.
///
/// # The pipe is closed on both paths
///
/// `stdin` is consumed by value and dropped here whatever happens. It used to be
/// taken from the child only when the sentinel arrived, so a missed sentinel
/// left the write end open forever -- and the remote `read -r __mt_pw` blocks
/// until its stdin closes. The upgrade then hung until the SSH session timed
/// out and surfaced as a connection failure, on a host the stats panel was
/// talking to the whole time.
///
/// # The wait is bounded in time, not only in lines
///
/// `MAX_SENTINEL_LINES` bounds how many lines are read, which bounds nothing at
/// all when no line ever arrives: `next_line` simply waits. A remote that is
/// itself blocked -- on its own `read`, on a lock, on a prompt -- and this
/// waiting for its first line is a mutual deadlock, and the upgrade never
/// finishes. The panel then sits in `STARTED` for the rest of the session,
/// blocking every later upgrade, while the stats stream to the same host keeps
/// working perfectly.
async fn deliver_sudo_password(
    stdout_lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    stdin: Option<tokio::process::ChildStdin>,
    secret: &str,
    patience: std::time::Duration,
) -> bool {
    let hunt = async {
        for _ in 0..MAX_SENTINEL_LINES {
            match stdout_lines.next_line().await {
                Ok(Some(line)) if line.trim() == ssh::PW_READY_SENTINEL => return true,
                // Anything else this early is banner noise; keep looking.
                Ok(Some(_)) => {}
                _ => return false,
            }
        }
        false
    };
    let ready = tokio::time::timeout(patience, hunt).await.unwrap_or(false);
    if let Some(mut stdin) = stdin {
        if ready {
            let _ = stdin.write_all(format!("{secret}\n").as_bytes()).await;
            let _ = stdin.flush().await;
        }
        // Dropped here on both paths, so the remote `read` cannot block.
    }
    ready
}

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

/// How long to wait for that sentinel before giving up.
///
/// Generous enough for a slow link and a chatty login banner, short enough that
/// a wedged remote costs one message rather than the rest of the session. The
/// line count alone is not a bound: a remote that prints nothing makes it
/// unreachable.
const SENTINEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

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

        let mut sudo_help = false;
        // Set when the remote says sudo refused the password we handed it.
        let mut sudo_rejected = false;
        let mut errbuf = Vec::new();
        loop {
            tokio::select! {
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            let line = line.trim_end_matches('\n').trim_end_matches('\r');
                            for part in line.split('\r') {
                                let clean = part.trim_end_matches('\r');
                                if clean.trim() == ssh::SUDO_FAILED_SENTINEL {
                                    sudo_rejected = true;
                                    continue;
                                }
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
            // A rejected sudo password is not a failing upgrade command, and
            // saying so sent the user to read their upgrade script when the
            // problem was the password. The command never ran at all.
            Ok(s) if sudo_rejected || s.code() == Some(ssh::SUDO_FAILED_CODE) => status_line(
                format!(
                    "\u{26A0} sudo refused the stored password on {} \u{2014} the upgrade did not run. \
                     Set a password for this host with o in Settings; an SSO password is \
                     only right if every host shares it.",
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

#[cfg(test)]
mod sudo_handshake_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{deliver_sudo_password, ssh, BufReader};
    use tokio::io::AsyncBufReadExt as _;
    use tokio::process::Command;

    /// Run `sh -c script` with piped stdio and put it through the handshake.
    /// Returns (sentinel seen, everything the child printed afterwards).
    async fn run(script: &str) -> (bool, String) {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();

        // A short bound so the give-up path does not cost the suite 20 seconds.
        // The production value is `SENTINEL_TIMEOUT`; what is under test is the
        // behaviour on either side of it, not the number.
        // The outer bound is the test's own: without a time bound inside
        // `deliver_sudo_password` the hunt for the sentinel never returns at
        // all, and this would hang rather than report.
        let ready = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            deliver_sudo_password(
                &mut lines,
                child.stdin.take(),
                "s3cret",
                std::time::Duration::from_millis(400),
            ),
        )
        .await
        .expect("the sentinel wait must be bounded in time, not only in lines");

        // Bounded on purpose. Unbounded, a child that never prints and never
        // exits -- which is exactly what the bug produces -- makes this test
        // hang instead of fail, and a test that hangs reports nothing.
        let mut rest = String::new();
        let drain = async {
            while let Ok(Some(line)) = lines.next_line().await {
                rest.push_str(&line);
                rest.push('\n');
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), drain).await;
        // The child must be able to finish. If stdin were left open it would sit
        // in `read` forever and this would hang rather than fail.
        let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
            .await
            .expect("the child must exit; a still-open stdin pipe is what hangs it");
        assert!(status.is_ok());
        (ready, rest)
    }

    #[tokio::test]
    async fn the_password_is_written_once_the_remote_says_it_is_ready() {
        let script = format!(
            "printf '{}\\n'; IFS= read -r p; printf 'got:%s\\n' \"$p\"",
            ssh::PW_READY_SENTINEL
        );
        let (ready, rest) = run(&script).await;

        assert!(ready, "the sentinel was printed, so it must be seen");
        assert!(
            rest.contains("got:s3cret"),
            "the password must reach the remote's `read`, got {rest:?}"
        );
    }

    /// Banner noise before the sentinel must not defeat the handshake.
    #[tokio::test]
    async fn lines_printed_before_the_sentinel_are_skipped() {
        let script = format!(
            "echo motd; echo 'Last login: today'; printf '{}\\n'; IFS= read -r p; printf 'got:%s\\n' \"$p\"",
            ssh::PW_READY_SENTINEL
        );
        let (ready, rest) = run(&script).await;

        assert!(ready);
        assert!(rest.contains("got:s3cret"), "got {rest:?}");
    }

    /// The regression. A remote that never signals readiness must still get its
    /// stdin closed, or its `read` blocks until the session dies and the whole
    /// thing is reported as an unreachable host.
    #[tokio::test]
    async fn a_missing_sentinel_still_closes_the_pipe() {
        // Never prints the sentinel, then blocks in `read` until stdin closes.
        let script = "IFS= read -r p; printf 'ended:%s\\n' \"$p\"";
        let (ready, rest) = run(script).await;

        assert!(!ready, "no sentinel was printed, so nothing may be sent");
        assert!(
            !rest.contains("s3cret"),
            "the password must not be sent to a remote that never asked: {rest:?}"
        );
        assert!(
            rest.contains("ended:"),
            "the child's `read` must have returned, which only happens when the \
             write end is dropped -- got {rest:?}"
        );
    }

    /// A remote that says nothing at all and exits: still no hang, still no
    /// password written into the void.
    #[tokio::test]
    async fn a_silent_remote_is_not_sent_the_password() {
        let (ready, rest) = run("exit 0").await;
        assert!(!ready);
        assert!(rest.is_empty(), "got {rest:?}");
    }
}
