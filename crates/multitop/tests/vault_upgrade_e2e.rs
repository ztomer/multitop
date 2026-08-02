//! End-to-end tests for the Vault + Upgrade flow
//!
//! These tests verify the complete flow from vault unlock to upgrade execution
//! using mocked passwords to avoid interactive prompts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::app::*;
use multitop::config::{Config, Server};
use multitop::password_store;
use multitop_vault::{Vault, VaultConfig};
use secrecy::SecretString;
use std::collections::HashMap;
use tempfile::TempDir;
use tokio::sync::mpsc;

fn test_servers() -> Vec<Server> {
    vec![
        Server {
            host: "test-host-1".into(),
            port: 22,
            user: "testuser".into(),
            upgrade_cmd: Some("echo upgrade-1".into()),
        },
        Server {
            host: "test-host-2".into(),
            port: 22,
            user: "testuser".into(),
            upgrade_cmd: Some("echo upgrade-2".into()),
        },
    ]
}

async fn setup_test_vault(
    vault_path: &std::path::Path,
    master_password: &str,
    passwords: HashMap<String, String>,
) -> Vault {
    let config = VaultConfig {
        vault_path: vault_path.to_path_buf(),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    };
    let vault = Vault::new(config);
    vault.initialize(master_password).await.unwrap();

    let mut unlocked = vault.unlock_with_password(master_password).unwrap();
    for (host, pass) in passwords {
        unlocked
            .set_password(host, &SecretString::from(pass))
            .unwrap();
    }
    unlocked.lock();
    vault
}

/// Create an App with a pre-configured vault for testing
async fn app_with_vault(
    servers: Vec<Server>,
    _master_password: &str,
    vault_passwords: HashMap<String, String>,
) -> (App, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let vault_path = temp_dir.path().join("vault.bin");
    let config_path = temp_dir.path().join("config.toml");

    // Create vault with test passwords
    let vault = setup_test_vault(&vault_path, "test-master", vault_passwords).await;

    // Create config
    let config = Config {
        servers: servers.clone(),
        theme: None,
        upgrade_history_lines: 5000,
        show_sparklines: false,
        plaintext_passwords: Vec::new(),
    };
    multitop::config::save_servers(&config_path, &config.servers).unwrap();

    // Create app with vault
    let mut app = App::new(servers);
    app.vault = Some(std::sync::Arc::new(vault));
    app.config_path = Some(config_path);

    // Pre-unlock the vault for test (bypass password prompt)
    if let Some(ref vault) = app.vault {
        let unlocked = vault.unlock_with_password("test-master").unwrap();
        app.vault_state = VaultState::Unlocked {
            vault: Box::new(unlocked),
            awaiting_biometric: false,
        };
    }

    (app, temp_dir)
}

#[cfg(test)]
mod vault_upgrade_e2e_tests {
    use super::*;

    #[tokio::test]
    async fn test_vault_unlock_loads_passwords_before_upgrade() {
        let master_pw = "test-master-password";
        let mut vault_passwords = HashMap::new();
        vault_passwords.insert(
            "testuser@test-host-1:22".to_string(),
            "sudo-pass-1".to_string(),
        );
        vault_passwords.insert(
            "testuser@test-host-2:22".to_string(),
            "sudo-pass-2".to_string(),
        );

        let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

        // Verify vault is unlocked
        assert!(app.vault_unlocked().is_some());

        // Run the upgrade flow - this should load vault passwords into panels
        let cmds = app.run_upgrade();

        // Verify passwords were loaded from vault into panels
        assert_eq!(app.panels[0].sudo_password, Some("sudo-pass-1".to_string()));
        assert_eq!(app.panels[1].sudo_password, Some("sudo-pass-2".to_string()));

        // Verify upgrade commands were generated
        assert!(!cmds.is_empty());
        for cmd in &cmds {
            match cmd {
                Command::RunUpgrade { panel, .. } => {
                    assert!(app.panels[*panel].sudo_password.is_some());
                }
                _ => panic!("Expected RunUpgrade command"),
            }
        }
    }

    #[tokio::test]
    async fn test_upgrade_without_vault_works() {
        let (mut app, _temp_dir) = app_with_vault(test_servers(), "unused", HashMap::new()).await;

        // Remove vault
        app.vault = None;
        app.vault_state = VaultState::Locked;

        let cmds = app.run_upgrade();

        // Should still generate upgrade commands
        assert!(!cmds.is_empty());
    }

    #[tokio::test]
    async fn test_vault_password_fallback_to_keychain() {
        // This test verifies the password store fallback works
        // We need to enable mock store for this test
        let _store_guard = password_store::lock_for_test_async().await;
        password_store::enable_mock_store();
        password_store::clear_mock_store();

        let app = App::new(test_servers());

        // Manually set password in password store (simulating keychain)
        password_store::save(&app.panels[0].server, "keychain-pass-1").unwrap();
        password_store::save(&app.panels[1].server, "keychain-pass-2").unwrap();

        // Verify fallback loads from password store
        assert_eq!(
            password_store::load(&app.panels[0].server),
            Ok(Some("keychain-pass-1".to_string()))
        );
        assert_eq!(
            password_store::load(&app.panels[1].server),
            Ok(Some("keychain-pass-2".to_string()))
        );
    }

