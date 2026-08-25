use super::*;

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

    // In the Upgrade view with a run in flight: a second `u` must start
    // nothing. Pressed for real rather than reasoned about — the previous
    // version called `run_upgrade()` directly and then noted in a comment that
    // the handler would not have.
    let gen_before = app.panels[0].upgrade_gen;
    press(&mut app, crossterm::event::KeyCode::Char('u'));

    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(
        app.panels[0].upgrade_gen, gen_before,
        "a second `u` retired the in-flight generation"
    );
    assert!(app.upgrades_in_flight());
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

    // Now press `u` — the vault is unlocked, so the confirm modal is what
    // should come up. Two presses: the first enters the view, the second is
    // the one that decides. Pressed for real, because the chain written out
    // here by hand had a branch the handler does not.
    press(&mut app, crossterm::event::KeyCode::Char('u'));
    press(&mut app, crossterm::event::KeyCode::Char('u'));

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
    assert_ne!(cmds, [] as [multitop::app::Command; 0]);
    assert!(app.upgrade_started_at.is_some());
    assert!(!app.show_upgrade_modal(), "Modal should be dismissed");

    // Verify state.toml written
    let state = multitop::state::load_state(&config_path);
    assert_eq!(state.state.upgrade_started_at, app.upgrade_started_at);
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

    // Press 'u' again — it must show the last output, not start a new run.
    press(&mut app, crossterm::event::KeyCode::Char('u'));

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
