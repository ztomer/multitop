//! The local exec path, end to end, with no `ssh` involved.
//!
//! `127.0.0.1` with port 0 is `is_local`, so these run this build's own agent
//! directly -- which is the point. Local and remote used to be two different
//! transports for one feature: `$SHELL -c` with two pipes and no pty here,
//! `ssh -tt` with a pty and merged streams there. They disagreed about line
//! endings, about whether stderr was separable, and about whether the command
//! could see a terminal. They are one path now, and these tests assert the
//! properties that path is supposed to have.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::config::Server;
use multitop::password_store;
use multitop::ssh;
use multitop_agent::exec::{ExecFrame, Stream};
use multitop_agent::proto::{decode_packet, Payload, HEADER_LEN, MAGIC};
use tokio::io::AsyncReadExt;

fn local_server(upgrade_cmd: &str) -> Server {
    Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some(upgrade_cmd.to_string()),
    }
}

fn enable_mock_store() {
    password_store::enable_mock_store();
    password_store::clear_mock_store();
}

fn request(command: &str) -> ExecFrame {
    ExecFrame::Request {
        command: command.to_string(),
        password: None,
        // Off deliberately. The lock is the agent's, it is tested against a
        // temporary directory in `crates/agent/tests/exec_run_test.rs`
        // (`a_held_lock_stops_the_second_run_and_says_so` and
        // `the_lock_is_released_when_the_run_ends`), and a test that took the
        // real one would contend with whatever else is running on this machine.
        use_lock: false,
        cols: 80,
        rows: 24,
    }
}

/// What one run produced: its two streams and its outcome.
struct Ran {
    stdout: String,
    stderr: String,
    exit: Option<i32>,
}

/// Run a command through the exec channel and decode everything it said.
///
/// Length-driven, like the client's own reader. A frame that does not decode
/// stops the loop rather than being skipped: the frames are not self-delimiting
/// inside a payload, so a reader that carries on is reading from the wrong
/// offset and inventing output.
async fn run(command: &str) -> Ran {
    let server = local_server(command);
    let mut child = ssh::spawn_exec(&server, &request(command))
        .await
        .expect("the local agent should spawn");
    let mut raw = Vec::new();
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_to_end(&mut raw)
        .await
        .unwrap();
    let _ = child.wait().await;

    let (mut out, mut err, mut exit) = (Vec::new(), Vec::new(), None);
    let mut pos = 0;
    while pos + HEADER_LEN <= raw.len() {
        assert_eq!(&raw[pos..pos + 4], MAGIC, "the stream stopped being frames");
        let len =
            u16::from_le_bytes([raw[pos + HEADER_LEN - 2], raw[pos + HEADER_LEN - 1]]) as usize;
        let end = pos + HEADER_LEN + len;
        assert!(end <= raw.len(), "a frame ran past the end of the stream");
        match decode_packet(&raw[pos..end]) {
            Some(Payload::Exec(ExecFrame::Out { stream, bytes, .. })) => match stream {
                Stream::Stdout => out.extend_from_slice(&bytes),
                Stream::Stderr => err.extend_from_slice(&bytes),
            },
            Some(Payload::Exec(ExecFrame::Exit { code, .. })) => exit = Some(code),
            _ => {}
        }
        pos = end;
    }
    Ran {
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        exit,
    }
}

#[tokio::test]
async fn a_local_command_runs_and_its_output_comes_back() {
    enable_mock_store();
    let ran = run("echo hello").await;
    assert!(ran.stdout.contains("hello"), "got {:?}", ran.stdout);
    assert_eq!(ran.exit, Some(0));
}

/// A local panel gets a terminal too. It did not before -- `$SHELL -c` with a
/// pipe on stdout meant `isatty(1)` was false, so `apt` and `docker` printed
/// something duller here than they did on a remote host running the same
/// command.
#[tokio::test]
async fn a_local_command_sees_a_terminal() {
    enable_mock_store();
    let ran = run("test -t 1 && echo TTY_YES || echo TTY_NO").await;
    assert!(ran.stdout.contains("TTY_YES"), "got {:?}", ran.stdout);
}

/// And its lines are terminated the way a terminal terminates them -- the same
/// bytes a remote host now sends for the same command.
#[tokio::test]
async fn a_local_command_produces_the_same_bytes_a_remote_one_would() {
    enable_mock_store();
    let ran = run("printf 'a\\nb\\n'").await;
    assert_eq!(ran.stdout, "a\r\nb\r\n");
}

/// stderr stays its own stream, which is what lets the panel colour a failure
/// differently from a thousand lines of ordinary output.
#[tokio::test]
async fn stderr_stays_separable_on_the_local_path() {
    enable_mock_store();
    let ran = run("echo out; echo problem >&2").await;
    assert!(ran.stdout.contains("out"));
    assert!(!ran.stdout.contains("problem"));
    assert!(ran.stderr.contains("problem"));
}

#[tokio::test]
async fn a_failing_local_command_reports_its_own_code() {
    enable_mock_store();
    assert_eq!(run("exit 9").await.exit, Some(9));
}

/// The obligation the panel's state machine rests on: a run always says it
/// finished. Without it the panel sits in `STARTED` for the session.
#[tokio::test]
async fn every_local_run_reports_an_exit() {
    enable_mock_store();
    for command in ["true", "exit 4", "no-such-command-anywhere-at-all"] {
        assert!(
            run(command).await.exit.is_some(),
            "{command:?} never reported finishing"
        );
    }
}
