//! A run's shell dies with the test that started it, pass or fail.
//!
//! The class (M.FLOODREAP, 2026-11-08): the flood test's interactive login
//! shell was found orphaned to launchd 12h23m after the run that started it,
//! blocked writing to a pty nobody read. The happy path reaps it -- `q`, then
//! the agent's own wait -- so a passing suite never shows the leak. The paths
//! that leak are the ones a test only takes on a bad day: a panic mid-run,
//! which drops the loop and `SIGKILL`s the agent, and a harness that kills the
//! test binary at its timeout, which runs no destructor at all.
//!
//! So the test here is a FAILING test, driven on purpose: `victim` starts a
//! real run and then either panics or waits to be killed, and the reaper
//! asserts that no shell carrying the victim's token outlives it.

use super::*;

/// The victim's token, and how it is to end. Unset: the victim is a no-op.
const TOKEN_VAR: &str = "MULTITOP_REAP_VICTIM_TOKEN";
const MODE_VAR: &str = "MULTITOP_REAP_VICTIM_MODE";

/// A command that keeps a shell busy, and ends on its own clock: the bound is
/// what keeps a leak this test fails to reap from outliving the run by more
/// than two minutes.
fn victim_cmd(token: &str) -> String {
    format!(
        "end=$(( $(date +%s) + 120 )); while [ $(date +%s) -lt $end ]; do \
         echo reap-{token}; sleep 0.05; done"
    )
}

/// Start a run, wait for it to be live, then fail the way `MODE_VAR` says.
/// Driven only by the reaper below, in a child process.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "driven by the_run_shell_dies_with_the_test_that_started_it"]
async fn victim() {
    let (Ok(token), Ok(mode)) = (std::env::var(TOKEN_VAR), std::env::var(MODE_VAR)) else {
        return;
    };
    let _keychain = isolate_keychain().await;
    let servers = vec![local_server(42031, &victim_cmd(&token))];
    let (h, tx) = PacedHarness::start(servers, (80, 24));
    press_until(
        &tx,
        KeyCode::Char('u'),
        || state_says_started(&h.cfg, "admin@127.0.0.1:42031"),
        "the victim's run never started",
    )
    .await;
    // `started_at` is written as the run is dispatched, before the agent has
    // forked its shell. The shell is live once its line is in the process
    // table, and only then is there anything to leak; tell the reaper so, on
    // stdout, which it reads.
    settle(
        || !holders(&token).is_empty(),
        "the victim's shell reaching the process table",
        || state_dump(&h.cfg),
    )
    .await;
    println!("victim-started");
    assert!(mode != "panic", "the victim fails mid-run, on purpose");
    // `kill`: wait for the reaper's SIGKILL, as a harness timeout delivers it.
    // Bounded, so a reaper that never sends it cannot leave this running.
    tokio::time::sleep(SETTLE_CEILING).await;
}

/// The pids of every process whose command line carries `token`.
fn holders(token: &str) -> Vec<i32> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", &format!("reap-{token}")])
        .output()
        .expect("pgrep runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

/// Reaps whatever the token names when the reaper itself ends, pass or fail:
/// the owner of a spawned process reaps it, and a failing assertion is the
/// path that would otherwise skip the cleanup.
struct ReapOnDrop(String);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        // `kill(1)` rather than `libc::kill`: no `unsafe` in a test file.
        for pid in holders(&self.0) {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
        }
    }
}

/// The victim process, killed and waited for when the reaper ends, pass or
/// fail: a failing assertion is the path that would otherwise skip it.
struct VictimGuard(std::process::Child);

impl Drop for VictimGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_victim(mode: &str) {
    use std::io::BufRead as _;
    let token = format!("{}-{mode}", std::process::id());
    let _reap = ReapOnDrop(token.clone());
    let mut child = VictimGuard(
        std::process::Command::new(std::env::current_exe().expect("own exe"))
            .args(["--ignored", "--exact", "reap::victim", "--nocapture"])
            .env(TOKEN_VAR, &token)
            .env(MODE_VAR, mode)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the victim starts"),
    );
    let stdout = child.0.stdout.take().expect("piped");
    let started = std::io::BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .any(|l| l.contains("victim-started"));
    assert!(started, "the victim never reported its run started");
    // Calibration lives in the victim: it says `victim-started` only once its
    // shell is in the process table, so a pass here is never a pass over a
    // shell that never existed.
    if mode == "kill" {
        let _ = child.0.kill();
    }
    let _ = child.0.wait();
    let deadline = std::time::Instant::now() + SETTLE_CEILING;
    let mut left = holders(&token);
    while !left.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        left = holders(&token);
    }
    assert!(
        left.is_empty(),
        "the run's shell outlived the test that started it ({mode}): pids {left:?}"
    );
}

/// A test that panics mid-run takes its run's shell with it.
#[test]
fn the_run_shell_dies_with_a_test_that_panics() {
    run_victim("panic");
}

/// A test killed at its timeout (SIGKILL: no destructor runs in it) still
/// takes its run's shell with it.
#[test]
fn the_run_shell_dies_with_a_test_that_is_killed() {
    run_victim("kill");
}
