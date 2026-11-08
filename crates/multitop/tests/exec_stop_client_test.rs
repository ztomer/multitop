//! Dropping the exec agent lets it tear its run down (M.KILLDROP).
//!
//! The client stopped the agent with tokio's `kill_on_drop`, a `SIGKILL`: the
//! agent's teardown never ran, and a command that ignores the hangup was left
//! running at PPID 1. The drop now sends `SIGTERM`, then `SIGKILL` after a grace.

// A test crate, said where clippy reads it (see child_isolation_test.rs).
#![cfg(test)]

mod common;

use std::time::{Duration, Instant};

use multitop::{password_store, ssh};
use tokio::io::AsyncReadExt;

/// `(pid, ppid)` of every process whose command line contains `needle`.
fn holders(needle: &str) -> Vec<(i32, i32)> {
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(needle) && !l.contains("ps -axo"))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect()
}

/// The owner reaps, pass or fail.
struct ReapOnDrop(String);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        for (pid, _) in holders(&self.0) {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
        }
    }
}

#[tokio::test]
async fn dropping_the_agent_reaps_a_command_that_ignores_the_hangup() {
    let _guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();

    let secs = 800_000 + std::process::id() % 99_999;
    let needle = format!("sleep {secs}");
    let _reap = ReapOnDrop(needle.clone());
    let ready = format!("ready-{secs}");
    let request = multitop_agent::exec::ExecFrame::Request {
        command: format!("trap '' HUP; echo {ready}; {needle}"),
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    let mut child = ssh::spawn_exec(&common::local_server("localhost"), &request)
        .await
        .expect("spawn");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    let started = Instant::now();
    while !seen.windows(ready.len()).any(|w| w == ready.as_bytes()) {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "calibration: the command never started"
        );
        let n = tokio::time::timeout(Duration::from_secs(30), stdout.read(&mut buf))
            .await
            .expect("agent output in time")
            .expect("agent output");
        assert!(n > 0, "calibration: the agent ended before the command ran");
        seen.extend_from_slice(&buf[..n]);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while holders(&needle).is_empty() {
        assert!(
            Instant::now() < deadline,
            "calibration: the sleep never ran"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The client's stop: the run's owner goes away.
    drop(child);
    drop(stdout);

    let deadline = Instant::now() + ssh::STOP_GRACE + Duration::from_secs(5);
    let mut left = holders(&needle);
    while !left.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        left = holders(&needle);
    }
    let orphans: Vec<_> = left.iter().filter(|(_, ppid)| *ppid == 1).collect();
    assert!(
        orphans.is_empty(),
        "the command was orphaned to init: {orphans:?}"
    );
    assert!(left.is_empty(), "the command outlived the stop: {left:?}");
}
