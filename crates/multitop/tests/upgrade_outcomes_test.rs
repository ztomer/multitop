//! What an upgrade run says when it ends, for every way it can end.
//!
//! Driven against a *local* server, so the upgrade command is a shell script
//! this test writes: that is the only way to make a run exit 111, print a
//! held-lock sentinel, or die on a signal on demand. Every branch here decides
//! what an operator is told, and telling them the wrong thing is what sends
//! them to read their upgrade script when the problem was the password.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use multitop::app::Msg;
use multitop::config::Server;
use multitop::password_store;
// The sentinels are the agent's now: both ends need one definition and the end
// that owns the pty is the one that can tell what a line is. What these tests
// assert is unchanged and is the point -- a command that *prints* a sentinel
// must not have it show up in the operator's log.
use multitop::tasks::spawn_upgrade;
use multitop_agent::exec::{LOCK_HELD_SENTINEL, SUDO_FAILED_SENTINEL};
use tokio::sync::mpsc;

/// Port 0 makes the server local, so `spawn_command` runs the script here
/// rather than reaching for `ssh`.
fn local_server(cmd: Option<&str>) -> Server {
    Server {
        host: "localhost".to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: cmd.map(str::to_string),
    }
}

/// The mock credential store also turns off the local upgrade lock file, so
/// these runs neither touch the OS keychain nor leave a lock in `~/.cache`.
async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Everything one upgrade run emitted: the log lines, and the closing note.
struct Run {
    lines: Vec<String>,
    note: String,
    success: bool,
}

impl Run {
    fn log(&self) -> String {
        self.lines.join("\n")
    }
}

async fn run_upgrade(cmd: Option<&str>, pass: Option<&str>) -> Run {
    let (tx, mut rx) = mpsc::channel::<Msg>(256);
    let handle = spawn_upgrade(0, 1, local_server(cmd), pass.map(str::to_string), tx);

    let mut lines = Vec::new();
    let mut note = None;
    let mut success = false;
    let collect = async {
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::AuxLine { line, .. } => lines.push(line),
                Msg::AuxRepaint { line, back, .. } => {
                    if back > 0 && lines.len() >= back {
                        lines.truncate(lines.len() - back);
                    }
                    lines.push(line);
                }
                Msg::AuxBegin {
                    header: Some(h), ..
                } => lines.push(h),
                Msg::AuxDone {
                    note: n,
                    success: s,
                    ..
                } => {
                    note = n;
                    success = s;
                    // Every exit must report AuxDone; nothing follows it.
                    break;
                }
                _ => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), collect)
        .await
        .expect("an upgrade must always finish by reporting AuxDone");
    handle.abort();

    Run {
        lines,
        note: note.expect("AuxDone must carry a note"),
        success,
    }
}

// ------------------------------------------------------------- nothing to run

#[tokio::test]
async fn a_server_with_no_upgrade_command_says_so_and_still_finishes() {
    // Returning without AuxDone would leave the panel reading "running"
    // forever and block every later upgrade.
    let _g = isolate().await;
    let run = run_upgrade(None, None).await;
    assert!(!run.success);
    assert!(run.note.contains("no upgrade_cmd"), "{}", run.note);
}

// ------------------------------------------------------------ ordinary outcomes

#[tokio::test]
async fn a_command_that_succeeds_reports_done() {
    let _g = isolate().await;
    let run = run_upgrade(Some("echo 'Reading package lists...'; exit 0"), None).await;
    assert!(run.success);
    assert!(run.note.contains("done"), "{}", run.note);
    assert!(run.log().contains("Reading package lists"), "{}", run.log());
    assert!(
        run.log().contains("Upgrade on localhost"),
        "the header is missing"
    );
}

#[tokio::test]
async fn a_command_that_merely_fails_is_not_blamed_on_the_network() {
    // Reporting every failure as "disconnected" blamed the network for a
    // command that exited non-zero on a host the stats view was talking to.
    let _g = isolate().await;
    let run = run_upgrade(Some("echo 'E: Broken packages' >&2; exit 100"), None).await;
    assert!(!run.success);
    assert!(run.note.contains("exited 100"), "{}", run.note);
    assert!(run.note.contains("host reachable"), "{}", run.note);
    assert!(
        run.log().contains("Broken packages"),
        "stderr was lost: {}",
        run.log()
    );
}

