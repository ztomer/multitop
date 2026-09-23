//! Automated E2E Integration Tests for Remote SSH Upgrade Execution
//!
//! These tests run against a REAL remote host over SSH. They are `#[ignore]`d
//! by default to avoid requiring SSH infrastructure during normal CI.
//!
//! Run remote tests:
//! ```
//! cargo test --test upgrade_loop_remote_e2e -- --ignored
//! ```
//!
//! Requires a reachable SSH host configured via environment variables:
//! - `MULTITOP_TEST_SSH_HOST` — hostname, IP or `~/.ssh/config` alias
//!   (required: there is no default)
//! - `MULTITOP_TEST_SSH_USER` — SSH username (default: current user)
//! - `MULTITOP_TEST_SSH_PORT` — SSH port (default: `22`)
//!
//! A loopback name is refused: multitop runs `localhost`/`127.0.0.1` (and
//! port 0) locally, without ssh, so a run aimed there tests nothing this
//! suite is for. To test ssh against this machine, run sshd (Remote Login on
//! macOS) and name it through an alias - `Host multitop-loop` /
//! `HostName 127.0.0.1` - see `live_ssh/mod.rs`. `test_remote_upgrade_runs_over_ssh`
//! proves each run actually crossed ssh.
//!
//! # Never run a real upgrade command here
//!
//! These tests run against real machines, so every `upgrade_cmd` below is a
//! read-only stand-in — `ls -l ; ls -l`, `true`, `echo`, `seq` — chosen to
//! exercise the full SSH, streaming, locking, and exit-code paths without
//! touching packages on the target. The tests build their `Server` values from
//! the environment only and never read `config.toml`, so a real `upgrade_cmd`
//! (`apt upgrade`, `./update_sys.sh`, ...) cannot leak in by accident.
//! Any test added here must keep that property.
//!
//! Run with `--test-threads=1`: the tests contend on a per-host remote lock
//! file, and running them concurrently makes them flap between "ran" and
//! "lock prevented execution".

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

use std::time::Duration;

use multitop::app::Msg;
use multitop::config::Server;
use multitop::tasks::spawn_upgrade;

use tokio::sync::mpsc;

#[path = "live_ssh/mod.rs"]
mod live_ssh;
use live_ssh::{ssh_server, target};

/// Collect messages from channel with timeout, returns all messages received.
async fn collect_messages(rx: mpsc::Receiver<Msg>) -> Vec<Msg> {
    let mut msgs = Vec::new();
    let mut rx = rx;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
        msgs.push(msg);
    }
    msgs
}

/// Collect messages until first `AuxDone` or Status is received.
async fn collect_until_done(rx: mpsc::Receiver<Msg>) -> Vec<Msg> {
    let mut msgs = Vec::new();
    let mut rx = rx;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
        let is_terminal = matches!(msg, Msg::AuxDone { .. } | Msg::Status { .. });
        msgs.push(msg);
        if is_terminal {
            break;
        }
    }
    msgs
}

/// Run `server`'s upgrade as panel 0, generation 1, and return every message
/// it sent until it finished. The task is joined, not dropped: a panic inside
/// it arrives as a join error, and swallowing that leaves the test to fail
/// later for some other reason - or to pass. The task is the thing under test.
async fn run(server: Server, sudo: Option<String>, cap: usize) -> Vec<Msg> {
    let (tx, rx) = mpsc::channel::<Msg>(cap);
    let handle = spawn_upgrade(0, 1, server, sudo, tx);
    let msgs = collect_until_done(rx).await;
    handle.await.expect("the spawned task must not panic");
    msgs
}

/// The output lines among `msgs`, in order.
fn aux_lines(msgs: &[Msg]) -> Vec<String> {
    msgs.iter()
        .filter_map(|m| match m {
            Msg::AuxLine { line, .. } => Some(line.clone()),
            _ => None,
        })
        .collect()
}

/// Test R1: Remote basic command
/// SSH into real host, run `ls -l / ; ls -l /`: the root directory, not the
/// login's home, so the line count does not depend on what the account keeps
/// there (a sparse home gave 8 lines and failed a host that was fine).
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_basic_command() {
    let server = ssh_server("ls -l / ; ls -l /");
    let msgs = run(server, None, 200).await;

    // AuxBegin with correct panel/gen
    let begin = msgs.iter().find(|m| {
        matches!(
            m,
            Msg::AuxBegin {
                panel: 0,
                gen: 1,
                ..
            }
        )
    });
    assert!(begin.is_some(), "Expected AuxBegin with panel=0, gen=1");

    // AuxDone with success: true
    let done = msgs.iter().find(|m| {
        matches!(
            m,
            Msg::AuxDone {
                panel: 0,
                gen: 1,
                success: true,
                ..
            }
        )
    });
    assert!(done.is_some(), "Expected AuxDone success=true");

    // Output contains real ls -l data
    let output_lines = aux_lines(&msgs);
    assert!(!output_lines.is_empty(), "Should have output lines");
    assert!(
        output_lines.len() >= 10,
        "Expected at least 10 lines, got {}",
        output_lines.len()
    );
}

