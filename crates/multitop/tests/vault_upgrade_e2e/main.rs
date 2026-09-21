//! End-to-end tests for the Vault + Upgrade flow
//!
//! These tests verify the complete flow from vault unlock to upgrade execution
//! using mocked passwords to avoid interactive prompts.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

// Integration-test crate: helper fns outside #[test] are not covered by
// clippy.toml's test exemption, so the restriction lints are expected here.

use multitop::app::*;
use multitop::config::{Config, Server};
use multitop::password_store;
use multitop_vault::{Vault, VaultConfig};
use secrecy::SecretString;
use std::collections::HashMap;
use tempfile::TempDir;
use tokio::sync::mpsc;

async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn test_servers() -> Vec<Server> {
    vec![
        Server {
            host: "test-host-1".into(),
            port: 22,
            user: "testuser".into(),
            upgrade_cmd: Some("echo upgrade-1".into()),
            custom_command: None,
        },
        Server {
            host: "test-host-2".into(),
            port: 22,
            user: "testuser".into(),
            upgrade_cmd: Some("echo upgrade-2".into()),
            custom_command: None,
        },
    ]
}

fn setup_test_vault(
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
    vault.initialize(master_password).unwrap();

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
fn app_with_vault(
    servers: Vec<Server>,
    _master_password: &str,
    vault_passwords: HashMap<String, String>,
) -> (App, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let vault_path = temp_dir.path().join("vault.bin");
    let config_path = temp_dir.path().join("config.toml");

    // Create vault with test passwords
    let vault = setup_test_vault(&vault_path, "test-master", vault_passwords);

    // Create config
    let config = Config {
        servers: servers.clone(),
        theme: None,
        upgrade_history_lines: 5000,
        history_lines_raised_from: None,
        banner_style: multitop::layout::BannerStyle::default(),
        plaintext_passwords: Vec::new(),
        alert_cpu: None,
        alert_mem: None,
        alert_disk: None,
        alerts: Vec::new(),
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
mod credential_stores;
mod setup_flows;
mod unlock_and_biometrics;
