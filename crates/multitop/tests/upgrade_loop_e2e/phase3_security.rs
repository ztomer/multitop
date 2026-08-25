use super::*;

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
    assert_ne!(cmds, [] as [multitop::app::Command; 0]);
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
    // Not `let _ =`: a panic inside the spawned task arrives here as a join
    // error, and swallowing it leaves the test to fail later for some other
    // reason -- or to pass. The task is the thing under test.
    handle.await.expect("the spawned task must not panic");

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
    assert_eq!(state.state.last_update, app.last_update);
    assert!(state.state.last_update.is_some());

    // Simulate App restart with same config
    let mut app2 = App::new(vec![local_server("ls -l")]);
    app2.config_path = Some(config_path.clone());

    let loaded = multitop::state::load_state(&config_path);
    assert_eq!(
        loaded.state.last_update, app.last_update,
        "last_update should persist across restart"
    );
}
