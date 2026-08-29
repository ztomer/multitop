//! Children must not be able to reach multitop's controlling terminal.
//!
//! multitop holds the terminal in raw mode inside the alternate screen. A child
//! in the same process group is in the *foreground* group, so opening
//! `/dev/tty` succeeds: its prompt is drawn over the frame and its read takes
//! keystrokes out of the event loop's input. `ssh` does exactly that for an
//! unknown host key or a passphrase, whatever its stdin is connected to.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::config::Server;
use multitop::{password_store, ssh};

/// The process group of a live pid, via `ps` -- there is no safe way to ask
/// `getpgid` directly with `unsafe_code` denied for the workspace.
fn process_group_of(pid: u32) -> String {
    let out = std::process::Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .expect("ps must run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn local_server() -> Server {
    Server {
        host: "localhost".to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: Some("sleep 5".to_string()),
    }
}

/// The upgrade command is the child most likely to want a terminal: a local
/// `upgrade_cmd` containing `sudo` opens `/dev/tty` for its password prompt.
///
/// It has one of its own now -- the agent allocates a pty and makes it the
/// child's controlling terminal -- which makes this *more* important rather
/// than less. A child that both wants a terminal and could reach multitop's own
/// is exactly the shape that wrecked the display before; the process group is
/// what the kernel uses to refuse it.
#[tokio::test]
async fn a_locally_spawned_upgrade_gets_its_own_process_group() {
    let _guard = password_store::lock_for_test_async().await;
    // Also skips the on-disk upgrade lock, which is not what is under test.
    password_store::enable_mock_store();
    password_store::clear_mock_store();

    let request = multitop_agent::exec::ExecFrame::Request {
        command: "sleep 5".to_string(),
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
    };
    let mut child = ssh::spawn_exec(&local_server(), &request)
        .await
        .expect("spawn");
    let pid = child.id().expect("the child must still be running");

    let theirs = process_group_of(pid);
    let ours = process_group_of(std::process::id());

    assert!(!theirs.is_empty(), "ps reported no group for the child");
    assert_ne!(
        theirs, ours,
        "the upgrade shell shares multitop's process group, so it is in the \
         foreground group and can take over the terminal"
    );

    let _ = child.kill().await;
}

/// `ssh` reaches for `/dev/tty` on its own account -- an unknown host key, a
/// passphrase, a keyboard-interactive prompt. In multitop's terminal that
/// question is invisible and unanswerable, and the panel just says nothing.
#[test]
fn ssh_is_told_never_to_prompt() {
    assert!(
        ssh::SSH_OPTS
            .windows(2)
            .any(|pair| pair == ["-o", "BatchMode=yes"]),
        "SSH_OPTS must carry BatchMode=yes: {:?}",
        ssh::SSH_OPTS
    );
}
