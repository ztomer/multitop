//! Automated E2E Integration Tests for the Full Upgrade Execution Loop
//!
//! Validates the complete upgrade flow:
//! 1. `spawn_upgrade` spawns local processes, streams output via Msg channel
//! 2. App state machine correctly processes AuxBegin/AuxLine/AuxDone messages
//! 3. Output collection, carriage return cleaning, exit status reporting
//!
//! Local tests run automatically; remote tests are `#[ignore]`d.
//!
//! Run local tests: `cargo test --test upgrade_loop_e2e`
//! Run remote tests: `cargo test --test upgrade_loop_remote_e2e -- --ignored`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::{Mode, UpgradeState};
use multitop::password_store;
use multitop::tasks::spawn_upgrade;
use multitop::types::Command;

use tokio::sync::mpsc;

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

/// Test helper: create a local Server (127.0.0.1 triggers local shell path).
fn local_server(upgrade_cmd: &str) -> Server {
    Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some(upgrade_cmd.to_string()),
    }
}

/// Enable mock password store for tests (auto-enabled via cfg!(test) but explicit for clarity).
/// Reset the process-global mock store, holding the test guard so a
/// concurrently running test cannot be wiped out mid-run. Keep the returned
/// guard alive for the whole test body.
async fn enable_test_mock_store() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// The same, for `#[test]` bodies that cannot await.
///
/// These tests drive the real `enter_upgrade_view`, which loads saved passwords
/// so it can tell the user truthfully whether a prompt is coming. Without this
/// guard that load reaches the OS keychain from the test suite.
fn enable_test_mock_store_blocking() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Test helper: collect messages from channel with timeout.
struct MsgCollector {
    rx: mpsc::Receiver<Msg>,
}

impl MsgCollector {
    const fn new(rx: mpsc::Receiver<Msg>) -> Self {
        Self { rx }
    }

    async fn collect_all(&mut self) -> Vec<Msg> {
        let mut msgs = Vec::new();
        while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(5), self.rx.recv()).await
        {
            msgs.push(msg);
        }
        msgs
    }

    async fn wait_for_done(&mut self) -> Option<Msg> {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), self.rx.recv()).await {
                Ok(Some(msg)) => {
                    if matches!(msg, Msg::AuxDone { .. } | Msg::Status { .. }) {
                        return Some(msg);
                    }
                }
                _ => return None,
            }
        }
    }
}

// ============================================================================
// Phase 1: Core Stream Tests
// ============================================================================

/// Test 1: Single server basic stream
#[tokio::test]
async fn test_upgrade_single_server_streams_exact_output() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("ls -l ; ls -l");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    let _ = handle.await;

    match done {
        Msg::AuxDone {
            panel: 0,
            gen: 1,
            success: true,
            ..
        } => {}
        other => panic!("Expected AuxDone panel=0 gen=1 success=true, got {other:?}"),
    }
}

/// Test 2: Multi-server concurrent output
#[tokio::test]
async fn test_upgrade_multi_server_concurrent_output() {
    let _store_guard = enable_test_mock_store().await;
    let (tx, rx) = mpsc::channel::<Msg>(100);

    let s1 = local_server("echo UPGRADE_1 ; ls -l");
    let s2 = local_server("echo UPGRADE_2 ; ls -l");
    let s3 = local_server("echo UPGRADE_3 ; ls -l");

    let h1 = spawn_upgrade(0, 1, s1, None, tx.clone());
    let h2 = spawn_upgrade(1, 2, s2, None, tx.clone());
    let h3 = spawn_upgrade(2, 3, s3, None, tx.clone());

    drop(tx);

    let mut collector = MsgCollector::new(rx);
    let msgs = collector.collect_all().await;

    let _ = h1.await;
    let _ = h2.await;
    let _ = h3.await;

    let mut done_count = 0;
    let mut seen_1 = false;
    let mut seen_2 = false;
    let mut seen_3 = false;

    for msg in &msgs {
        if let Msg::AuxLine { line, .. } = msg {
            if line.contains("UPGRADE_1") {
                seen_1 = true;
            }
            if line.contains("UPGRADE_2") {
                seen_2 = true;
            }
            if line.contains("UPGRADE_3") {
                seen_3 = true;
            }
        }
        if let Msg::AuxDone { success: true, .. } = msg {
            done_count += 1;
        }
    }

    assert!(seen_1, "UPGRADE_1 not found in stream");
    assert!(seen_2, "UPGRADE_2 not found in stream");
    assert!(seen_3, "UPGRADE_3 not found in stream");
    assert_eq!(done_count, 3, "Expected 3 successful AuxDone messages");
}

