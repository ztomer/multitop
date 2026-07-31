//! Task spawning integration tests.

use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::tasks::spawn_upgrade;
use multitop::password_store;
use multitop::state;
use multitop_agent::SortBy;
use tokio::sync::mpsc;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some("echo test".to_string()),
    }
}

fn enable_mock_store() {
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    password_store::delete_sso().unwrap();
}

#[tokio::test]
async fn test_spawn_upgrade_generation_tracking() {
    enable_mock_store();
    let server = test_server("127.0.0.1");
    let (tx, mut rx) = mpsc::channel::<Msg>(100);

    let handle = spawn_upgrade(0, 42, server.clone(), None, tx.clone());

    let msg = rx.recv().await.unwrap();
    match msg {
        Msg::AuxBegin { panel: 0, gen: 42, .. } => {}
        other => panic!("Expected AuxBegin with gen=42, got {:?}", other),
    }

    let _ = handle.await;
}

#[tokio::test]
async fn test_spawn_upgrade_sets_mode_and_state() {
    enable_mock_store();
    let mut app = App::new(vec![test_server("127.0.0.1")]);
    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(1);

    // Use password_actions::apply to trigger upgrade through the normal path
    let servers = vec![test_server("127.0.0.1")];
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    // Panel should be in Upgrade mode with STARTED state
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(app.panels[0].upgrade_gen, 1);
}

#[tokio::test]
async fn test_spawn_upgrade_saves_state_file() {
    enable_mock_store();
    let mut app = App::new(vec![test_server("127.0.0.1")]);
    let tmp_path = std::env::temp_dir().join(format!("multitop_test_state_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(1);
    let servers = vec![test_server("127.0.0.1")];

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    // State file should be saved with upgrade_started_at
    let state_obj = state::load_state(&tmp_path);
    assert!(state_obj.upgrade_started_at.is_some());

    let _ = std::fs::remove_file(tmp_path);
}

#[tokio::test]
async fn test_task_cancellation_on_panel_switch() {
    enable_mock_store();
    let mut app = App::new(vec![test_server("127.0.0.1"), test_server("127.0.0.2")]);
    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(2);

    let servers = vec![test_server("127.0.0.1"), test_server("127.0.0.2")];

    // Start upgrade on panel 0
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    assert!(tasks.aux[0].is_some());
    assert!(tasks.aux_is_upgrade[0]);

    // Start upgrade on panel 1 - should NOT cancel panel 0's task
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 1,
            password: "test_pass2".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    // Panel 0's task should still be running
    assert!(tasks.aux[0].is_some());

    // Panel 1 should have new task
    assert!(tasks.aux[1].is_some());
    assert!(tasks.aux_is_upgrade[1]);

    // Now start another upgrade on panel 0 - this SHOULD cancel panel 0's old task
    let handle0 = tasks.aux[0].take().expect("task should exist");

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass3".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    // Panel 0's old task should have been replaced (completed or aborted)
    let _result = handle0.await;
    // Panel 0 should have new task
    assert!(tasks.aux[0].is_some());

    // Panel 0 should have new task
    assert!(tasks.aux[0].is_some());
}

#[tokio::test]
async fn test_concurrent_upgrade_generations_isolated() {
    enable_mock_store();
    let server = test_server("127.0.0.1");
    let (tx, mut rx) = mpsc::channel::<Msg>(100);

    // Spawn upgrade with gen=1
    let handle1 = spawn_upgrade(0, 1, server.clone(), None, tx.clone());

    // Spawn upgrade with gen=2 (simulates panel switch)
    let handle2 = spawn_upgrade(0, 2, server.clone(), None, tx.clone());

    let mut gen1_done = false;
    let mut gen2_done = false;

    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::AuxDone { gen, success, .. } if gen == 1 && success => gen1_done = true,
            Msg::AuxDone { gen, success, .. } if gen == 2 && success => gen2_done = true,
            _ => {}
        }
        if gen1_done && gen2_done {
            break;
        }
    }

    let _ = handle1.await;
    let _ = handle2.await;

    assert!(gen1_done);
    assert!(gen2_done);
}