/// A command stopped by a signal must not be announced as a success, and the
/// operator must be told which signal.
///
/// `SIGKILL` in a nested `sh`, not `kill -TERM $$` as this test used to do.
/// Two reasons, both learned by watching it fail:
///
/// * an **interactive** shell ignores `SIGTERM`, and the command now runs under
///   `zsh -l -i` -- which is what makes an alias like `ud` resolve, and is what
///   the remote path always did. `kill -TERM $$` there kills nothing and the
///   run succeeds, which is correct and is not what this test is about.
/// * the agent's own child is `/bin/sh`, and a signal that kills something
///   nested inside it reaches this side as an ordinary exit status of 128+N.
///   `signalled` stays honest -- it means *our* child was signalled -- and the
///   128+N convention is decoded separately, because "exited 137" alone sends
///   an operator hunting a bug in a command the OOM killer stopped.
#[tokio::test]
async fn a_command_killed_by_a_signal_says_which_signal() {
    let _g = isolate().await;
    let run = run_upgrade(Some("sh -c 'kill -9 $$'"), None).await;
    assert!(!run.success, "a killed command is not a successful upgrade");
    assert!(
        run.note.contains("killed by signal 9"),
        "the signal has to be named: {}",
        run.note
    );
    assert!(run.note.contains("137"), "{}", run.note);
}

// ------------------------------------------------------------------ sentinels

#[tokio::test]
async fn a_refused_sudo_password_is_reported_as_that_not_as_a_failing_command() {
    // Saying "the command failed" here sends the user to read their upgrade
    // script when the problem is the password — the command never ran at all.
    let _g = isolate().await;
    // A leading newline because a login shell may write a prompt escape before
    // the first command's output, with no newline of its own — a marker has to
    // be alone on its line to be one. Production covers the other case with the
    // distinct exit code, which the test below pins separately.
    let run = run_upgrade(
        Some(&format!(r"printf '\n{SUDO_FAILED_SENTINEL}\n'; exit 111")),
        None,
    )
    .await;

    assert!(!run.success);
    assert!(run.note.contains("sudo refused"), "{}", run.note);
    assert!(run.note.contains("did not run"), "{}", run.note);
    assert!(
        !run.log().contains(SUDO_FAILED_SENTINEL),
        "an internal marker was printed into the operator's log:\n{}",
        run.log()
    );
}

#[tokio::test]
async fn the_sudo_sentinel_is_recognised_on_stderr_too() {
    // The local path keeps its pipes separate, so either stream can carry it;
    // one-sentinel-per-stream is what made the detection dead over `ssh -tt`.
    let _g = isolate().await;
    let run = run_upgrade(
        Some(&format!("echo {SUDO_FAILED_SENTINEL} >&2; exit 1")),
        None,
    )
    .await;
    assert!(run.note.contains("sudo refused"), "{}", run.note);
    assert!(!run.log().contains(SUDO_FAILED_SENTINEL), "{}", run.log());
}

#[tokio::test]
async fn a_held_lock_is_reported_with_the_file_to_remove() {
    // Naming the file is the whole fix: the lock only breaks automatically
    // after six hours, so a leftover from a killed run needs removing by hand.
    let _g = isolate().await;
    let run = run_upgrade(
        Some(&format!("echo {LOCK_HELD_SENTINEL} >&2; exit 125")),
        None,
    )
    .await;

    assert!(!run.success);
    assert!(run.note.contains("holds the lock"), "{}", run.note);
    assert!(run.note.contains("upgrade.lock"), "{}", run.note);
    assert!(!run.log().contains(LOCK_HELD_SENTINEL), "{}", run.log());
}

#[tokio::test]
async fn the_lock_sentinel_is_recognised_on_stdout_too() {
    // A remote runs under `ssh -tt`, and a pty has one stream.
    let _g = isolate().await;
    let run = run_upgrade(
        Some(&format!(r"printf '\n{LOCK_HELD_SENTINEL}\n'; exit 1")),
        None,
    )
    .await;
    assert!(run.note.contains("holds the lock"), "{}", run.note);
    assert!(!run.log().contains(LOCK_HELD_SENTINEL), "{}", run.log());
}