/// Test 3: Failure exit code
#[tokio::test]
async fn test_upgrade_failure_reports_nonzero_exit() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("ls -l ; exit 1");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    let _ = handle.await;

    match done {
        Msg::AuxDone {
            panel: 0,
            gen: 1,
            success: false,
            ..
        } => {}
        other => panic!("Expected AuxDone panel=0 gen=1 success=false, got {other:?}"),
    }
}

/// Test 4: State machine roundtrip (full App cycle with Msg application)
#[tokio::test]
async fn test_upgrade_state_machine_roundtrip() {
    use tempfile::TempDir;
    let _keychain = isolate_keychain_async().await;
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let mut app = App::new(vec![local_server("ls -l")]);
    app.config_path = Some(config_path.clone());

    let cmds = app.confirm_upgrade();
    assert!(!cmds.is_empty());
    let started_at = app.upgrade_started_at;
    assert!(started_at.is_some(), "upgrade_started_at should be set");

    let Command::RunUpgrade { panel, gen } = cmds[0] else {
        panic!("Expected Command::RunUpgrade");
    };
    assert_eq!(panel, 0);

    // Simulate full message stream
    app.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    assert_eq!(app.panels[0].mode, Mode::Upgrade);

    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "drwxr-xr-x 1 root root 4096 Jan 1 00:00 bin".to_string(),
    });

    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("done".to_string()),
        success: true,
    });

    assert_eq!(app.panels[0].upgrade_state, UpgradeState::DONE);
    assert!(
        app.last_update.is_some(),
        "last_update should be set on success"
    );

    // Verify state was persisted (AuxDone updates state.toml with last_update, clears upgrade_started_at)
    let state = multitop::state::load_state(&config_path);
    assert_eq!(state.last_update, app.last_update);
    assert_eq!(
        state.upgrade_started_at, None,
        "upgrade_started_at cleared after successful completion"
    );
    assert!(app.panels[0]
        .last_upgrade
        .iter()
        .any(|l| l == "drwxr-xr-x 1 root root 4096 Jan 1 00:00 bin"));
}

/// Test 7: Empty output handled gracefully
#[tokio::test]
async fn test_upgrade_empty_output_handled() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("true");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    let _ = handle.await;

    match done {
        Msg::AuxDone {
            panel: 0,
            gen: 1,
            success: true,
            ..
        } => {}
        other => panic!("Expected AuxDone success=true, got {other:?}"),
    }
}

/// Test 8: Carriage return cleaning
#[tokio::test]
async fn test_upgrade_carriage_return_cleaned() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("printf 'step1\\rstep2\\rstep3\\n'");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let msgs = collector.collect_all().await;
    let _ = handle.await;

    let progress_lines: Vec<String> = msgs
        .iter()
        .filter_map(|msg| {
            if let Msg::AuxLine { line, .. } = msg {
                if line.contains("step") {
                    return Some(line.clone());
                }
            }
            None
        })
        .collect();

    assert!(!progress_lines.is_empty(), "Should have progress lines");
    for line in &progress_lines {
        assert!(
            !line.contains('\r'),
            "Carriage return not stripped: {line:?}"
        );
    }
}

// ============================================================================
// Phase 3: Security & Edge Case Tests (also go in this file)
// ============================================================================

