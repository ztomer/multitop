//! Shared exec pty reader and agent installer for process actions.

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::mpsc::Sender;

use multitop_agent::exec::{ExecFrame, MarkerKind, Stream};
use multitop_agent::proto::{decode_packet, Payload, HEADER_LEN, MAGIC};

use crate::app::Msg;
use crate::config::Server;
use crate::fmt::{error_line, status_line};
use crate::ssh;
use crate::tasks::painted::{paint_msg, Painter};

pub const STALL_AFTER: Duration = Duration::from_secs(30);

pub struct ExecAction<'a> {
    pub idx: usize,
    pub gen: u64,
    pub server: &'a Server,
    pub command: &'a str,
    pub pass: Option<&'a str>,
    pub tx: &'a Sender<Msg>,
    pub header: &'a str,
    pub action_desc: &'a str,
}

pub async fn generic_exec(action: &ExecAction<'_>) -> String {
    let _ = action
        .tx
        .send(Msg::AuxBegin {
            panel: action.idx,
            gen: action.gen,
            header: Some(status_line(action.header.to_string())),
        })
        .await;
    let mut last_err = String::new();
    for attempt in 0..2 {
        match attempt_once(action).await {
            Ok(note) => return note,
            Err(need_agent) => {
                if let Some(arch) = need_agent {
                    if attempt == 0
                        && install_agent(action.idx, action.gen, action.server, &arch, action.tx)
                            .await
                            == Ok(())
                    {
                        continue;
                    }
                }
                last_err = format!(
                    "{} failed: no agent on {}",
                    action.action_desc, action.server.host
                );
                break;
            }
        }
    }
    if last_err.is_empty() {
        last_err = format!("{} failed on {}", action.action_desc, action.server.host);
    }
    last_err
}

pub async fn install_agent(
    idx: usize,
    gen: u64,
    server: &Server,
    arch_str: &str,
    tx: &Sender<Msg>,
) -> Result<(), ()> {
    let Some(arch) = ssh::Arch::from_uname(arch_str) else {
        return Err(());
    };
    let _ = tx
        .send(Msg::AuxLine {
            panel: idx,
            gen,
            line: status_line(format!("\u{2192} installing the agent on {}", server.host)),
        })
        .await;
    let token = format!("{}-{gen}", std::process::id());
    match ssh::upload_agent(server, arch, &token).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(format!("\u{26A0} {e}")),
                })
                .await;
            Err(())
        }
    }
}

