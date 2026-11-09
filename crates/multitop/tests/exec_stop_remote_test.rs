//! A remote run's agent tears itself down when the client stops (M.REMOTESTOP).
//!
//! On a remote host the client's `SIGTERM` (M.KILLDROP) reaches `ssh`, never
//! the agent: the agent is `ssh`'s remote child and hears nothing. It stops
//! only because its write to the closed channel fails, which the agent latches
//! (`Watched`, 0615a25) and the pump reads as "end the run". The local stop
//! test cannot see that path -- there the agent is signalled directly and its
//! own handler does the work -- so this one puts a relay between the two.
//!
//! The relay is `ssh`'s shape: the agent runs as a child that the client never
//! signals, its stdout runs through a pipe the relay holds, and the relay dying
//! closes that pipe.

// A test crate, said where clippy reads it (see child_isolation_test.rs).
#![cfg(test)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use multitop::ssh;
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

/// Install the relay as `<dir>/multitop-agent` (the name `local_agent_command`
/// runs as-is) and name it through the seam. It runs the real agent as the
/// first stage of a pipeline: a `SIGTERM` to the relay's pid ends the relay
/// alone, and the agent's next write meets a pipe whose reader is `cat`, which
/// ends on the client's closed stdout in turn -- as `sshd` closes the channel.
fn install_relay(dir: &Path) {
    let relay = dir.join("multitop-agent");
    let script = format!(
        "#!/bin/sh\n\"{}\" --agent \"$@\" | cat\n",
        env!("CARGO_BIN_EXE_multitop")
    );
    std::fs::write(&relay, script).expect("write relay");
    std::fs::set_permissions(&relay, std::fs::Permissions::from_mode(0o755)).expect("chmod relay");
    std::env::set_var("MULTITOP_AGENT_EXE", &relay);
}

#[tokio::test]
async fn a_remote_agent_ends_its_run_when_the_client_stops_the_relay() {
    // The server comes from the seam every test uses; the relay then replaces
    // the agent it names, so this test alone runs behind the relay.
    let server = common::local_server("localhost");
    let dir = tempfile::tempdir().expect("tempdir");
    install_relay(dir.path());

    let token = format!("remote-stop-{}", std::process::id());
    let _reap = ReapOnDrop(token.clone());
    // Ignores the hangup and SIGTERM, as a tool busy in a trap would, and
    // keeps writing: the writes are what meet the closed channel. Bounded by
    // its own clock, so a leak this test fails to reap still ends in a minute.
    let ready = format!("ready-{token}");
    let request = multitop_agent::exec::ExecFrame::Request {
        command: format!(
            "trap '' HUP TERM; echo {ready}; end=$(( $(date +%s) + 60 )); \
             while [ $(date +%s) -lt $end ]; do echo {token}; sleep 0.05; done"
        ),
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    let mut child = ssh::spawn_exec(&server, &request).await.expect("spawn");
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
    // The relay is what the client holds: the agent is a different process,
    // one the client's stop never names.
    let before = holders(&token);
    assert!(
        !before.is_empty(),
        "calibration: the command is not running, saw {before:?}"
    );

    // The client's stop: SIGTERM to the relay (the ssh), then the pipes close.
    drop(child);
    drop(stdout);

    // The agent's own grace for the command's session is 2 s; the client's
    // stop grace covers it. Past that bound the agent has not torn down.
    let deadline = Instant::now() + ssh::STOP_GRACE + Duration::from_secs(5);
    let mut left = holders(&token);
    while !left.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        left = holders(&token);
    }
    assert!(
        left.is_empty(),
        "the remote agent's run outlived the client's stop by {:?}: {left:?}",
        ssh::STOP_GRACE + Duration::from_secs(5)
    );
}
