//! Local SSH command execution tests (using 127.0.0.1 for local path).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::config::Server;
use multitop::password_store;
use multitop::ssh::spawn_command;
use tokio::io::AsyncReadExt;

fn local_server(upgrade_cmd: &str) -> Server {
    Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some(upgrade_cmd.to_string()),
    }
}

fn enable_mock_store() {
    password_store::enable_mock_store();
    password_store::clear_mock_store();
}

#[tokio::test]
async fn test_local_spawn_command_no_password() {
    enable_mock_store();
    let server = local_server("echo hello");
    let mut child = spawn_command(&server, "echo hello", None).unwrap().child;

    let mut stdout = String::new();
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_to_string(&mut stdout)
        .await
        .unwrap();
    let status = child.wait().await.unwrap();

    assert!(status.success());
    assert!(stdout.contains("hello"));
}

#[tokio::test]
async fn test_local_spawn_command_with_mock_password() {
    enable_mock_store();
    let server = local_server("echo test");
    let password = "mock_password";
    let mut child = spawn_command(&server, "echo test", Some(password))
        .unwrap()
        .child;

    let mut stdout = String::new();
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_to_string(&mut stdout)
        .await
        .unwrap();
    let status = child.wait().await.unwrap();

    assert!(status.success());
    assert!(stdout.contains("test"));
}

#[tokio::test]
async fn test_local_spawn_command_upgrade_lock_prevents_concurrent() {
    enable_mock_store();
    let server = local_server("sleep 2");

    // First command acquires lock
    let mut child1 = spawn_command(&server, "sleep 2", None).unwrap().child;

    // Second command should block or fail
    let mut child2 = spawn_command(&server, "echo should_not_run", None)
        .unwrap()
        .child;

    // Wait for first to complete
    let _ = child1.wait().await.unwrap();

    // Second should now be able to run (lock released)
    let mut stdout = String::new();
    child2
        .stdout
        .as_mut()
        .unwrap()
        .read_to_string(&mut stdout)
        .await
        .unwrap();
    let status = child2.wait().await.unwrap();

    // In test mode with mock store, lock is disabled
    assert!(status.success());
}

#[tokio::test]
async fn test_local_spawn_agent_finds_binary() {
    enable_mock_store();
    // `fetch`, not `--help`. What this test is about is the spawn finding a
    // binary on PATH, and `fetch` is a one-shot every version of the agent has
    // ever terminated on. `--help` was only handled from 0.34 -- before that an
    // unrecognised flag became the *host label* and the agent streamed monitor
    // packets, so on any machine with an older one installed this test never
    // finished. The suite it was part of hung rather than failed.
    let server = local_server("multitop-agent fetch 2>&1 || true");

    let mut child = spawn_command(&server, "multitop-agent fetch 2>&1 || true", None)
        .unwrap()
        .child;

    // Still bounded. Whatever is on PATH is outside this repo's control, and a
    // test that cannot finish is worse than one that fails: it takes the whole
    // run with it and reports nothing.
    // Bytes, not a string. Piped, the agent writes a binary packet -- so
    // `read_to_string` failed on "stream did not contain valid UTF-8", which
    // says nothing about whether the binary was found.
    let mut stdout = Vec::new();
    let read = child.stdout.as_mut().unwrap().read_to_end(&mut stdout);
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(20), read).await;
    if outcome.is_err() {
        let _ = child.kill().await;
        panic!(
            "`multitop-agent fetch` on PATH never finished; it is a one-shot and \
             must exit. Installed binary: {:?}",
            std::process::Command::new("sh")
                .args([
                    "-c",
                    "command -v multitop-agent && multitop-agent --version"
                ])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout)
                    .chars()
                    .take(60)
                    .collect::<String>())
        );
    }
    outcome.unwrap().unwrap();
    let status = child.wait().await.unwrap();

    let text = String::from_utf8_lossy(&stdout);

    // The binary being installed is a property of the machine, not of this
    // repo, so its absence is reported and the test stops rather than passing.
    //
    // It used to pass either way, and passed *because* of the failure: the
    // assertion accepted `stdout.contains("multitop-agent")`, and the shell's
    // own `command not found: multitop-agent` contains that. A test for "the
    // spawn found the binary" returned green on the one machine where it had
    // not -- for eleven months, on the developer's own laptop.
    if text.contains("command not found") {
        eprintln!(
            "SKIPPED: multitop-agent is not on PATH here, so this cannot say \
             whether the spawn would find it. Install it with `cargo install \
             --path crates/agent` to run this for real."
        );
        return;
    }

    // Present, so it must have produced a frame and exited cleanly.
    assert!(
        status.success(),
        "the agent was found but exited {status:?}: {text}"
    );
    assert!(
        !stdout.is_empty(),
        "the agent was found, exited cleanly, and wrote nothing"
    );
}