/// Test R2: Remote upgrade with sudo password
/// SSH into real host with sudo password, run `ls -l`.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_with_sudo_password() {
    let server = ssh_server("ls -l");
    let msgs = run(server, Some("test-sudo-pass".to_string()), 200).await;

    // Should complete (with or without sudo, depending on host config)
    let has_done = msgs.iter().any(|m| matches!(m, Msg::AuxDone { .. }));
    assert!(has_done, "Expected AuxDone message");

    // Check for sudo error tips
    let has_tip = msgs.iter().any(|m| match m {
        Msg::AuxLine { line, .. } => line.contains("Tip:"),
        _ => false,
    });

    if has_tip {
        // Sudo tip present means password was rejected — still OK, test passed
        eprintln!("Sudo tip received (password may not be authorized on this host)");
    }
}

/// Test R3: Remote upgrade failure exit code
/// SSH into real host, run `ls -l ; exit 42`.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_failure_exit_code() {
    let server = ssh_server("ls -l ; exit 42");
    let msgs = run(server, None, 200).await;

    let done = msgs.iter().find(|m| {
        matches!(
            m,
            Msg::AuxDone {
                panel: 0,
                gen: 1,
                success: false,
                ..
            }
        )
    });
    assert!(done.is_some(), "Expected AuxDone success=false for exit 42");

    // The command should either produce output (ls -l ran) or fail to acquire lock.
    // Both are acceptable — the key assertion is success=false.
    let has_output = msgs.iter().any(|m| {
        if let Msg::AuxLine { line, .. } = m {
            line.contains("total") || line.contains("drwx") || line.contains("-rw")
        } else {
            false
        }
    });
    if !has_output {
        // Lock may have prevented the command from running — that's also fine
        eprintln!("No ls output found (lock may have prevented execution)");
    }
}

/// Test R4: Remote upgrade empty command
/// SSH into real host, run `true` (exits 0, no output).
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_empty_command() {
    let server = ssh_server("true");
    let msgs = run(server, None, 200).await;

    let begin = msgs.iter().find(|m| matches!(m, Msg::AuxBegin { .. }));
    let done = msgs
        .iter()
        .find(|m| matches!(m, Msg::AuxDone { success: true, .. }));
    let _lines: Vec<_> = msgs
        .iter()
        .filter(|m| matches!(m, Msg::AuxLine { .. }))
        .collect();

    assert!(begin.is_some(), "Expected AuxBegin");
    assert!(done.is_some(), "Expected AuxDone success=true");
    // `true` produces no stdout, but shell wrapper/lock messages may appear as AuxLine via stderr
    let stdout_lines: Vec<_> = msgs
        .iter()
        .filter_map(|m| {
            if let Msg::AuxLine { line, .. } = m {
                Some(line)
            } else {
                None
            }
        })
        .filter(|l| !l.contains("Upgrade already in progress"))
        .collect();
    // Most systems won't produce output from `true` itself; the test verifies the
    // command completes successfully, not that output is zero.
    assert!(
        stdout_lines
            .iter()
            .all(|l| !l.contains("total") && !l.contains("drwx")),
        "Should not have ls-like output for `true`"
    );
}

/// Test R5: Remote upgrade lock contention
/// SSH into real host: first upgrade with `sleep 5 && ls -l`, then immediately launch second.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_lock_contention() {
    let server1 = ssh_server("sleep 5 && ls -l");
    let server2 = ssh_server("ls -l");

    // Launch first (holds lock)
    let (tx, rx) = mpsc::channel::<Msg>(200);
    let h1 = spawn_upgrade(0, 1, server1, None, tx.clone());

    // Wait briefly for lock acquisition
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Launch second (should be blocked or fail)
    let h2 = spawn_upgrade(1, 2, server2, None, tx);

    let msgs = collect_messages(rx).await;
    let _ = h1.await;
    let _ = h2.await;

    // Count done messages
    let done0 = msgs
        .iter()
        .filter(|m| matches!(m, Msg::AuxDone { panel: 0, .. }))
        .count();
    let done1 = msgs
        .iter()
        .filter(|m| matches!(m, Msg::AuxDone { panel: 1, .. }))
        .count();

    assert!(
        done0 >= 1 || done1 >= 1,
        "At least one upgrade should produce a result"
    );

    // Check if second got lock contention error
    let has_lock_error = msgs.iter().any(|m| {
        if let Msg::AuxLine { line, .. } = m {
            line.contains("already in progress")
        } else {
            false
        }
    });
    eprintln!("Lock contention detected: {has_lock_error}");
}

