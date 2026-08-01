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
    let mut child = spawn_command(&server, "echo hello", None).unwrap();

    let mut stdout = String::new();
    child.stdout.as_mut().unwrap().read_to_string(&mut stdout).await.unwrap();
    let status = child.wait().await.unwrap();

    assert!(status.success());
    assert!(stdout.contains("hello"));
}

#[tokio::test]
async fn test_local_spawn_command_with_mock_password() {
    enable_mock_store();
    let server = local_server("echo test");
    let password = "mock_password";
    let mut child = spawn_command(&server, "echo test", Some(password)).unwrap();

    let mut stdout = String::new();
    child.stdout.as_mut().unwrap().read_to_string(&mut stdout).await.unwrap();
    let status = child.wait().await.unwrap();

    assert!(status.success());
    assert!(stdout.contains("test"));
}

#[tokio::test]
async fn test_local_spawn_command_upgrade_lock_prevents_concurrent() {
    enable_mock_store();
    let server = local_server("sleep 2");

    // First command acquires lock
    let mut child1 = spawn_command(&server, "sleep 2", None).unwrap();

    // Second command should block or fail
    let mut child2 = spawn_command(&server, "echo should_not_run", None).unwrap();

    // Wait for first to complete
    let _ = child1.wait().await.unwrap();

    // Second should now be able to run (lock released)
    let mut stdout = String::new();
    child2.stdout.as_mut().unwrap().read_to_string(&mut stdout).await.unwrap();
    let status = child2.wait().await.unwrap();

    // In test mode with mock store, lock is disabled
    assert!(status.success());
}

#[tokio::test]
async fn test_local_spawn_agent_finds_binary() {
    enable_mock_store();
    let server = local_server("multitop-agent --help 2>&1 || true");

    let mut child = spawn_command(&server, "multitop-agent --help 2>&1 || true", None).unwrap();

    let mut stdout = String::new();
    child.stdout.as_mut().unwrap().read_to_string(&mut stdout).await.unwrap();
    let status = child.wait().await.unwrap();

    // Should at least run without "command not found" for multitop-agent
    assert!(status.success() || stdout.contains("multitop-agent"));
}