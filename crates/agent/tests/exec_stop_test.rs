//! A stop the client sends the agent ends the run's whole session (M.KILLDROP).
//!
//! The client used to stop an exec agent with `SIGKILL` (tokio's
//! `kill_on_drop`), so the agent's own teardown never ran and a command that
//! ignores the hangup outlived the stop, orphaned to init. Now the client sends
//! `SIGTERM` and the agent tears the run down: both process groups, the
//! leader's and the terminal's foreground one.

// A test crate, said where clippy reads it (see exec_run_test.rs).
#![cfg(test)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use multitop_agent::exec::ExecFrame;
use multitop_agent::proto::{encode_packet, Payload};

/// `(pid, ppid)` of every process whose command line contains `needle`.
fn holders(needle: &str) -> Vec<(i32, i32)> {
    let out = Command::new("ps")
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
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
    }
}

/// The command ignores the hangup -- `nohup`'s whole trick, and what a shell
/// busy in a trap does -- and sleeps in the terminal's foreground group, not
/// the session leader's. Only the agent's teardown signalling that group ends
/// it; without it the sleep is left at PPID 1.
#[test]
fn a_term_to_the_agent_reaps_a_command_that_ignores_the_hangup() {
    // A sleep length unique to this run, so its command line names it.
    let secs = 900_000 + std::process::id() % 99_999;
    let needle = format!("sleep {secs}");
    let _reap = ReapOnDrop(needle.clone());
    let ready = format!("ready-{secs}");
    let command = format!("trap '' HUP; echo {ready}; {needle}");

    let mut agent = Command::new(env!("CARGO_BIN_EXE_multitop-agent"))
        .arg("exec")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the agent starts");
    let packet = encode_packet(&Payload::Exec(ExecFrame::Request {
        command,
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
    }));
    {
        let mut stdin = agent.stdin.take().expect("stdin");
        stdin.write_all(&packet).expect("request written");
    }

    // Read until the command says it is about to sleep.
    let mut stdout = agent.stdout.take().expect("stdout");
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    let started = Instant::now();
    while !seen.windows(ready.len()).any(|w| w == ready.as_bytes()) {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "calibration: the command never started"
        );
        let n = stdout.read(&mut buf).expect("agent output");
        assert!(n > 0, "calibration: the agent ended before the command ran");
        seen.extend_from_slice(&buf[..n]);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while holders(&needle).is_empty() {
        assert!(
            Instant::now() < deadline,
            "calibration: the sleep never ran"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The client's stop.
    let sent = Command::new("kill")
        .args(["-TERM", &agent.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(sent.success());
    // Drain so the agent's last frames never block on a full pipe.
    let drain = std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = stdout.read_to_end(&mut rest);
    });
    let stop_started = Instant::now();
    let status = agent.wait().expect("the agent is reaped");
    let _ = drain.join();
    assert!(
        stop_started.elapsed() < Duration::from_secs(10),
        "the agent took {:?} to stop",
        stop_started.elapsed()
    );
    assert!(
        status.code().is_some(),
        "the agent died of the signal instead of tearing the run down: {status:?}"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut left = holders(&needle);
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        left = holders(&needle);
    }
    let orphans: Vec<_> = left.iter().filter(|(_, ppid)| *ppid == 1).collect();
    assert!(
        orphans.is_empty(),
        "the command was orphaned to init: {orphans:?}"
    );
    assert!(left.is_empty(), "the command outlived the stop: {left:?}");
}