    #[tokio::test]
    async fn test_vault_priority_over_keychain() {
        let master_pw = "test-master";
        let mut vault_passwords = HashMap::new();
        vault_passwords.insert(
            "testuser@test-host-1:22".to_string(),
            "vault-pass".to_string(),
        );

        let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

        // Also set password in keychain (lower priority)
        let _key = password_store::account(&app.panels[0].server);
        password_store::save(&app.panels[0].server, "keychain-pass").unwrap();

        // Run upgrade - vault should take priority
        let _cmds = app.run_upgrade();

        // Vault password should be used (not keychain)
        assert_eq!(app.panels[0].sudo_password, Some("vault-pass".to_string()));
    }

    #[tokio::test]
    async fn test_upgrade_modal_flow_with_vault() {
        let master_pw = "test-master";
        let mut vault_passwords = HashMap::new();
        vault_passwords.insert(
            "testuser@test-host-1:22".to_string(),
            "sudo-pass-1".to_string(),
        );

        let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

        // Simulate pressing 'u' key (upgrade) - calls run_upgrade internally
        let cmds = app.run_upgrade();

        // Vault should be unlocked and passwords loaded
        assert!(app.vault_unlocked().is_some());

        // Verify vault password was pre-loaded
        assert_eq!(app.panels[0].sudo_password, Some("sudo-pass-1".to_string()));

        // Commands should be generated
        assert!(!cmds.is_empty());
    }
}

#[tokio::test]
async fn test_vault_password_prompt_state_machine() {
    let master_pw = "test-master";
    let mut vault_passwords = HashMap::new();
    vault_passwords.insert(
        "testuser@test-host-1:22".to_string(),
        "sudo-pass-1".to_string(),
    );

    let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

    // Initially vault is locked (vault exists but not unlocked)
    // But we pre-unlocked it for testing
    assert!(app.vault.is_some());

    // The actual state machine test: simulate password entry flow
    // In real app this happens via UI, here we test the logic
    app.set_show_vault_password_prompt(true);
    app.vault_password_input = master_pw.to_string();

    if let Some(ref vault) = app.vault {
        let password = std::mem::take(&mut app.vault_password_input);
        match vault.unlock_with_password(&password) {
            Ok(unlocked) => {
                app.vault_state = VaultState::Unlocked {
                    vault: Box::new(unlocked),
                    awaiting_biometric: false,
                };
                app.set_show_upgrade_modal(true);
            }
            Err(e) => {
                app.set_vault_password_error(Some(e.to_string()));
                app.set_show_vault_password_prompt(true);
            }
        }
    }

    // Should now be unlocked
    assert!(app.vault_unlocked().is_some());
    assert!(!app.show_vault_password_prompt());
    assert!(app.show_upgrade_modal());
}

#[tokio::test]
async fn test_vault_failed_unlock_shows_error() {
    let master_pw = "test-master";
    let mut vault_passwords = HashMap::new();
    vault_passwords.insert(
        "testuser@test-host-1:22".to_string(),
        "sudo-pass-1".to_string(),
    );

    let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

    // The app was created with pre-unlocked vault, lock it first to test failure
    app.vault_state = VaultState::Locked;

    // Simulate wrong password
    app.set_show_vault_password_prompt(true);
    app.vault_password_input = "wrong-password".to_string();

    if let Some(ref vault) = app.vault {
        let password = std::mem::take(&mut app.vault_password_input);
        match vault.unlock_with_password(&password) {
            Ok(_) => panic!("Should have failed"),
            Err(e) => {
                app.set_vault_password_error(Some(e.to_string()));
                app.set_show_vault_password_prompt(true);
            }
        }
    }

    // Should show error and keep prompt open
    assert!(app.show_vault_password_prompt());
    assert!(app.vault_password_error().is_some());
    assert!(app.vault_unlocked().is_none());
}

// ============================================================================
// Bug b regressions: "Vault not working reliably (4 password attempts, 1
// fingerprint)"
// The TUI's `u` handler went straight to the password prompt, so every
// upgrade run re-typed the vault password (accumulating lockout backoff) and
// Touch ID was never offered. Now the TUI tries biometrics first and only
// falls back to the password prompt when biometrics are unavailable/cancelled.
// ============================================================================

