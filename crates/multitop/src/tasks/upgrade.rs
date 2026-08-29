//! Reading one upgrade, now that its output arrives framed.
//!
//! This used to be a byte reader with a 100 ms timer, a partial-line buffer and
//! a `\r`-splitting loop, because the stream it read had no record boundaries
//! in it and they had to be guessed. They are on the wire now, so almost all of
//! that is gone: what is left reads frames, hands the bytes to the screen
//! model, and reports the exit the agent actually observed.
//!
//! Two obligations survive from the old version and are the reason this file is
//! shaped the way it is.
//!
//! **Every exit reports `AuxDone`.** Returning without one leaves the panel in
//! `UpgradeState::STARTED` for the rest of the session: it says "running"
//! forever, `upgrades_in_flight()` never clears so quitting asks for a
//! confirmation about a run that ended long ago, and no further upgrade can
//! start on any host -- all while the stats stream to the same host keeps
//! working, which makes it look as though the host went away. The old code had
//! two `return`s that skipped it. Here there is one exit, in [`spawn_upgrade`],
//! and [`run_upgrade`] cannot leave without passing through it.
//!
//! **Silence is bounded.** The agent sends a heartbeat about once a second, so
//! a gap longer than [`STALL_AFTER`] means the far end is gone rather than
//! busy. Before, a run that stopped producing output simply never ended.

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use multitop_agent::exec::{ExecFrame, MarkerKind, Stream};
use multitop_agent::proto::{decode_packet, Payload, HEADER_LEN, MAGIC};

use crate::app::Msg;
use crate::config::Server;
use crate::fmt::{error_line, header_line, status_line};
use crate::ssh;
use crate::tasks::painted::{is_sudo_help, Paint, Painter};
use crate::tasks::verdict::{sudo_tips, verdict, Outcome, Report};

/// How long the client waits with no frame at all before calling it lost.
///
/// The agent heartbeats every second while its child lives, so this is thirty
/// missed heartbeats, not thirty seconds of a slow command. A long `apt` is
/// noisy in exactly the way this deadline needs.
const STALL_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

#[must_use]
pub fn spawn_upgrade(
    idx: usize,
    gen: u64,
    server: Server,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = run_upgrade(idx, gen, &server, pass.as_deref(), &tx).await;
        // The one exit. Everything above returns an `Outcome`; nothing above
        // returns from this task.
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(outcome.note),
                success: outcome.success,
            })
            .await;
    })
}

async fn run_upgrade(
    idx: usize,
    gen: u64,
    server: &Server,
    pass: Option<&str>,
    tx: &Sender<Msg>,
) -> Outcome {
    let Some(command) = server.upgrade_cmd.clone() else {
        return Outcome {
            note: status_line("\u{26A0} no upgrade_cmd configured"),
            success: false,
        };
    };

    // Under the mock password store the credential is a fixture, not a
    // password. Handing it to `sudo -v` would fail every time and turn every
    // streaming test into a sudo-refusal test. This is the seam the old
    // `spawn_command` had as `awaits_password: password.is_some() &&
    // !is_mock_enabled()`, kept in the one place that now decides it.
    let credential = pass.filter(|_| !crate::password_store::is_mock_enabled());
    let request = ExecFrame::Request {
        command,
        password: credential.map(str::to_string),
        // The mock password store means a test, and two tests contending on one
        // host-wide lock would block each other for reasons that have nothing
        // to do with what they are testing.
        use_lock: !crate::password_store::is_mock_enabled(),
        cols: 0,
        rows: 0,
    };

    let _ = tx
        .send(Msg::AuxBegin {
            panel: idx,
            gen,
            header: Some(header_line(format!("Upgrade on {}", server.host))),
        })
        .await;

    // Two attempts at most: a host with no agent gets one installed and is
    // asked again. A second miss is a real failure, not a race.
    let mut report = Report::default();
    for attempt in 0..2 {
        report = attempt_once(idx, gen, server, &request, tx).await;
        let Some(arch) = report.need_agent.clone() else {
            break;
        };
        if attempt > 0 {
            break;
        }
        match install_agent(idx, gen, server, &arch, tx).await {
            Ok(()) => {}
            Err(note) => {
                return Outcome {
                    note,
                    success: false,
                }
            }
        }
    }

    for line in std::mem::take(&mut report.errbuf) {
        let _ = tx
            .send(Msg::AuxLine {
                panel: idx,
                gen,
                line: error_line(line),
            })
            .await;
    }

    // The tips go into the log, not into the closing status line. They are
    // three lines of instruction and the note is one line the panel truncates;
    // more to the point, an operator reads them where the failure is.
    if report.sudo_help {
        for tip in sudo_tips(pass) {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: tip,
                })
                .await;
        }
    }

    verdict(server, &report)
}

