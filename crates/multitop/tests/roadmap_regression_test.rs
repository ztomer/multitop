//! Comprehensive Integration & Regression Tests for Roadmap Features
//!
//! Validates:
//! 1. Upgrade Modal Workflow & key shortcuts (`u` -> modal -> `u` / confirm -> upgrade).
//! 2. Runtime State Persistence (`state.toml` creation, saving, loading).
//! 3. Single Sign-On (SSO) Master Password lifecycle & automatic fallback.
//! 4. Sparkline history updates for Memory (`M:`) and CPU (`C:`) in panel header.
//! 5. Consistent `user@host` display across panel titles.

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::password_store::{self, clear_mock_store, enable_mock_store};
use multitop::state::{self, AppState};

fn test_server(user: &str, host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: user.to_string(),
        upgrade_cmd: Some("echo upgrade".to_string()),
    }
}

#[test]
fn test_upgrade_modal_workflow_and_state_saving() {
    let temp_dir = std::env::temp_dir().join("multitop_test_modal_flow");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let server = test_server("admin", "srv01.net");
    let mut app = App::new(vec![server]);
    app.config_path = Some(config_path.clone());

    // Initially modal is closed
    assert!(!app.show_upgrade_modal);
    assert_eq!(app.last_update, None);

    // Open modal
    app.show_upgrade_modal = true;
    assert!(app.show_upgrade_modal);

    // Confirm upgrade
    let cmds = app.confirm_upgrade();

    // Modal is now closed and commands generated
    assert!(!app.show_upgrade_modal);
    assert_eq!(cmds.len(), 1);

    // Simulate upgrade completing so last_update gets persisted
    let gen = match &cmds[0] {
        multitop::app::Command::RunUpgrade { gen, .. } => *gen,
        _ => unreachable!(),
    };
    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: None,
        success: true,
    });
    assert!(app.last_update.is_some());

    // Verify state.toml persisted
    let state = state::load_state(&config_path);
    assert_eq!(state.last_update, app.last_update);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sso_master_password_lifecycle_and_fallback() {
    enable_mock_store();
    clear_mock_store();
    password_store::clear_sso_cache();

    let server1 = test_server("user1", "host1.org");
    let server2 = test_server("user2", "host2.org");

    // Save SSO Master Password
    password_store::save_sso("sso_master_secret_456").unwrap();
    assert_eq!(
        password_store::load_sso().unwrap().as_deref(),
        Some("sso_master_secret_456")
    );

    // Server with no explicit password falls back to SSO password
    assert_eq!(
        password_store::load(&server1).unwrap().as_deref(),
        Some("sso_master_secret_456")
    );
    assert_eq!(
        password_store::load(&server2).unwrap().as_deref(),
        Some("sso_master_secret_456")
    );

    // Explicit server password overrides SSO
    password_store::save(&server1, "override_pass_789").unwrap();
    assert_eq!(
        password_store::load(&server1).unwrap().as_deref(),
        Some("override_pass_789")
    );
    assert_eq!(
        password_store::load(&server2).unwrap().as_deref(),
        Some("sso_master_secret_456")
    );

    // Cleanup SSO
    password_store::delete_sso().unwrap();
    assert_eq!(password_store::load_sso().unwrap(), None);
    assert_eq!(
        password_store::load(&server2).unwrap(),
        None
    );
}

#[test]
fn test_state_persistence_roundtrip() {
    let temp_dir = std::env::temp_dir().join("multitop_test_state_roundtrip");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let initial = AppState {
        last_update: Some(1722000000),
        upgrade_started_at: None,
    };

    state::save_state(&config_path, &initial).expect("save state");
    let loaded = state::load_state(&config_path);

    assert_eq!(loaded, initial);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sparklines_and_header_formatting() {
    let server = test_server("deployer", "prod-node-1");
    let mut app = App::new(vec![server]);

    // Push telemetry sample
    let snap = multitop_agent::render::Snapshot {
        host: "prod-node-1".to_string(),
        cpu_pct: 75.0,
        mem: multitop_agent::proc::Usage::new(10, 8),
        ..Default::default()
    };

    app.apply(Msg::Packet {
        panel: 0,
        gen: 0,
        payload: multitop_agent::proto::Payload::Monitor(snap),
        dims: (80, 24),
    });

    assert!(!app.sparklines_cpu[0].render_bar().is_empty());
    assert!(!app.sparklines_mem[0].render_bar().is_empty());

    // Verify panel server target includes user@host
    assert_eq!(app.panels[0].server.target(), "deployer@prod-node-1");
}

#[test]
fn test_username_consistency_across_panes() {
    let server_with_user = test_server("alice", "db-host");
    let server_no_user = Server {
        host: "bare-host".to_string(),
        port: 22,
        user: "".to_string(),
        upgrade_cmd: None,
    };

    assert_eq!(server_with_user.target(), "alice@db-host");
    assert_eq!(server_no_user.target(), "bare-host");
}