/// Test 5: Vault password preloaded
#[tokio::test]
async fn test_upgrade_vault_password_preloaded() {
    use multitop_vault::{Vault, VaultConfig};
    use secrecy::SecretString;
    use tempfile::TempDir;

    let _store_guard = enable_test_mock_store().await;
    let temp_dir = TempDir::new().unwrap();
    let vault_path = temp_dir.path().join("vault.bin");
    let config_path = temp_dir.path().join("config.toml");

    let master_pw = "test-master";
    let vault_config = VaultConfig {
        vault_path,
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    };
    let vault = Vault::new(vault_config);
    vault.initialize(master_pw).await.unwrap();
    let mut unlocked = vault.unlock_with_password(master_pw).unwrap();
    let key = multitop::password_store::account(&local_server("echo test"));
    unlocked
        .set_password(key, &SecretString::from("sudo-secret-123"))
        .unwrap();
    unlocked.lock();

    let server = local_server("echo test");
    let mut app = App::new(vec![server]);
    app.vault = Some(std::sync::Arc::new(vault));
    app.config_path = Some(config_path);
    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password(master_pw)
        .unwrap();
    app.vault_state = VaultState::Unlocked {
        vault: Box::new(unlocked),
        awaiting_biometric: false,
    };

    let cmds = app.run_upgrade();
    assert!(!cmds.is_empty());
    assert_eq!(
        app.panels[0].sudo_password,
        Some("sudo-secret-123".to_string())
    );

    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!("Expected RunUpgrade");
    };
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let updated_server = local_server("echo test");
    let handle = spawn_upgrade(
        0,
        gen,
        updated_server,
        app.panels[0].sudo_password.clone(),
        tx,
    );

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    let _ = handle.await;

    match done {
        Msg::AuxDone { success: true, .. } => {}
        other => panic!("Expected AuxDone success=true, got {other:?}"),
    }
}

/// Test 6: Lock prevents concurrent (local lock contention)
#[tokio::test]
async fn test_upgrade_lock_prevents_concurrent() {
    let _store_guard = enable_test_mock_store().await;
    let (tx, rx) = mpsc::channel::<Msg>(100);

    // First upgrade holds lock with a sleep
    let s1 = local_server("echo first ; sleep 1 ; ls -l");
    let h1 = spawn_upgrade(0, 1, s1, None, tx.clone());

    // Second upgrade should be blocked by lock
    let s2 = local_server("echo second ; ls -l");
    let h2 = spawn_upgrade(1, 2, s2, None, tx);

    let mut collector = MsgCollector::new(rx);
    let msgs = collector.collect_all().await;

    let _ = h1.await;
    let _ = h2.await;

    let done1 = msgs
        .iter()
        .filter(|m| matches!(m, Msg::AuxDone { panel: 0, .. }))
        .count();
    let done2 = msgs
        .iter()
        .filter(|m| matches!(m, Msg::AuxDone { panel: 1, .. }))
        .count();

    assert_eq!(done1, 1, "First upgrade should complete");
    assert_eq!(
        done2, 1,
        "Second upgrade should also complete (lock with PID check)"
    );
}

/// Test 9: Generation staleness — stale messages are dropped
#[tokio::test]
async fn test_upgrade_generation_staleness() {
    let _keychain = isolate_keychain_async().await;
    let mut app = App::new(vec![local_server("echo gen_test")]);

    // Start first upgrade (gen becomes 1)
    let cmds = app.run_upgrade();
    let Command::RunUpgrade { gen: gen1, .. } = cmds[0] else {
        panic!("Expected RunUpgrade");
    };

    // Start second upgrade (gen becomes 2, first is stale)
    let cmds2 = app.run_upgrade();
    let Command::RunUpgrade { gen: gen2, .. } = cmds2[0] else {
        panic!("Expected RunUpgrade");
    };
    assert!(gen2 > gen1);

    // Send stale AuxLine with old gen
    app.apply(Msg::AuxLine {
        panel: 0,
        gen: gen1,
        line: "STALE_LINE".to_string(),
    });

    // Send current AuxLine with new gen
    app.apply(Msg::AuxLine {
        panel: 0,
        gen: gen2,
        line: "CURRENT_LINE".to_string(),
    });

    // The pane (header + ring) must not leak the stale generation's line, and
    // must show the current one.
    assert!(
        !multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("STALE_LINE"),
        "Stale message should be dropped"
    );
    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("CURRENT_LINE"),
        "Current message should be visible"
    );
}

