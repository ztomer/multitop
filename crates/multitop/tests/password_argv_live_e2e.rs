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

/// The password to use. `MULTITOP_LIVE_PW` supplies a real one so the success
/// path can be checked; without it the wrong password above is used, which
/// still proves the leak properties but stops before sudo elevates.
///
/// Taken from the environment, never an argument -- putting it in argv is the
/// bug this test exists to guard.
fn password() -> (String, bool) {
    std::env::var("MULTITOP_LIVE_PW")
        .map_or_else(|_| (DUMMY_PW.to_string(), false), |real| (real, true))
}

/// Never let the secret reach an assertion message.
fn redact(text: &str, secret: &str) -> String {
    text.replace(secret, "<REDACTED>")
}

/// Drop CSI/OSC escapes so the listing can be recognised.
///
/// The target's `ls` is aliased to a colourising replacement, so the output
/// carries no `total N` header and the permission bits are wrapped in escapes --
/// a check for a line starting with `drwx` finds nothing even though the command
/// ran perfectly well.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                for n in chars.by_ref() {
                    if n == '\u{7}' || n == '\u{1b}' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

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

/// The command line of one process, by pid.
///
/// By pid rather than by searching the whole table: this harness receives the
/// real password through its own environment, so its own command line contains
/// the secret, and any text filter wide enough to catch the ssh process also
/// catches the harness -- including, on the first attempt, matching the filter's
/// own source text echoed in the invocation. The claim is about the process this
/// program spawned, so ask about exactly that process.
fn args_of(pid: u32) -> String {
    std::process::Command::new("/bin/ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "args="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Drive the readiness handshake and collect everything the remote said after
/// it: the same sequence the upgrade task performs.
async fn handshake_and_collect(child: &mut tokio::process::Child, secret: &str) -> Vec<String> {
    let stdout = child.stdout.take().expect("stdout piped");
    let mut lines = BufReader::new(stdout).lines();

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
        .write_all(format!("{secret}\n").as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();
    drop(stdin);

    let mut body = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        body.push(line);
    }
    let _ = child.wait().await;
    body
}

#[tokio::test]
#[ignore = "requires a live SSH host; set MULTITOP_LIVE_HOST"]
async fn the_password_is_never_in_argv_or_the_output() {
    let Some(server) = live_server() else {
        eprintln!("MULTITOP_LIVE_HOST unset -- skipping");
        return;
    };

    let (secret, is_real) = password();
    println!(
        "using {} password",
        if is_real {
            "the real"
        } else {
            "a deliberately wrong"
        }
    );
    let spawned = ssh::spawn_command(&server, "ls -l; ls -l", Some(secret.as_str()))
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
    let pid = child
        .id()
        .expect("the ssh child should have a pid while running");
    let argv = args_of(pid);
    assert!(
        !argv.trim().is_empty(),
        "could not read the ssh process arguments, so the check would pass vacuously"
    );
    assert!(
        argv.contains("ssh"),
        "expected the ssh process, got: {}",
        redact(&argv, &secret)
    );
    assert!(
        !argv.contains(secret.as_str()),
        "the password appeared in the ssh command line: {}",
        redact(&argv, &secret)
    );
    println!("ssh argv inspected (pid {pid}); the password is not in it");

    let body = handshake_and_collect(&mut child, &secret).await;

    let output = body.join("\n");
    // The pty would echo anything written before `stty -echo` ran; this is why
    // the writer waits for the sentinel rather than writing immediately.
    assert!(
        !output.contains(secret.as_str()),
        "the password was echoed back into the panel output:\n{}",
        redact(&output, &secret)
    );
    assert!(
        !output.contains(ssh::PW_READY_SENTINEL),
        "the sentinel leaked into panel output:\n{}",
        redact(&output, &secret)
    );
    println!("output clean, {} line(s) after the handshake", body.len());

    // With a real password sudo elevates and the `&&` runs the command, so the
    // listing has to be there. Without one, sudo refuses and nothing runs --
    // and asserting that is what stops a wrong password looking like success.
    // A long listing always shows permission bits, whichever `ls` is in use.
    let plain = strip_ansi(&output);
    let ran = plain.contains("rwx") || plain.contains("total ");
    if is_real {
        assert!(
            ran,
            "sudo accepted the password but the command did not run:\n{}",
            redact(&output, &secret)
        );
        println!("success path confirmed: sudo elevated and the command ran");
    } else {
        assert!(
            !ran,
            "a wrong password must not reach the command:\n{}",
            redact(&output, &secret)
        );
        println!("wrong password correctly stopped before the command");
    }
}