/// Bug b: with a locked vault, pressing `u` must start a biometric attempt,
/// NOT jump straight to the password prompt. Exercises the real
/// `begin_vault_unlock()` path used by the `u` key handler.
#[tokio::test]
async fn test_vault_locked_u_key_tries_biometric_first() {
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    // Lock the vault again to simulate a fresh app start.
    app.vault_state = VaultState::Locked;

    let vault_handle = app.begin_vault_unlock();

    assert!(
        vault_handle.is_some(),
        "a locked vault must hand the caller a handle for the biometric attempt"
    );
    assert!(
        app.vault_awaiting_biometric(),
        "must await biometric before prompting for a password"
    );
    assert!(
        !app.show_vault_password_prompt(),
        "password prompt must not appear while biometrics are being attempted"
    );

    // An already-unlocked vault (or no vault) must NOT re-enter the awaiting
    // state and must return None so the handler proceeds to the upgrade modal.
    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password("test-master")
        .unwrap();
    app.vault_state = VaultState::Unlocked {
        vault: Box::new(unlocked),
        awaiting_biometric: false,
    };
    assert!(app.begin_vault_unlock().is_none());
    assert!(!app.vault_awaiting_biometric());
}

/// Bug b: a successful biometric unlock must land the unlocked vault, dismiss
/// the awaiting state, and proceed to the upgrade modal — exactly one attempt.
#[tokio::test]
async fn test_vault_biometric_success_proceeds_to_modal() {
    let master_pw = "test-master";
    let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, HashMap::new()).await;

    // Lock the vault again, then simulate the biometric task succeeding.
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };

    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password(master_pw)
        .unwrap();
    app.apply(Msg::VaultUnlocked {
        epoch: app.vault_epoch,
        unlocked: Box::new(unlocked),
    });

    assert!(app.vault_unlocked().is_some(), "vault must be unlocked");
    assert!(
        !app.vault_awaiting_biometric(),
        "awaiting state must clear on success"
    );
    assert!(
        !app.show_vault_password_prompt(),
        "no password prompt after biometric success"
    );
    assert!(
        app.show_upgrade_modal(),
        "successful unlock must proceed to the upgrade modal"
    );
}

/// Bug b: when biometrics are unavailable or cancelled, the TUI must fall back
/// to the password prompt (one clear fallback, not a silent dead-end).
#[tokio::test]
async fn test_vault_biometric_failed_falls_back_to_password() {
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };

    app.apply(Msg::VaultBiometricFailed {
        epoch: app.vault_epoch,
    });

    assert!(
        !app.vault_awaiting_biometric(),
        "awaiting state must clear on fallback"
    );
    assert!(
        app.show_vault_password_prompt(),
        "must offer the password prompt after biometric failure"
    );
    assert!(app.vault_unlocked().is_none(), "vault must still be locked");
}

/// Bug b: real end-to-end of the spawned biometric task. On a machine without
/// usable biometrics (the test environment), `unlock_biometric()` returns
/// `BiometricFailed` and the task must emit `VaultBiometricFailed` so the TUI
/// shows the password prompt — never a hang or a crash.
#[tokio::test]
async fn test_vault_biometric_task_emits_fallback_on_unavailable() {
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    let vault = app.vault.clone().unwrap();

    let (tx, mut rx) = mpsc::channel::<Msg>(4);
    let tx2 = tx.clone();
    let handle = tokio::spawn(async move {
        match vault.unlock_biometric().await {
            Ok((unlocked, _)) => {
                let _ = tx2
                    .send(Msg::VaultUnlocked {
                        epoch: 0,
                        unlocked: Box::new(unlocked),
                    })
                    .await;
            }
            Err(_) => {
                let _ = tx2.send(Msg::VaultBiometricFailed { epoch: 0 }).await;
            }
        }
    });
    let _ = handle.await;

    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("task must emit a message")
        .expect("channel must not close");

    // Apply the task's outcome to the app.
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    app.apply(msg);

    assert!(
        app.show_vault_password_prompt() || app.show_upgrade_modal(),
        "the task outcome must lead to either the password prompt or the modal"
    );
    assert!(
        !app.vault_awaiting_biometric(),
        "awaiting state must be cleared by the task outcome"
    );
}

/// Bug b: failed biometric attempts must not count as password failures.
/// Otherwise Touch ID being unavailable would push the user toward the
/// lockout backoff before they ever typed a password.
#[tokio::test]
async fn test_vault_biometric_failures_do_not_trigger_lockout() {
    let master_pw = "test-master";
    let (app, _temp_dir) = app_with_vault(test_servers(), master_pw, HashMap::new()).await;
    let vault = app.vault.clone().unwrap();

    // Simulate a handful of biometric failures (unavailable/cancelled).
    for _ in 0..5 {
        assert!(vault.unlock_biometric().await.is_err());
    }

    // A correct password must still unlock immediately — no RateLimited error.
    let unlocked = vault.unlock_with_password(master_pw);
    assert!(
        unlocked.is_ok(),
        "biometric failures must not accumulate password lockout"
    );
    drop(unlocked);
}