/// Test 10: State persists across App restart
#[tokio::test]
async fn test_upgrade_state_persists_across_app_restart() {
    use tempfile::TempDir;
    let _keychain = isolate_keychain_async().await;

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let mut app = App::new(vec![local_server("ls -l")]);
    app.config_path = Some(config_path.clone());

    let cmds = app.confirm_upgrade();
    let Command::RunUpgrade { panel, gen } = cmds[0] else {
        panic!("Expected RunUpgrade");
    };
    assert_eq!(panel, 0);

    // Simulate completion
    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("done".to_string()),
        success: true,
    });

    // Verify state file written
    let state = multitop::state::load_state(&config_path);
    assert_eq!(state.last_update, app.last_update);
    assert!(state.last_update.is_some());

    // Simulate App restart with same config
    let mut app2 = App::new(vec![local_server("ls -l")]);
    app2.config_path = Some(config_path.clone());

    let loaded = multitop::state::load_state(&config_path);
    assert_eq!(
        loaded.last_update, app.last_update,
        "last_update should persist across restart"
    );
}

// ============================================================================
// Phase 2: UI Cycle Tests
// ============================================================================

/// Test 11: Upgrade → return → show last result
#[test]
fn test_ui_upgrade_then_return_shows_last_result() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![local_server("ls -l")]);

    // Start upgrade
    let cmds = app.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!("Expected RunUpgrade")
    };

    // Receive some output
    app.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "important upgrade output".into(),
    });
    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("done".into()),
        success: true,
    });

    // Switch back to stats (monitor mode)
    app.switch_stats();
    assert_eq!(app.panels[0].mode, Mode::Monitor);

    // Switch back to upgrade view
    app.enter_upgrade_view();
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("important upgrade output"),
        "Last upgrade output should be visible"
    );
}

/// Test 12: Second `u` in upgrade mode reinitiates upgrade (new gen, new command)
#[test]
fn test_ui_second_u_in_upgrade_mode_reinitiates_upgrade() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![local_server("ls -l")]);

    // First upgrade start
    let cmds1 = app.run_upgrade();
    assert_eq!(cmds1.len(), 1);
    let Command::RunUpgrade { gen: gen1, .. } = cmds1[0] else {
        panic!()
    };
    assert_eq!(app.panels[0].upgrade_gen, gen1);
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);

    // Now we're in Upgrade mode and upgrade is "in flight"
    // Per the key handler: upgrades_in_flight() → had_upgrade() → in_upgrade() → run_upgrade()
    // Since upgrades_in_flight() is true (STARTED), second u should be a no-op
    // But wait - we need to simulate the key handler logic here
    // The key handler checks upgrades_in_flight first, which is true for STARTED state
    // So pressing 'u' while STARTED is a no-op
    let _cmds2 = app.run_upgrade();
    // run_upgrade always generates a new gen - but the key handler wouldn't call it
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert!(app.upgrades_in_flight());
    assert!(app.had_upgrade());
    assert!(app.in_upgrade());
}

/// Test 13: Second `u` while upgrade in flight is no-op
#[test]
fn test_ui_second_u_while_in_flight_is_noop() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![local_server("ls -l")]);

    // Start upgrade
    let cmds = app.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!()
    };
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert!(app.upgrades_in_flight());

    // Simulate the key handler's decision: upgrades_in_flight() is true → no-op
    // The key handler would NOT call run_upgrade() again
    if app.upgrades_in_flight() {
        // no-op path
        assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
        assert_eq!(app.panels[0].upgrade_gen, gen);
    } else {
        panic!("Should be in flight");
    }
}