/// Put this build's agent on a host that had none, saying so as it goes.
async fn install_agent(
    idx: usize,
    gen: u64,
    server: &Server,
    arch_str: &str,
    tx: &Sender<Msg>,
) -> Result<(), String> {
    let Some(arch) = ssh::Arch::from_uname(arch_str) else {
        return Err(status_line(format!(
            "\u{26A0} unsupported architecture '{arch_str}' on {} \u{2014} multitop ships x86_64 \
             and aarch64",
            server.host
        )));
    };
    let _ = tx
        .send(Msg::AuxLine {
            panel: idx,
            gen,
            line: status_line(format!("\u{2192} installing the agent on {}", server.host)),
        })
        .await;
    let token = format!("{}-{gen}", std::process::id());
    ssh::upload_agent(server, arch, &token)
        .await
        .map_err(|e| status_line(format!("\u{26A0} {e}")))
}

/// Run once and read it to the end.
async fn attempt_once(
    idx: usize,
    gen: u64,
    server: &Server,
    request: &ExecFrame,
    tx: &Sender<Msg>,
) -> Report {
    let mut report = Report::default();
    let mut child = match ssh::spawn_exec(server, request).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(crate::stream::spawn_failure(server, &e)),
                })
                .await;
            return report;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Some(stdout) = stdout {
        read_frames(idx, gen, BufReader::new(stdout), tx, &mut report).await;
    }
    // Read after, not concurrently: this pipe carries only what `ssh` and the
    // bootstrap say -- `Permission denied`, `===NEEDAGENT===` -- and the agent's
    // own stderr arrives as frames on the other one. There is nothing here to
    // race with.
    if let Some(stderr) = stderr {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(arch) = ssh::parse_need_agent(&line) {
                report.need_agent = Some(arch.to_string());
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || is_connection_noise(&trimmed.to_lowercase()) {
                continue;
            }
            report.preamble = Some(trimmed.to_string());
        }
    }
    if report.exit.is_none() && report.need_agent.is_none() && !report.stalled {
        // The stream ended without an Exit frame. That is the agent dying or
        // the connection dropping, and it has to be named -- reporting it as a
        // clean finish is exactly the lie this channel was built to stop.
        if let Some(text) = report.preamble.clone() {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(text),
                })
                .await;
        }
    }
    let _ = child.wait().await;
    report
}

/// Whether a stderr line is `ssh` describing its own teardown.
fn is_connection_noise(lower: &str) -> bool {
    lower.contains("shared connection to")
        || (lower.contains("connection to") && lower.contains("closed"))
}

