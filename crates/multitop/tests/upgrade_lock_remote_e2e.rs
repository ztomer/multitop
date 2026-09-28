//! The upgrade lock, over a real SSH connection to a real host.
//!
//! Its own file because the lock is a separate subsystem with a separate
//! observable: it is a directory on the REMOTE host, at a path
//! `multitop_agent::exec::lock` defines, and nothing the client sends says when
//! it was taken. Every other assertion in the live suites is mediated by the
//! product's own output; this one has to ask the host.
//!
//! It was `test_remote_upgrade_lock_contention` inside
//! `upgrade_loop_remote_e2e.rs`, and it asserted nothing about contention. It
//! `eprintln!`d the result instead of asserting it; it read the message out of
//! the wrong `Msg` variant (the verdict arrives in `AuxDone.note`, not in
//! `AuxLine.line`); the string it wanted, "already in progress", was wording
//! from the quoted-shell lock wrappers that `multitop_agent::exec::lock`
//! replaced, so that match was dead and the flag it computed was provably
//! always false; and its one hard assertion, "at least one upgrade should
//! produce a result", holds whether or not locking works at all.
//!
//! Its `upgrade_cmd`s are read-only stand-ins (`/bin/ls -1 /tmp`) so a real
//! machine is never touched, and `/bin/ls -1` specifically: the channel hands
//! the command a real terminal, so a bare `ls` resolves the operator's alias
//! and prints coloured, icon-decorated, multi-column output -- and an assertion
//! on the literal `total` never matches, because there is an escape sequence
//! between every letter. `tests/test_exec_live.py` documents the trap in full.
//!
//! `#[ignore]`d like every live suite, for the same reason and with the same
//! environment: `MULTITOP_TEST_SSH_HOST` (required -- there is no default, and
//! a loopback name is refused because multitop runs it locally and would not
//! cross ssh at all), `MULTITOP_TEST_SSH_USER`, `MULTITOP_TEST_SSH_PORT`.
//!
//! Run with `--test-threads=1`. The lock is per host, so two of these at once
//! contend for the thing under test.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

use std::time::Duration;

use multitop::app::Msg;
use multitop::tasks::spawn_upgrade;

use tokio::sync::mpsc;

#[path = "live_ssh/mod.rs"]
mod live_ssh;
use live_ssh::{ssh_server, target};

/// How long the run's messages may be silent before the collection is finished.
///
/// A ceiling, not a guess at how long an upgrade takes: these are real
/// subprocesses on a real host, and a window that closes early stops the
/// collection SILENTLY, so a truncated list goes on to fail some later
/// assertion about output that was never missing.
const QUIET: Duration = Duration::from_secs(60);

/// How long a probe may be asked "is it true yet?" before the test gives up.
///
/// A ceiling, not a guess -- the shape `event_loop_e2e::Harness::expect_dims`
/// carries the note for. The lock appears within milliseconds of a run starting
/// on a healthy host, and the generous ceiling is here so a slow round trip on
/// a loaded machine does not become a failure. Never reached on the pass path.
const PROBE_CEILING: Duration = Duration::from_secs(30);

async fn collect_messages(rx: mpsc::Receiver<Msg>) -> Vec<Msg> {
    let mut msgs = Vec::new();
    let mut rx = rx;
    while let Ok(Some(msg)) = tokio::time::timeout(QUIET, rx.recv()).await {
        msgs.push(msg);
    }
    msgs
}

/// The upgrade lock's path, relative to the user's home.
///
/// Derived from [`multitop_agent::exec::lock::default_path`] rather than
/// written out, so there is one definition of where the lock lives and this
/// test cannot drift from it. Only the components *after* the home directory
/// are kept, because the home differs per host: the remote shell expands these
/// against its own `$HOME`, and a test that hard-coded `/home/someone` would be
/// asking a question of the wrong machine.
fn remote_lock_path() -> String {
    let full = multitop_agent::exec::lock::default_path();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let relative = home
        .as_deref()
        .and_then(|h| full.strip_prefix(h).ok())
        .map_or_else(|| full.clone(), std::path::Path::to_path_buf);
    relative.to_string_lossy().into_owned()
}