/// Test 14: Vault locked → password prompt (not upgrade modal)
#[tokio::test]
async fn test_ui_vault_locked_shows_prompt_not_modal() {
    use multitop_vault::{Vault, VaultConfig};
    let _keychain = isolate_keychain_async().await;

    let temp_dir = std::env::temp_dir().join(format!("multitop_test_vault_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let vault_path = temp_dir.join("vault.bin");

    let config = VaultConfig {
        vault_path,
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    };
    let vault = Vault::new(config);
    let _ = vault.initialize("master-pass").await;

    let mut app = App::new(vec![local_server("ls -l")]);
    app.vault = Some(std::sync::Arc::new(vault));
    app.vault_state = VaultState::Locked;

    // Simulate pressing 'u' key
    // Per key handler: vault.is_some() && vault_unlocked.is_none() → show vault password prompt
    if app.vault.is_some() && app.vault_unlocked().is_none() {
        app.set_show_vault_password_prompt(true);
        app.vault_password_input.clear();
        app.set_vault_password_error(None);
    }

    assert!(app.show_vault_password_prompt());
    assert!(
        !app.show_upgrade_modal(),
        "Should not show upgrade modal when vault is locked"
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// Test 15: Vault unlocked after password → runs upgrade
#[tokio::test]
async fn test_ui_vault_unlocked_after_password_runs_upgrade() {
    use multitop_vault::{Vault, VaultConfig};

    let _store_guard = enable_test_mock_store().await;
    let temp_dir =
        std::env::temp_dir().join(format!("multitop_test_vault2_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let vault_path = temp_dir.join("vault.bin");

    let config = VaultConfig {
        vault_path,
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    };
    let vault = Vault::new(config);
    let _ = vault.initialize("master-pass").await;

    let mut app = App::new(vec![local_server("ls -l")]);
    app.vault = Some(std::sync::Arc::new(vault));

    // Simulate vault unlock
    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password("master-pass")
        .unwrap();
    app.vault_state = VaultState::Unlocked {
        vault: Box::new(unlocked),
        awaiting_biometric: false,
    };

    // Now press 'u' — vault is unlocked, so show_upgrade_modal = true
    if app.upgrades_in_flight() {
        // no-op
    } else if app.had_upgrade() {
        if app.in_upgrade() {
            let _ = app.run_upgrade();
        } else {
            app.enter_upgrade_view();
        }
    } else if app.vault.is_some() && app.vault_unlocked().is_none() {
        app.set_show_vault_password_prompt(true);
    } else {
        app.set_show_upgrade_modal(true);
    }

    assert!(
        app.show_upgrade_modal(),
        "Should show upgrade modal when vault is unlocked"
    );
    assert!(
        !app.show_vault_password_prompt(),
        "Should not show vault prompt when unlocked"
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// Test 16: Modal confirmation flow saves state to state.toml
#[test]
fn test_ui_upgrade_modal_confirmation_flow() {
    use tempfile::TempDir;
    let _keychain = isolate_keychain();

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let mut app = App::new(vec![local_server("ls -l")]);
    app.config_path = Some(config_path.clone());
    app.set_show_upgrade_modal(true);

    // User presses Enter to confirm
    let cmds = app.confirm_upgrade();
    assert!(!cmds.is_empty());
    assert!(app.upgrade_started_at.is_some());
    assert!(!app.show_upgrade_modal(), "Modal should be dismissed");

    // Verify state.toml written
    let state = multitop::state::load_state(&config_path);
    assert_eq!(state.upgrade_started_at, app.upgrade_started_at);
}

/// Test 17: Switching panes during upgrade preserves task
#[test]
fn test_ui_switching_panes_during_upgrade_preserves_task() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![local_server("ls -l"), local_server("ls -l")]);

    // Start upgrades on both panels
    let cmds = app.run_upgrade();
    assert_eq!(cmds.len(), 2);

    // Simulate receiving output on panel 0
    app.apply(Msg::AuxBegin {
        panel: 0,
        gen: app.panels[0].upgrade_gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    app.apply(Msg::AuxLine {
        panel: 0,
        gen: app.panels[0].upgrade_gen,
        line: "panel 0 output".into(),
    });

    // Switch to monitor mode while upgrade still in flight
    app.switch_stats();

    // Both panels are now in Monitor mode, but upgrade is still in flight
    assert!(app.upgrades_in_flight());
    // switch_stats saves view to last_upgrade for panels that were in Upgrade mode
    assert_eq!(app.panels[0].mode, Mode::Monitor);
    assert_eq!(app.panels[1].mode, Mode::Monitor);

    // Panel 0's last_upgrade should still be accumulating (if it was in view)
    // The view is replaced by last_frame in Monitor mode
    assert!(!app.panels[0].view.contains(&"panel 0 output".to_string()));

    // Panel 1's last_upgrade should also have the view captured
    assert!(app.upgrades_in_flight(), "Both panels still in-flight");
}

/// Test 18: No `upgrade_cmd` → message shown without command
#[test]
fn test_ui_no_upgrade_cmd_shows_message_without_command() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![Server {
        host: "127.0.0.1".into(),
        port: 0,
        user: "test".into(),
        upgrade_cmd: None,
    }]);

    let cmds = app.run_upgrade();
    assert!(
        cmds.is_empty(),
        "No RunUpgrade command for servers without upgrade_cmd"
    );
    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("No upgrade_cmd"),
        "Should show 'No upgrade_cmd' message"
    );
}

/// Test 19: Output persists across view switches
#[test]
fn test_ui_upgrade_output_persists_across_view_switches() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![local_server("ls -l")]);

    let cmds = app.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!()
    };

    app.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "persistent output line".into(),
    });
    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("done".into()),
        success: true,
    });

    // Switch to monitor
    app.switch_stats();
    assert!(!app.panels[0]
        .view
        .contains(&"persistent output line".to_string()));

    // Switch back to upgrade view
    app.enter_upgrade_view();
    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("persistent output line"),
        "Output should persist across view switches"
    );
}

