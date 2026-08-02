//! Live check that the sudo password never becomes visible.
//!
//! `#[ignore]`d: needs a real host reachable over SSH with key auth. Run with
//!
//! ```text
//! MULTITOP_LIVE_HOST=192.168.0.33 MULTITOP_LIVE_USER=ztomer \
//!   cargo test --release --test password_argv_live_e2e -- --ignored --nocapture
//! ```
//!
//! The upgrade command is fixed to `ls -l; ls -l` so a live run cannot change
//! anything on the target.
//!
//! The password used here is deliberately wrong. That is enough to prove the
//! properties that matter -- the secret must not appear in any process's
//! arguments, and the pty must not echo it back into the panel -- and it means
//! the run cannot actually elevate on someone's machine. It does cost one failed
//! `sudo` authentication in the host's auth log.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::config::Server;
use multitop::ssh;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Not a real credential, and long enough to be unambiguous in a `ps` dump.
const DUMMY_PW: &str = "multitop-live-probe-not-a-real-password-8f3a1c";

fn live_server() -> Option<Server> {
    let host = std::env::var("MULTITOP_LIVE_HOST").ok()?;
    Some(Server {
        host,
        port: std::env::var("MULTITOP_LIVE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(22),
        user: std::env::var("MULTITOP_LIVE_USER").unwrap_or_default(),
        upgrade_cmd: Some("ls -l; ls -l".to_string()),
    })
}

/// Every command line on this machine, so we can prove the secret is in none.
fn all_process_args() -> String {
    std::process::Command::new("/bin/ps")
        .args(["-ww", "-A", "-o", "args="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "requires a live SSH host; set MULTITOP_LIVE_HOST"]
async fn the_password_is_never_in_argv_or_the_output() {
    let Some(server) = live_server() else {
        eprintln!("MULTITOP_LIVE_HOST unset -- skipping");
        return;
    };

    let spawned = ssh::spawn_command(&server, "ls -l; ls -l", Some(DUMMY_PW))
        .expect("spawn over ssh should succeed with key auth");
    let ssh::Spawned {
        mut child,
        awaits_password,
    } = spawned;

    assert!(
        awaits_password,
        "a real (non-mock) password path must ask for a stdin handshake"
    );

    // Sample the process table while ssh is alive. This is the property the
    // change exists for: the command used to carry `echo '<password>' | sudo -S`
    // as an argument, and process arguments are readable by anyone.
    let snapshot = all_process_args();
    assert!(
        !snapshot.contains(DUMMY_PW),
        "the password appeared in a process command line"
    );
    let ssh_lines: Vec<&str> = snapshot
        .lines()
        .filter(|l| l.contains("ssh ") && l.contains(&server.host))
        .collect();
    assert!(
        !ssh_lines.is_empty(),
        "expected to see the ssh process in order to inspect its arguments"
    );
    println!(
        "ssh argv seen, {} line(s), none containing the secret",
        ssh_lines.len()
    );

    let stdout = child.stdout.take().expect("stdout piped");
    let mut lines = BufReader::new(stdout).lines();

    // Same handshake the upgrade task performs.
    let mut ready = false;
    let mut preamble = Vec::new();
    for _ in 0..50 {
        match lines.next_line().await {
            Ok(Some(line)) if line.trim() == ssh::PW_READY_SENTINEL => {
                ready = true;
                break;
            }
            Ok(Some(l)) => preamble.push(l),
            _ => break,
        }
    }
    assert!(
        ready,
        "no readiness sentinel from the remote; saw: {preamble:?}"
    );
    println!(
        "sentinel received after {} preamble line(s)",
        preamble.len()
    );

    let mut stdin = child.stdin.take().expect("stdin piped");
    stdin
        .write_all(format!("{DUMMY_PW}\n").as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();
    drop(stdin);

    let mut body = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        body.push(line);
    }
    let _ = child.wait().await;

    let output = body.join("\n");
    // The pty would echo anything written before `stty -echo` ran; this is why
    // the writer waits for the sentinel rather than writing immediately.
    assert!(
        !output.contains(DUMMY_PW),
        "the password was echoed back into the panel output:\n{output}"
    );
    assert!(
        !output.contains(ssh::PW_READY_SENTINEL),
        "the sentinel leaked into panel output:\n{output}"
    );
    println!("output clean, {} line(s) after the handshake", body.len());
}
