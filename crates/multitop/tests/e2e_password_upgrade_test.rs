//! Automated E2E Integration Tests for Password Persistence & Upgrade Execution Flow
//!
//! Validates:
//! 1. Setting and deleting passwords via PasswordManager, persisting to OS credential store.
//! 2. Automatic loading of stored passwords during App/Panel initialization.
//! 3. Execution of upgrade tasks (spawn_upgrade) using stored passwords to stream command output.
//! 4. In-stream guidance tip generation when sudo authentication is missing.

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::passwords::{self, ConfigSection, PasswordAction};
use multitop::tasks::spawn_upgrade;

fn test_server(host: &str, upgrade_cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: upgrade_cmd.map(String::from),
    }
}

#[tokio::test]
async fn test_e2e_password_storage_and_os_keyring_lifecycle() {
    let server = test_server("localhost", Some("echo 'hello'"));
    let mut app = App::new(vec![server.clone()]);

    // Open passwords section
    passwords::open(&mut app, 0, false);
    assert!(app.password_manager.is_some());
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );

    // Save a password
    let save_action = PasswordAction::Save {
        panel: 0,
        password: "e2e_test_secret_123".to_string(),
        resume_upgrade: false,
    };

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(save_action, &mut app, std::slice::from_ref(&server), &tx, &mut tasks);

    // Verify in-memory panel state
    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("e2e_test_secret_123")
    );
    assert!(app.panels[0].password_saved);

    // Verify OS Credential Store persistence
    let loaded = multitop::password_store::load(&server).expect("keyring load");
    assert_eq!(loaded.as_deref(), Some("e2e_test_secret_123"));

    // Delete password
    let delete_action = PasswordAction::Delete { panel: 0 };
    multitop::password_actions::apply(delete_action, &mut app, std::slice::from_ref(&server), &tx, &mut tasks);

    assert_eq!(app.panels[0].sudo_password, None);
    assert!(!app.panels[0].password_saved);

    // Verify OS Credential Store cleanup
    let reloaded = multitop::password_store::load(&server).expect("keyring load after delete");
    assert_eq!(reloaded, None);
}

#[tokio::test]
async fn test_e2e_app_initialization_auto_loads_stored_password() {
    let server = test_server("127.0.0.1", Some("echo 'test'"));

    // Pre-seed OS credential store
    let _ = multitop::password_store::save(&server, "preseeded_password");

    // Initialize App
    let app = App::new(vec![server.clone()]);
    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("preseeded_password")
    );
    assert!(app.panels[0].password_saved);

    // Cleanup credential store
    let _ = multitop::password_store::delete(&server);
}

#[tokio::test]
async fn test_e2e_spawn_upgrade_streams_output_with_stored_password() {
    let server = test_server("127.0.0.1", Some("echo 'E2E_UPGRADE_STREAM_OK'"));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Msg>(100);

    // Spawn upgrade task with password
    let handle = spawn_upgrade(
        0,
        1,
        server,
        Some("e2e_dummy_sudo_pass".to_string()),
        tx,
    );

    let mut begin_received = false;
    let mut stream_line_received = false;
    let mut done_received = false;

    while let Ok(msg) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
        let Some(msg) = msg else { break };
        match msg {
            Msg::AuxBegin { panel: 0, gen: 1, header } => {
                begin_received = true;
                assert!(header.unwrap_or_default().contains("Upgrade on 127.0.0.1"));
            }
            Msg::AuxLine { panel: 0, gen: 1, line } => {
                if line.contains("E2E_UPGRADE_STREAM_OK") {
                    stream_line_received = true;
                }
            }
            Msg::AuxDone { panel: 0, gen: 1, note } => {
                done_received = true;
                assert!(note.unwrap_or_default().contains("done"));
                break;
            }
            _ => {}
        }
    }

    let _ = handle.await;

    assert!(begin_received, "AuxBegin stream message expected");
    assert!(stream_line_received, "E2E_UPGRADE_STREAM_OK line expected in stream");
    assert!(done_received, "AuxDone stream message expected");
}

#[tokio::test]
async fn test_e2e_spawn_upgrade_emits_in_stream_tip_on_sudo_failure() {
    // Sudo prompt output simulation
    let server = test_server("127.0.0.1", Some("echo 'sudo: a terminal is required to authenticate'"));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Msg>(100);

    let handle = spawn_upgrade(0, 2, server, None, tx);

    let mut tip_received = false;

    while let Ok(msg) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
        let Some(msg) = msg else { break };
        if let Msg::AuxLine { line, .. } = msg {
            if line.contains("Set password in settings ('e')") {
                tip_received = true;
                break;
            }
        }
    }

    let _ = handle.await;

    assert!(tip_received, "In-stream guidance tip expected when sudo authentication is missing");
}
