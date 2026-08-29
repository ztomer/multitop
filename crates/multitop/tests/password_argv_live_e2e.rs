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
use multitop_agent::exec::ExecFrame;
use multitop_agent::proto::{decode_packet, Payload, HEADER_LEN, MAGIC};

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

/// Read the exec channel to its end and return what the operator would see.
///
/// There is no handshake here any more. The readiness sentinel and the write
/// that answered it were the client's job when the client had to find a line in
/// a stream whose shape it did not control; the password is a field in the
/// request now, and the agent -- which owns the pty -- writes it when the far
/// side says echo is off.
async fn collect_output(child: &mut tokio::process::Child) -> (String, Option<i32>) {
    use tokio::io::AsyncReadExt;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut raw = Vec::new();
    let _ = stdout.read_to_end(&mut raw).await;
    let _ = child.wait().await;

    let mut text = Vec::new();
    let mut exit = None;
    let mut pos = 0;
    while pos + HEADER_LEN <= raw.len() {
        if &raw[pos..pos + 4] != MAGIC {
            break;
        }
        let len =
            u16::from_le_bytes([raw[pos + HEADER_LEN - 2], raw[pos + HEADER_LEN - 1]]) as usize;
        let end = pos + HEADER_LEN + len;
        if end > raw.len() {
            break;
        }
        if let Some(Payload::Exec(frame)) = decode_packet(&raw[pos..end]) {
            match frame {
                ExecFrame::Out { bytes, .. } => text.extend_from_slice(&bytes),
                ExecFrame::Exit { code, .. } => exit = Some(code),
                _ => {}
            }
        }
        pos = end;
    }
    (String::from_utf8_lossy(&text).into_owned(), exit)
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

    let command = server.upgrade_cmd.clone().expect("a command to run");
    let request = ExecFrame::Request {
        command: command.clone(),
        password: Some(secret.clone()),
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    let mut child = ssh::spawn_exec(&server, &request)
        .await
        .expect("spawn over ssh should succeed with key auth");

    // The property this test exists for, sampled while ssh is alive. The
    // password used to be `echo '<password>' | sudo -S` inside an argument, and
    // `/proc/<pid>/cmdline` is world-readable.
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
    // And now the command is not there either. It was, under the old transport:
    // the whole `upgrade_cmd` was interpolated into the remote argument, so a
    // host's package mirrors and internal names were readable by every account
    // on this machine for the length of a run.
    assert!(
        !argv.contains(&command),
        "the upgrade command appeared in the ssh command line: {}",
        redact(&argv, &secret)
    );
    println!("ssh argv inspected (pid {pid}); neither the password nor the command is in it");

    let (output, exit) = collect_output(&mut child).await;

    // The pty would echo anything written before `stty -echo` ran, which is why
    // the agent waits for the marker rather than writing immediately.
    assert!(
        !output.contains(secret.as_str()),
        "the password was echoed back into the panel output:\n{}",
        redact(&output, &secret)
    );
    for sentinel in [
        multitop_agent::exec::PW_READY_SENTINEL,
        multitop_agent::exec::SUDO_FAILED_SENTINEL,
        multitop_agent::exec::STARTED_SENTINEL,
        multitop_agent::exec::DONE_SENTINEL,
    ] {
        assert!(
            !output.contains(sentinel),
            "{sentinel} leaked into panel output:\n{}",
            redact(&output, &secret)
        );
    }
    println!("output clean, {} byte(s) of it", output.len());

    // With a real password sudo elevates and the command runs, so the listing
    // has to be there. Without one, sudo refuses and nothing runs -- and
    // asserting that is what stops a wrong password looking like success.
    let plain = strip_ansi(&output);
    let ran = plain.contains("rwx") || plain.contains("total ");
    if is_real {
        assert!(
            ran,
            "sudo accepted the password but the command did not run:\n{}",
            redact(&output, &secret)
        );
        assert_eq!(exit, Some(0), "a successful run must report exit 0");
        println!("success path confirmed: sudo elevated and the command ran");
    } else {
        assert!(
            !ran,
            "a wrong password must not reach the command:\n{}",
            redact(&output, &secret)
        );
        assert_eq!(
            exit,
            Some(multitop_agent::exec::SUDO_FAILED_CODE),
            "a refused password must be reported as its own outcome, not as a failing command"
        );
        println!("wrong password correctly stopped before the command, and said so");
    }
}