pub async fn attempt_once(action: &ExecAction<'_>) -> Result<String, Option<String>> {
    let credential = action
        .pass
        .filter(|_| !crate::password_store::is_mock_enabled());
    let request = ExecFrame::Request {
        command: action.command.to_string(),
        password: credential.map(str::to_string),
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    let mut child = match ssh::spawn_exec(action.server, &request).await {
        Ok(c) => c,
        Err(e) => {
            let _ = action
                .tx
                .send(Msg::AuxLine {
                    panel: action.idx,
                    gen: action.gen,
                    line: error_line(crate::stream::spawn_failure(action.server, &e)),
                })
                .await;
            return Ok(format!("failed: {e}"));
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (stalled, exit_code) = if let Some(stdout) = stdout {
        drain_stdout(stdout, action).await
    } else {
        (false, None)
    };

    let report_need_agent = if let Some(stderr) = stderr {
        read_need_agent(stderr).await
    } else {
        None
    };

    let _ = child.wait().await;
    if let Some(need) = report_need_agent {
        return Err(Some(need));
    }
    if stalled {
        return Ok(format!("{} stalled", action.action_desc));
    }
    match exit_code {
        Some(0) => Ok(format!("{} succeeded", action.action_desc)),
        Some(code) => Ok(format!("{} exited {code}", action.action_desc)),
        None => Ok(format!("{} finished", action.action_desc)),
    }
}

async fn drain_stdout(stdout: ChildStdout, action: &ExecAction<'_>) -> (bool, Option<i32>) {
    let mut reader = BufReader::new(stdout);
    let mut header = [0u8; HEADER_LEN];
    let mut painter = Painter::new();
    let mut stalled = false;
    let mut exit_code = None;

    loop {
        let read = tokio::time::timeout(STALL_AFTER, reader.read_exact(&mut header)).await;
        let Ok(read) = read else {
            stalled = true;
            let _ = action
                .tx
                .send(Msg::AuxLine {
                    panel: action.idx,
                    gen: action.gen,
                    line: error_line(format!(
                        "no word from the agent for {}s \u{2014} the connection is gone",
                        STALL_AFTER.as_secs()
                    )),
                })
                .await;
            break;
        };
        if read.is_err() || &header[..4] != MAGIC {
            break;
        }
        let len = u16::from_le_bytes([header[HEADER_LEN - 2], header[HEADER_LEN - 1]]) as usize;
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).await.is_err() {
            break;
        }
        let mut packet = header.to_vec();
        packet.append(&mut body);
        if let Some(Payload::Hello(hello)) = decode_packet(&packet) {
            let local = env!("CARGO_PKG_VERSION");
            if hello.needs_replacement(local) {
                let _ = action
                    .tx
                    .send(Msg::AuxLine {
                        panel: action.idx,
                        gen: action.gen,
                        line: error_line(format!(
                            "agent version mismatch: remote {} vs local {local}",
                            hello.agent_version
                        )),
                    })
                    .await;
            }
            continue;
        }
        let Some(Payload::Exec(frame)) = decode_packet(&packet) else {
            continue;
        };
        if let Some(code) = handle_exec_frame(frame, &mut painter, action).await {
            exit_code = Some(code);
            break;
        }
    }
    if let Some(paint) = painter.finish() {
        let _ = action
            .tx
            .send(Msg::AuxLine {
                panel: action.idx,
                gen: action.gen,
                line: paint.text,
            })
            .await;
    }
    (stalled, exit_code)
}

/// Paint one stream's bytes with the run's shared cursor and send what lands.
///
/// Stdout goes plain, stderr red; placement (append vs overwrite) is the
/// painter's call either way. Blank stderr paints are dropped, as before --
/// a colour wrapper around nothing is a row of nothing -- while blank stdout
/// lines still append: a blank line between two paragraphs is output.
async fn send_painted(
    action: &ExecAction<'_>,
    painter: &mut Painter,
    bytes: &[u8],
    style: impl Fn(String) -> String,
    drop_empty: bool,
) {
    for paint in painter.feed_bytes(bytes) {
        if drop_empty && paint.text.trim().is_empty() && paint.back == 0 && paint.erase_below == 0 {
            continue;
        }
        let line = style(paint.text.clone());
        let _ = action
            .tx
            .send(paint_msg(action.idx, action.gen, &paint, line))
            .await;
    }
}

async fn handle_exec_frame(
    frame: ExecFrame,
    painter: &mut Painter,
    action: &ExecAction<'_>,
) -> Option<i32> {
    match frame {
        ExecFrame::Out {
            stream: Stream::Stdout,
            bytes,
            ..
        } => {
            send_painted(action, painter, &bytes, std::convert::identity, false).await;
            None
        }
        ExecFrame::Out {
            stream: Stream::Stderr,
            bytes,
            ..
        } => {
            // Through the same painter, not around it: `\r` progress on
            // stderr rewrites one line exactly like stdout does, and a
            // per-chunk `AuxLine` would append a copy per tick. One cursor
            // serves both streams -- the remote pty has only one -- so the
            // painter is shared, and only the styling differs.
            send_painted(action, painter, &bytes, error_line, true).await;
            None
        }
        ExecFrame::Marker(MarkerKind::SudoFailed) => {
            let _ = action
                .tx
                .send(Msg::AuxLine {
                    panel: action.idx,
                    gen: action.gen,
                    line: error_line("sudo: password rejected".to_string()),
                })
                .await;
            None
        }
        ExecFrame::Exit { code, .. } => Some(code),
        _ => None,
    }
}

async fn read_need_agent(stderr: ChildStderr) -> Option<String> {
    let mut lines = BufReader::new(stderr).lines();
    let mut report_need_agent = None;
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(arch) = ssh::parse_need_agent(&line) {
            report_need_agent = Some(arch.to_string());
            continue;
        }
        if !line.trim().is_empty() {
            report_need_agent = None;
        }
    }
    report_need_agent
}

#[cfg(test)]
mod send_painted_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::tasks::{Paint, Painter};

    fn action<'a>(server: &'a Server, tx: &'a Sender<Msg>) -> ExecAction<'a> {
        ExecAction {
            idx: 0,
            gen: 0,
            server,
            command: "cmd",
            pass: None,
            tx,
            header: "header",
            action_desc: "action",
        }
    }

    fn server() -> Server {
        Server {
            host: "host".to_string(),
            port: 0,
            user: String::new(),
            upgrade_cmd: None,
            custom_command: None,
        }
    }

    /// Placement is the painter's call: appends stay lines, rewinds become
    /// repaints, and the caller's styling survives either way.
    #[test]
    fn paint_msg_routes_by_movement_and_keeps_styling() {
        let append = Paint {
            text: "plain".to_string(),
            back: 0,
            erase_below: 0,
        };
        assert!(matches!(
            paint_msg(0, 0, &append, append.text.clone()),
            Msg::AuxLine { .. }
        ));
        let repaint = Paint {
            text: "red".to_string(),
            back: 2,
            erase_below: 0,
        };
        match paint_msg(0, 0, &repaint, error_line("red")) {
            Msg::AuxRepaint { back, line, .. } => {
                assert_eq!(back, 2);
                assert!(line.contains("red"), "styling must survive: {line:?}");
            }
            other => panic!("a rewind must repaint, got {other:?}"),
        }
    }

    /// `\r` progress on stderr rewrites one line exactly like stdout: before
    /// the shared painter it arrived as one `AuxLine` per chunk and every
    /// tick appended a copy.
    #[tokio::test]
    async fn stderr_progress_repaints_instead_of_appending() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let server = server();
        let action = action(&server, &tx);
        let mut painter = Painter::new();
        send_painted(&action, &mut painter, b"10%\n", error_line, true).await;
        send_painted(
            &action,
            &mut painter,
            b"\r\x1b[2K\x1b[1A\x1b[2K100%\n",
            error_line,
            true,
        )
        .await;
        drop(tx);
        let mut msgs = Vec::new();
        while let Some(msg) = rx.recv().await {
            msgs.push(msg);
        }
        assert_eq!(msgs.len(), 2, "two paints, not three: {msgs:?}");
        assert!(matches!(msgs[0], Msg::AuxLine { .. }));
        match &msgs[1] {
            Msg::AuxRepaint { back, line, .. } => {
                assert_eq!(*back, 1, "rewrites the newest row: {msgs:?}");
                assert!(line.contains("100%"), "styling kept: {line:?}");
            }
            other => panic!("the rewind must repaint, got {other:?}"),
        }
    }

    /// Blank stderr paints are dropped, as before -- a colour wrapper around
    /// nothing is a row of nothing -- while blank stdout lines still append.
    #[tokio::test]
    async fn blank_stderr_is_dropped_and_blank_stdout_is_kept() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let server = server();
        let action = action(&server, &tx);
        let mut painter = Painter::new();
        send_painted(&action, &mut painter, b"  \n", error_line, true).await;
        send_painted(
            &action,
            &mut painter,
            b"  \n",
            std::convert::identity,
            false,
        )
        .await;
        drop(tx);
        let mut count = 0;
        while rx.recv().await.is_some() {
            count += 1;
        }
        assert_eq!(count, 1, "only the stdout blank survives");
    }
}
