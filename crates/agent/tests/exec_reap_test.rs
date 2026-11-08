//! A run whose client is gone ends its shell's whole session (M.FLOODREAP).
//!
//! Rust ignores `SIGPIPE`, so a client that vanished reached the agent only as
//! write errors -- which `send` dropped, and the run went on with nobody
//! reading. And the pid the agent forks is the session leader, not the tree:
//! an interactive shell moves itself into a process group of its own, so a
//! signal to the pid or its group leaves the command running, orphaned.

// A test crate, said where clippy reads it (see exec_run_test.rs).
#![cfg(test)]

use std::io::Write;
use std::time::{Duration, Instant};

use multitop_agent::exec::run::{run, Request};

/// A client that reads until it has seen `token`, and is gone after that:
/// every later write fails as a closed pipe does.
struct Vanishing {
    seen: Vec<u8>,
    token: Vec<u8>,
    gone: bool,
}

impl Write for Vanishing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.gone {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.seen.extend_from_slice(buf);
        self.gone = self.seen.windows(self.token.len()).any(|w| w == self.token);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.gone {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }
}

fn holders(token: &str) -> Vec<i32> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", token])
        .output()
        .expect("pgrep runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

/// The owner reaps, pass or fail: whatever the assertion leaves, this kills.
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

/// The command ignores both the hangup and `SIGTERM`, as a shell busy in a
/// trap or a tool that masks them would; only a `SIGKILL` to the group the
/// command runs in ends it. Bounded by its own clock, so a leak this test
/// fails to reap still ends within a minute.
#[test]
fn a_run_whose_client_is_gone_ends_its_whole_session() {
    let token = format!("reap-agent-{}", std::process::id());
    let _reap = ReapOnDrop(token.clone());
    let cmd = format!(
        "trap '' HUP TERM; end=$(( $(date +%s) + 60 )); \
         while [ $(date +%s) -lt $end ]; do echo {token}; sleep 0.05; done"
    );
    let req = Request {
        command: &cmd,
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
        host: "test",
        lock_path: None,
    };
    let mut out = Vanishing {
        seen: Vec::new(),
        token: token.as_bytes().to_vec(),
        gone: false,
    };
    let started = Instant::now();
    let _ = run(&req, &mut out);
    assert!(
        out.gone,
        "calibration: the run's output never reached the client"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "run() outlived its client by {:?}",
        started.elapsed()
    );
    // `run` reaps the leader before it returns; a group member is SIGKILLed
    // in the same breath, so the table settles within moments.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut left = holders(&token);
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        left = holders(&token);
    }
    assert!(
        left.is_empty(),
        "the run's shell outlived the run: pids {left:?}"
    );
}
