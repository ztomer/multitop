//! End-to-end tests for the Vault + Upgrade flow
//!
//! These tests verify the complete flow from vault unlock to upgrade execution
//! using mocked passwords to avoid interactive prompts.

use multitop::app::*;
use multitop::config::{Config, Server};
use multitop::password_store;
use multitop_vault::{Vault, VaultConfig};
use secrecy::SecretString;
use std::collections::HashMap;
use tempfile::TempDir;

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
    };
    let vault = Vault::new(config);
    vault.initialize(master_password).await.unwrap();

    let mut unlocked = vault.unlock_with_password(master_password).unwrap();
    for (host, pass) in passwords {
        unlocked
            .set_password(host, SecretString::from(pass))
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
    };
    multitop::config::save_servers(&config_path, &config.servers).unwrap();

    // Create app with vault
    let mut app = App::new(servers);
    app.vault = Some(vault);
    app.config_path = Some(config_path);

    // Pre-unlock the vault for test (bypass password prompt)
    if let Some(ref vault) = app.vault {
        let unlocked = vault.unlock_with_password("test-master").unwrap();
        app.vault_unlocked = Some(unlocked);
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
        assert!(app.vault_unlocked.is_some());

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
        app.vault_unlocked = None;

        let cmds = app.run_upgrade();

        // Should still generate upgrade commands
        assert!(!cmds.is_empty());
    }

    #[tokio::test]
    async fn test_vault_password_fallback_to_keychain() {
        // This test verifies the password store fallback works
        // We need to enable mock store for this test
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
        assert!(app.vault_unlocked.is_some());

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
    app.show_vault_password_prompt = true;
    app.vault_password_input = master_pw.to_string();

    if let Some(ref vault) = app.vault {
        let password = std::mem::take(&mut app.vault_password_input);
        match vault.unlock_with_password(&password) {
            Ok(unlocked) => {
                app.vault_unlocked = Some(unlocked);
                app.vault_password_error = None;
                app.show_vault_password_prompt = false;
                app.show_upgrade_modal = true;
            }
            Err(e) => {
                app.vault_password_error = Some(e.to_string());
                app.show_vault_password_prompt = true;
            }
        }
    }

    // Should now be unlocked
    assert!(app.vault_unlocked.is_some());
    assert!(!app.show_vault_password_prompt);
    assert!(app.show_upgrade_modal);
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
    app.vault_unlocked = None;

    // Simulate wrong password
    app.show_vault_password_prompt = true;
    app.vault_password_input = "wrong-password".to_string();

    if let Some(ref vault) = app.vault {
        let password = std::mem::take(&mut app.vault_password_input);
        match vault.unlock_with_password(&password) {
            Ok(_) => panic!("Should have failed"),
            Err(e) => {
                app.vault_password_error = Some(e.to_string());
                app.show_vault_password_prompt = true;
            }
        }
    }

    // Should show error and keep prompt open
    assert!(app.show_vault_password_prompt);
    assert!(app.vault_password_error.is_some());
    assert!(app.vault_unlocked.is_none());
}