/// Read the framed stream, painting as it goes.
async fn read_frames<R>(idx: usize, gen: u64, mut stdout: R, tx: &Sender<Msg>, report: &mut Report)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut painter = Painter::new();
    let mut header = [0u8; HEADER_LEN];
    loop {
        let read = tokio::time::timeout(STALL_AFTER, stdout.read_exact(&mut header)).await;
        let Ok(read) = read else {
            report.stalled = true;
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(format!(
                        "no word from the agent for {}s \u{2014} the connection is gone",
                        STALL_AFTER.as_secs()
                    )),
                })
                .await;
            return;
        };
        if read.is_err() {
            break;
        }
        if &header[..4] != MAGIC {
            // Not framing. Something else got to stdout first -- a login
            // banner, a profile that prints. Kept so the failure can name it.
            report.preamble = Some(String::from_utf8_lossy(&header).trim_end().to_string());
            break;
        }
        let len = u16::from_le_bytes([header[HEADER_LEN - 2], header[HEADER_LEN - 1]]) as usize;
        let mut body = vec![0u8; len];
        if stdout.read_exact(&mut body).await.is_err() {
            break;
        }
        let mut packet = header.to_vec();
        packet.append(&mut body);
        let Some(Payload::Exec(frame)) = decode_packet(&packet) else {
            continue;
        };
        if apply_frame(idx, gen, &frame, &mut painter, tx, report).await {
            break;
        }
    }
    if let Some(paint) = painter.finish() {
        send_paint(idx, gen, &paint, tx).await;
    }
}

/// Act on one frame. Returns whether the run has ended.
async fn apply_frame(
    idx: usize,
    gen: u64,
    frame: &ExecFrame,
    painter: &mut Painter,
    tx: &Sender<Msg>,
    report: &mut Report,
) -> bool {
    match frame {
        ExecFrame::Out {
            stream: Stream::Stdout,
            bytes,
            ..
        } => {
            // One scanner over both streams, for the reason the marker scanner
            // was given one: two streams disagreeing about what counts is how
            // one of them stops recognising it.
            if is_sudo_help(&String::from_utf8_lossy(bytes).to_lowercase()) {
                report.sudo_help = true;
            }
            for paint in painter.feed_bytes(bytes) {
                send_paint(idx, gen, &paint, tx).await;
            }
        }
        ExecFrame::Out {
            stream: Stream::Stderr,
            bytes,
            ..
        } => keep_stderr(&String::from_utf8_lossy(bytes), report),
        ExecFrame::Marker(MarkerKind::SudoFailed) => report.sudo_rejected = true,
        ExecFrame::Marker(MarkerKind::LockHeld) => report.lock_held = true,
        ExecFrame::Exit { code, signalled } => {
            report.exit = Some((*code, *signalled));
            return true;
        }
        // A heartbeat is the absence of news; the `Started`/`Done`/`PwReady`
        // markers are the agent's own bookkeeping; `Begin` says a run started,
        // which the panel already knows. And only a client sends a `Request`,
        // and this is the client. Nothing to draw for any of them.
        ExecFrame::Marker(_)
        | ExecFrame::Alive { .. }
        | ExecFrame::Begin { .. }
        | ExecFrame::Request { .. } => {}
    }
    false
}

/// Remember the last few stderr lines, minus the parts that are not the
/// operator's business.
fn keep_stderr(chunk: &str, report: &mut Report) {
    for line in chunk.split('\n') {
        let trimmed = line.trim_end_matches('\r').trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if is_sudo_help(&lower) {
            report.sudo_help = true;
        }
        // `ssh` narrating its own teardown. It is not the command's output and
        // it is not the reason anything failed.
        if is_connection_noise(&lower) {
            continue;
        }
        if report.errbuf.len() >= crate::consts::MAX_UPGRADE_ERR_LINES {
            report.errbuf.remove(0);
        }
        report.errbuf.push(trimmed.to_string());
    }
}

async fn send_paint(idx: usize, gen: u64, paint: &Paint, tx: &Sender<Msg>) {
    let msg = if paint.back == 0 && paint.erase_below == 0 {
        Msg::AuxLine {
            panel: idx,
            gen,
            line: paint.text.clone(),
        }
    } else {
        Msg::AuxRepaint {
            panel: idx,
            gen,
            line: paint.text.clone(),
            back: paint.back,
            erase_below: paint.erase_below,
        }
    };
    let _ = tx.send(msg).await;
}