#[tokio::test]
async fn the_distinct_exit_codes_are_enough_on_their_own() {
    // The sentinel exists to survive a lost exit status in a noisy login
    // shell; the code has to work without the sentinel just the same.
    let _g = isolate().await;
    assert!(run_upgrade(Some("exit 111"), None)
        .await
        .note
        .contains("sudo refused"));
    assert!(run_upgrade(Some("exit 125"), None)
        .await
        .note
        .contains("holds the lock"));
}

// ----------------------------------------------------------------- sudo hints

#[tokio::test]
async fn sudo_asking_for_a_terminal_earns_the_hint_that_fits() {
    let _g = isolate().await;
    let script = "echo 'sudo: no tty present and no askpass program specified' >&2; exit 1";

    // With no password stored, the hint is to set one.
    let without = run_upgrade(Some(script), None).await;
    assert!(
        without.log().contains("Set password in settings"),
        "{}",
        without.log()
    );
    assert!(without.log().contains("NOPASSWD"), "{}", without.log());

    // With one stored, setting it again is not the advice — checking it is.
    let with = run_upgrade(Some(script), Some("hunter2")).await;
    assert!(
        with.log().contains("Check password in settings"),
        "{}",
        with.log()
    );
    assert!(
        !with.log().contains("Set password in settings"),
        "the wrong hint was given to someone who already has a password"
    );
}

#[tokio::test]
async fn a_command_that_merely_forgot_sudo_still_gets_a_hint() {
    // apt's "are you root?" contains no "sudo" at all, so without that arm a
    // missing `sudo` prefix was reported as a failing command with no hint.
    let _g = isolate().await;
    let run = run_upgrade(
        Some("echo 'E: Could not open lock file - are you root?' >&2; exit 100"),
        None,
    )
    .await;
    assert!(
        run.log().contains("Tip:"),
        "no hint was offered:\n{}",
        run.log()
    );
}

// -------------------------------------------------------------- log behaviour

#[tokio::test]
async fn a_repainting_progress_line_logs_only_what_it_ended_on() {
    // A tool that rewrites one line with carriage returns must contribute one
    // line, not one per tick.
    let _g = isolate().await;
    let run = run_upgrade(Some(r"printf '10%%\r50%%\r100%% done\n'; exit 0"), None).await;

    let body: Vec<&String> = run.lines.iter().filter(|l| l.contains('%')).collect();
    assert_eq!(
        body.len(),
        1,
        "one repainted line became {} lines: {body:?}",
        body.len()
    );
    assert!(body[0].contains("100%"), "{:?}", body[0]);
}

#[tokio::test]
async fn ssh_s_own_closing_chatter_is_left_out_of_the_log() {
    let _g = isolate().await;
    let run = run_upgrade(
        Some("echo 'Shared connection to web-01 closed.' >&2; echo 'real problem' >&2; exit 1"),
        None,
    )
    .await;
    assert!(run.log().contains("real problem"), "{}", run.log());
    assert!(
        !run.log().contains("Shared connection"),
        "ssh's own chatter reached the operator's log:\n{}",
        run.log()
    );
}

#[tokio::test]
async fn stderr_is_read_to_its_own_end_rather_than_stopping_with_stdout() {
    // The two pipes close together, so whichever `select!` polled first used to
    // decide whether stderr was read at all — and stderr is where the reason
    // lives. A run that failed for a nameable reason reported "exited 1" about
    // half the time.
    let _g = isolate().await;
    for _ in 0..5 {
        let run = run_upgrade(Some("echo out; echo 'the actual reason' >&2; exit 3"), None).await;
        assert!(
            run.log().contains("the actual reason"),
            "stderr was thrown away:\n{}",
            run.log()
        );
    }
}

#[tokio::test]
async fn a_flood_of_stderr_keeps_the_most_recent_lines() {
    // apt writes its progress display to stderr too, and a hundred rewrites of
    // one bar would otherwise evict the actual error message.
    let _g = isolate().await;
    let run = run_upgrade(
        Some("for i in $(seq 1 200); do echo \"noise $i\" >&2; done; exit 1"),
        None,
    )
    .await;
    let log = run.log();
    assert!(log.contains("noise 200"), "the newest line was dropped");
    assert!(
        !log.contains("noise 1\n"),
        "the buffer was not bounded:\n{log}"
    );
}