/// Run a read-only shell snippet on the live host, over the same ssh the product
/// uses, and say whether it exited zero.
///
/// Resolved through `target`, so it is held to the same refusal rules the
/// upgrade path is: a loopback target answers a different question and is
/// refused rather than quietly probed. The agent's own `sh_quote` does the
/// quoting, for the reason the release script had to learn the hard way --
/// hand-written quoting of a path is how a probe ends up asking about a
/// filename that does not exist.
fn probe(script: &str) -> bool {
    let Ok(server) = target(
        std::env::var("MULTITOP_TEST_SSH_HOST").ok().as_deref(),
        &std::env::var("MULTITOP_TEST_SSH_USER")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "root".to_string()),
        std::env::var("MULTITOP_TEST_SSH_PORT").ok().as_deref(),
        "true",
    ) else {
        return false;
    };
    let sh = multitop::ssh_opts::sh_quote(script);
    std::process::Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "SendEnv=-*",
            "-T",
        ])
        .arg("-p")
        .arg(server.port.to_string())
        .arg(server.target().as_ref())
        .arg(format!("sh -c {sh}"))
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Wait until the remote host's upgrade lock is, or is not, held.
///
/// What the `sleep(500ms)` this replaces was guessing at, and it could not be
/// the right length: the lock is taken by the agent on the remote host, inside
/// a connection this process has not finished establishing, so a beat was the
/// only thing on offer. Half the time run 2 started first, took the lock, and
/// run 1 was the one refused -- which is a real contention and the opposite
/// attribution, and nothing in the test could tell.
///
/// It *is* observable, because the lock is a directory on the remote host at a
/// path the agent itself defines. Asking is one round trip and is true or
/// false; guessing is neither.
async fn wait_for_remote_lock(held: bool, what: &str) {
    let ask = format!(r#"test -d "$HOME/{}""#, remote_lock_path());
    let ceiling = tokio::time::timeout(PROBE_CEILING, async {
        // `test -d` on a missing directory is a NON-ZERO exit, so a false probe
        // is the answer "not held" rather than a failure to ask.
        while probe(&ask) != held {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        ceiling.is_ok(),
        "{what}: the lock on the remote host was still {} after {PROBE_CEILING:?}",
        if held { "absent" } else { "held" }
    );
}

/// The note the product reports when a run is refused the lock.
///
/// Matched on the words the verdict is built from, in the variant it actually
/// arrives in. The previous version read `AuxLine.line` for a string the
/// product does not put there -- and the string it wanted, "already in
/// progress", is wording from the quoted-shell lock wrappers that
/// `multitop_agent::exec::lock` replaced, so the match was dead. Matching the
/// stable part of the sentence rather than all of it means rewording the
/// recovery advice does not break this.
fn note_says_lock_held(msgs: &[Msg], panel: usize) -> bool {
    msgs.iter().any(|m| {
        matches!(
            m,
            Msg::AuxDone { panel: p, note: Some(n), success: false, .. }
                if *p == panel && n.contains("holds the lock")
        )
    })
}

/// The lines one panel's run printed.
///
/// The lock assertion's instrument rather than a general helper: "refused
/// means never ran" is a claim about *output*, and a run can report contention
/// and execute the command anyway. Only the note check would pass in that
/// case; this is what rules it out.
fn run_output(msgs: &[Msg], panel: usize) -> Vec<&str> {
    msgs.iter()
        .filter_map(|m| {
            if let Msg::AuxLine { panel: p, line, .. } = m {
                (*p == panel).then_some(line.as_str())
            } else {
                None
            }
        })
        .collect()
}

/// A second run against a host whose lock is held must be REFUSED, and must not
/// execute its command.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn test_remote_upgrade_lock_contention() {
    let server1 = ssh_server("sleep 5 && /bin/ls -1 /tmp");
    let server2 = ssh_server("/bin/ls -1 /tmp");

    // Run 1 holds the lock for the length of its command.
    let (tx, rx) = mpsc::channel::<Msg>(200);
    let h1 = spawn_upgrade(0, 1, server1, None, (80, 24), tx.clone());
    wait_for_remote_lock(true, "run 1 never took the lock").await;

    let h2 = spawn_upgrade(1, 2, server2, None, (80, 24), tx);
    let msgs = collect_messages(rx).await;
    let _ = h1.await;
    let _ = h2.await;

    // And it is gone again afterwards. A lock that outlives its run blocks
    // every later upgrade of that host for as long as the owner looks alive,
    // which is the whole failure the stale-break exists to prevent.
    wait_for_remote_lock(false, "the lock outlived its run").await;

    // Both runs finish and say so, on their own generations.
    for (panel, gen) in [(0, 1), (1, 2)] {
        assert!(
            msgs.iter().any(
                |m| matches!(m, Msg::AuxDone { panel: p, gen: g, .. } if *p == panel && *g == gen)
            ),
            "panel {panel} never reported finishing"
        );
    }

    // Run 1 ran.
    let out1 = run_output(&msgs, 0);
    assert!(
        out1.iter()
            .any(|l| l.contains("pty_dump") || l.contains("agent_script")),
        "run 1 was meant to execute `sleep 5 && /bin/ls -1 /tmp` and named no \
         file in /tmp -- so the ORACLE is wrong, not the product, which is the \
         distinction the live exec suite's docstring is entirely about:\n{out1:?}"
    );

    // Run 2 was told why, and produced nothing at all.
    assert!(
        note_says_lock_held(&msgs, 1),
        "run 2 was not told the lock was held. Notes: {:?}",
        msgs.iter()
            .filter_map(|m| match m {
                Msg::AuxDone { panel: 1, note, .. } => note.as_deref(),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    let out2 = run_output(&msgs, 1);
    assert!(
        out2.is_empty(),
        "a run refused the lock executed `/bin/ls -1 /tmp` anyway:\n{out2:?}"
    );
}