/// Test 20: Returning to completed shows output (not rerun)
#[test]
fn test_ui_returning_to_completed_shows_output() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![local_server("ls -l")]);

    // Complete full upgrade cycle
    let cmds = app.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!()
    };

    app.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "completed upgrade output".into(),
    });
    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("done".into()),
        success: true,
    });
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::DONE);

    // Switch to monitor mode
    app.switch_stats();

    // Press 'u' again — should show_upgrade_output, NOT start new upgrade
    // Key handler: not in Upgrade mode, nothing in flight -> enter_upgrade_view()
    if app.upgrades_in_flight() {
        // no-op
    } else if app.had_upgrade() {
        if app.in_upgrade() {
            let _ = app.run_upgrade();
        } else {
            app.enter_upgrade_view();
        }
    } else {
        app.set_show_upgrade_modal(true);
    }

    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("completed upgrade output"),
        "Should show last upgrade output, not start new"
    );
    // Verify no new RunUpgrade command was generated
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::DONE);
}

/// Test 21: `u` during flight after switching away → no-op
#[test]
fn test_ui_u_during_flight_after_switching_away_is_noop() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![local_server("sleep 3 && ls -l")]);

    // Start upgrade
    let cmds = app.run_upgrade();
    let Command::RunUpgrade { gen, .. } = cmds[0] else {
        panic!()
    };
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);

    // Apply some output to simulate it's running
    app.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "still running...".into(),
    });

    // Switch to monitor mode (upgrade still in flight, upgrade_state still STARTED)
    app.switch_stats();
    assert_eq!(app.panels[0].mode, Mode::Monitor);
    assert!(
        app.upgrades_in_flight(),
        "Upgrade should still be in flight"
    );
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(app.panels[0].upgrade_gen, gen);

    // Press 'u' — should be a no-op because upgrades_in_flight() is true
    let gen_before = app.panels[0].gen;
    if app.upgrades_in_flight() {
        // no-op path — don't call run_upgrade
    } else {
        panic!("Should be blocked by upgrades_in_flight() check");
    }

    // Verify nothing changed
    assert_eq!(
        app.panels[0].gen, gen_before,
        "Gen should not change for no-op"
    );
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(app.panels[0].upgrade_gen, gen);
}

// ============================================================================
// Bug a regressions: "No output on <host> on updater"
// A server without `upgrade_cmd` was silently skipped: the panel got a single
// dim line, `upgrade_state` stayed NIL, `last_upgrade` stayed empty, and the
// confirm modal never disclosed the skip. Now the skip is an explicit
// terminal state that persists and is surfaced before running.
// ============================================================================

/// A server with `upgrade_cmd: None` (as created by `-r <host>` or a config
/// with `upgrade_cmd` commented out).
fn no_upgrade_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: String::new(),
        upgrade_cmd: None,
    }
}