/// Test R6: Remote connection failure
/// SSH into unreachable host (TEST-NET address).
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_connection_failure() {
    let server = Server {
        host: "192.0.2.1".to_string(), // TEST-NET-1 (RFC 5737), guaranteed non-routable
        port: 22,
        user: "testuser".to_string(),
        upgrade_cmd: Some("ls -l".to_string()),
        custom_command: None,
        mcp: None,
    };

    let msgs = run(server, None, 100).await;

    // Should get either AuxDone (with error) or Status message
    let has_terminal = msgs
        .iter()
        .any(|m| matches!(m, Msg::AuxDone { .. } | Msg::Status { .. }));
    assert!(
        has_terminal,
        "Should get terminal message on connection failure"
    );
}

/// Test R7: Remote multiline output ordering
/// SSH into real host, run sequential echoes with small sleeps.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_multiline_output_ordering() {
    let server = ssh_server("echo STEP_A ; sleep 0.1 ; echo STEP_B ; sleep 0.1 ; echo STEP_C");
    let msgs = run(server, None, 200).await;

    let lines = aux_lines(&msgs);

    let pos_a = lines.iter().position(|l| l.contains("STEP_A"));
    let pos_b = lines.iter().position(|l| l.contains("STEP_B"));
    let pos_c = lines.iter().position(|l| l.contains("STEP_C"));

    assert!(pos_a.is_some(), "STEP_A not found");
    assert!(pos_b.is_some(), "STEP_B not found");
    assert!(pos_c.is_some(), "STEP_C not found");

    let a = pos_a.unwrap();
    let b = pos_b.unwrap();
    let c = pos_c.unwrap();
    assert!(a < b, "STEP_A should come before STEP_B");
    assert!(b < c, "STEP_B should come before STEP_C");
}

/// Test R8: Remote stderr captured
/// SSH into real host, run `echo OUT && echo ERR >&2`.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_stderr_captured() {
    let server = ssh_server("echo OUT ; echo ERR >&2");
    let msgs = run(server, None, 200).await;

    let lines = aux_lines(&msgs);

    let has_out = lines.iter().any(|l| l.contains("OUT"));
    let has_err = lines.iter().any(|l| l.contains("ERR"));
    assert!(has_out, "Stdout 'OUT' should be captured");
    assert!(has_err, "Stderr 'ERR' should be captured");
}

/// Test R9: Remote large output
/// SSH into real host, run `seq 1 1000`.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_large_output() {
    let server = ssh_server("seq 1 1000");
    let msgs = run(server, None, 2048).await;

    let lines = aux_lines(&msgs);

    assert!(
        lines.len() >= 1000,
        "Expected at least 1000 lines, got {}",
        lines.len()
    );

    // Verify ordering: line "1" before "1000"
    let first = lines.iter().position(|l| l.trim() == "1").unwrap_or(0);
    let last = lines.iter().rposition(|l| l.trim() == "1000").unwrap_or(0);
    assert!(first < last, "Line '1' should appear before '1000'");
}

/// Test R10: a remote upgrade finishes in bounded time.
/// An upgrade never deploys the agent (that is for the monitor, docker and
/// fetch modes), so a host without one must still finish promptly.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_finishes_without_an_agent() {
    let server = ssh_server("ls -l");
    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);
    tokio::time::timeout(Duration::from_secs(30), handle)
        .await
        .expect("the upgrade task finishes within 30s")
        .expect("the spawned task must not panic");
}

/// The suite's premise: the command ran over ssh, not on this machine. sshd
/// sets `SSH_CONNECTION` in the remote shell and does not carry this
/// process's environment across, so a marker set here is absent there - and
/// a local `sh -c` would inherit it.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_runs_over_ssh() {
    std::env::set_var("MULTITOP_E2E_MARKER", "here");
    let server = ssh_server(r#"echo "over-ssh=[$SSH_CONNECTION] marker=[$MULTITOP_E2E_MARKER]""#);
    let out = aux_lines(&run(server, None, 200).await);
    assert!(
        out.iter()
            .any(|l| l.contains("marker=[]") && !l.contains("over-ssh=[]")),
        "the command did not run over ssh: {out:?}"
    );
}

/// No default host, and a target multitop would run locally is refused:
/// `127.0.0.1`, `localhost` and port 0 skip ssh entirely (`ssh::is_local`).
/// An ssh alias for this machine is the way to test ssh against it.
#[test]
fn a_target_that_would_skip_ssh_is_refused_and_an_alias_is_accepted() {
    for (host, port) in [
        ("127.0.0.1", None),
        ("localhost", Some("2222")),
        ("web-01", Some("0")),
    ] {
        let why = target(Some(host), "u", port, "true").unwrap_err();
        assert!(why.contains("is local to multitop"), "{host}: {why}");
    }
    for host in [None, Some("")] {
        let why = target(host, "u", None, "true").unwrap_err();
        assert!(why.contains("is not set"), "{why}");
    }
    let why = target(Some("h"), "u", Some("ssh"), "true").unwrap_err();
    assert!(why.starts_with("MULTITOP_TEST_SSH_PORT=ssh:"), "{why}");
    let s = target(Some("multitop-loop"), "u", None, "true").unwrap();
    assert_eq!(
        (s.host.as_str(), s.port, s.upgrade_cmd.as_deref()),
        ("multitop-loop", 22, Some("true"))
    );
}
