//! Tests for the shared exec pty reader, beside the reader rather than inside
//! it — the same split `upgrade_view` uses (`#[path]` test module), for the
//! same reason: the reader file stays under the 500-line cap.

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
        mcp: None,
    }
}

/// The `finish()` tail of a run routes like any other paint: an
/// unterminated last line is already on screen via its open-line paint,
/// so finishing must overwrite it, not append a second copy.
///
/// No agent, no pty, no sleeps: `drain_stdout` is generic over the byte
/// stream, so the test feeds it synthetic framed packets through a duplex
/// pipe -- the exact bytes a tool killed mid-write leaves behind.
#[tokio::test]
async fn unterminated_final_line_is_not_appended_twice() {
    use crate::app::App;
    use crate::panel::{Mode, UpgradeState};
    use multitop_agent::exec::{ExecFrame, Stream};
    use multitop_agent::proto::{encode_packet, Payload};

    let _g = crate::password_store::lock_for_test_async().await;
    crate::password_store::enable_mock_store();
    crate::password_store::clear_mock_store();

    let frame = |payload: &Payload| encode_packet(payload);
    let mut stream = Vec::new();
    stream.extend(frame(&Payload::Exec(ExecFrame::Out {
        stream: Stream::Stdout,
        seq: 0,
        bytes: b"hello\n".to_vec(),
    })));
    // No trailing newline: the tool died mid-write.
    stream.extend(frame(&Payload::Exec(ExecFrame::Out {
        stream: Stream::Stdout,
        seq: 1,
        bytes: b"tail-no-newline".to_vec(),
    })));
    stream.extend(frame(&Payload::Exec(ExecFrame::Exit {
        code: 0,
        signalled: false,
    })));

    let server = Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
        mcp: None,
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let action = ExecAction {
        idx: 0,
        gen: 0,
        server: &server,
        command: "cmd",
        pass: None,
        tx: &tx,
        header: "header",
        action_desc: "action",
    };
    let (reader, mut writer) = tokio::io::duplex(stream.len().max(1024));
    tokio::io::AsyncWriteExt::write_all(&mut writer, &stream)
        .await
        .unwrap();
    drop(writer);
    let (stalled, exit) = drain_stdout(reader, &action).await;
    assert!(!stalled);
    assert_eq!(exit, Some(0));
    drop(tx);

    let mut app = App::new(vec![server]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = Mode::Upgrade;
    while let Some(msg) = rx.recv().await {
        app.apply(msg);
    }
    let tails = app.panels[0]
        .last_upgrade
        .iter()
        .filter(|l| l.contains("tail-no-newline"))
        .count();
    assert_eq!(
        tails,
        1,
        "the unterminated last line was appended twice:\n{}",
        app.panels[0]
            .last_upgrade
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
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