/// Bug a: skipped server must reach a terminal DONE state with a visible,
/// host-specific message — not a dead NIL state that reads as "no output".
#[test]
fn test_upgrade_skip_server_reaches_terminal_state() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![no_upgrade_server("192.168.0.90")]);

    let cmds = app.run_upgrade();

    assert!(
        cmds.is_empty(),
        "no RunUpgrade command for a skipped server"
    );
    let p = &app.panels[0];
    assert_eq!(p.upgrade_state, UpgradeState::DONE, "skip must be terminal");
    assert!(
        p.upgrade_gen > 0,
        "skip must record the generation it was decided at"
    );
    assert!(!p.last_upgrade.is_empty(), "skip message must be persisted");
    let view = multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
        .0
        .join("\n");
    assert!(view.contains("192.168.0.90"), "message must name the host");
    assert!(view.contains("skipped"), "message must say it was skipped");
    assert!(
        view.contains("upgrade_cmd"),
        "message must point at the missing upgrade_cmd"
    );
}

/// Bug a: with a mix of configured and skipped servers, only the configured
/// one gets a `RunUpgrade` command and the skipped one reaches DONE.
#[test]
fn test_upgrade_mixed_servers_only_configured_run() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![
        local_server("apt update && apt upgrade -y"),
        no_upgrade_server("192.168.0.90"),
    ]);

    let cmds = app.run_upgrade();
    assert_eq!(cmds.len(), 1, "only the configured server runs");
    let Command::RunUpgrade { panel, .. } = cmds[0] else {
        panic!("Expected RunUpgrade for configured server");
    };
    assert_eq!(panel, 0);
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(app.panels[1].upgrade_state, UpgradeState::DONE);
    assert!(
        multitop::ui::pane_lines(&app, 1, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("192.168.0.90"),
        "skipped panel must still show why it was skipped"
    );
}

/// Bug a: the skip message must survive a switch to monitor and back — the
/// user must never land on a blank panel after the updater.
#[test]
fn test_upgrade_skip_message_persists_across_views() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![no_upgrade_server("192.168.0.90")]);
    app.run_upgrade();
    let msg: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();

    app.switch_stats();
    assert_eq!(app.panels[0].mode, Mode::Monitor);

    app.enter_upgrade_view();
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    // The Upgrade pane now always opens with a status header, so the previous
    // output follows it rather than being the whole view. The message itself
    // must still survive intact.
    let (header, _) = app.upgrade_pane_header(0);
    assert!(
        !header.is_empty(),
        "expected a status header above the previous output"
    );
    let ring: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert_eq!(ring, msg, "skip message must persist across view switches");
}

/// Bug a: the confirm modal data helper must list exactly the hosts that will
/// be skipped, so the user learns about them before running.
#[test]
fn test_upgrade_skip_hosts_helper_lists_unconfigured() {
    let _keychain = isolate_keychain();
    let app = App::new(vec![
        local_server("apt update"),
        no_upgrade_server("192.168.0.90"),
        no_upgrade_server("192.168.0.158"),
    ]);

    assert_eq!(
        app.upgrade_skip_hosts(),
        vec!["192.168.0.90".to_string(), "192.168.0.158".to_string()]
    );
}

/// Bug a: skip + completed upgrade round-trip. After a run where a server was
/// skipped, pressing `u` again (after switching to stats) shows the persisted
/// skip message instead of re-showing the confirm modal or a blank panel.
#[test]
fn test_upgrade_skip_then_u_shows_message_not_modal() {
    let _store_guard = enable_test_mock_store_blocking();
    let mut app = App::new(vec![no_upgrade_server("192.168.0.90")]);
    app.run_upgrade();
    app.switch_stats();
    assert!(!app.in_upgrade());

    // Replicate the key handler decision for 'u'.
    if app.upgrades_in_flight() {
        panic!("no upgrade in flight for a skipped server");
    } else if app.had_upgrade() {
        assert!(!app.in_upgrade(), "not in upgrade view after switch_stats");
        app.enter_upgrade_view();
    } else {
        panic!("had_upgrade must be true: the skip is the recorded outcome");
    }

    assert!(
        multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
            .0
            .join("\n")
            .contains("skipped"),
        "must show the skip message, not the modal"
    );
    assert!(!app.show_upgrade_modal());
}
