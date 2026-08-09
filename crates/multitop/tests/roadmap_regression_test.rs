//! Comprehensive Integration & Regression Tests for Roadmap Features
//!
//! Validates:
//! 1. Upgrade Modal Workflow & key shortcuts (`u` -> modal -> `u` / confirm -> upgrade).
//! 2. Runtime State Persistence (`state.toml` creation, saving, loading).
//! 3. Single Sign-On (SSO) Master Password lifecycle & automatic fallback.
//! 5. Consistent `user@host` display across panel titles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::state::{self, AppState};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// An integration binary is compiled without `cfg(test)`, so the mock store is
/// not in force unless it is asked for, and anything holding an `App` reaches
/// `password_store` several calls down. Without this these tests query the real
/// OS keychain: every rebuild changes the binary's code signature, so macOS
/// raises an access dialog and the suite stops until a human dismisses it.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

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
    let _keychain = isolate_keychain();
    let temp_dir = std::env::temp_dir().join("multitop_test_modal_flow");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let server = test_server("admin", "srv01.net");
    let mut app = App::new(vec![server]);
    app.config_path = Some(config_path.clone());

    // Initially modal is closed
    assert!(!app.show_upgrade_modal());
    assert_eq!(app.last_update, None);

    // Open modal
    app.set_show_upgrade_modal(true);
    assert!(app.show_upgrade_modal());

    // Confirm upgrade
    let cmds = app.confirm_upgrade();

    // Modal is now closed and commands generated
    assert!(!app.show_upgrade_modal());
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
    assert_eq!(state.state.last_update, app.last_update);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_state_persistence_roundtrip() {
    let _keychain = isolate_keychain();
    let temp_dir = std::env::temp_dir().join("multitop_test_state_roundtrip");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let initial = AppState {
        last_update: Some(1_722_000_000),
        upgrade_started_at: None,
        hosts: std::collections::BTreeMap::new(),
    };

    state::save_state(&config_path, &initial).expect("save state");
    let loaded = state::load_state(&config_path);

    assert_eq!(loaded.state, initial);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_header_formatting() {
    let _keychain = isolate_keychain();
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
        epoch: app.panels_epoch,
        payload: multitop_agent::proto::Payload::Monitor(snap),
        dims: (80, 24),
    });

    // Verify panel server target includes user@host
    assert_eq!(app.panels[0].server.target(), "deployer@prod-node-1");
}

#[test]
fn test_username_consistency_across_panes() {
    let _keychain = isolate_keychain();
    let server_with_user = test_server("alice", "db-host");
    let server_no_user = Server {
        host: "bare-host".to_string(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
    };

    assert_eq!(server_with_user.target(), "alice@db-host");
    assert_eq!(server_no_user.target(), "bare-host");
}

/// A monitor packet from the previous panel list must not paint the panel that
/// moved into its slot.
///
/// This is the defect `replace_panels` bumps the epoch to prevent, and its own
/// doc comment says so: "after a deletion the task for the removed host paints
/// the panel that moved into its slot -- one machine's statistics under another
/// machine's name." The guard was there and the arm that carries the statistics
/// never consulted it. Monitor tasks are long-lived and stamp every packet
/// `gen: 0`, so a `gen` check would have rejected live stats forever once an
/// edit moved the generations off zero -- which is why that arm checked
/// nothing. The packet carries the epoch now, and every arm is guarded by it.
#[test]
fn a_packet_from_the_old_panel_list_cannot_paint_the_new_one() {
    let _keychain = isolate_keychain();
    let a = Server {
        host: "alpha".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: None,
    };
    let b = Server {
        host: "beta".to_string(),
        ..a.clone()
    };
    let mut app = App::new(vec![a, b.clone()]);
    let stale_epoch = app.panels_epoch;

    // The user removes alpha; beta moves into slot 0.
    app.replace_panels(vec![b]);
    assert_eq!(app.panels[0].server.host, "beta");

    // A packet that alpha's monitor task had already put on the wire, naming
    // slot 0 and stamped with the panel list it was started for.
    let snap = multitop_agent::render::Snapshot {
        host: "alpha".to_string(),
        cpu_pct: 99.0,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        ..Default::default()
    };
    let changed = app.apply(Msg::Packet {
        panel: 0,
        gen: 0,
        epoch: stale_epoch,
        payload: multitop_agent::proto::Payload::Monitor(snap),
        dims: (80, 24),
    });

    assert!(
        !changed,
        "a packet for a retired panel list changes nothing"
    );
    assert!(
        app.panels[0].last_monitor.is_none(),
        "and must not be stored: beta's pane would then be showing alpha's \
         statistics under beta's name"
    );
}
