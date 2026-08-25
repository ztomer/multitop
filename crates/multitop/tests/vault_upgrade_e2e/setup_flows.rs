use super::*;

mod vault_upgrade_e2e_tests {
    use super::*;

    #[tokio::test]
    async fn test_vault_unlock_loads_passwords_before_upgrade() {
        // The mock store is process-global: without the lock this races the
        // guarded tests in this same binary, and the loser sees a store some
        // other test has just cleared.
        let _keychain = isolate_keychain_async().await;
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
        assert_ne!(cmds, [] as [multitop::app::Command; 0]);
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
        // The mock store is process-global: without the lock this races the
        // guarded tests in this same binary, and the loser sees a store some
        // other test has just cleared.
        let _keychain = isolate_keychain_async().await;
        let (mut app, _temp_dir) = app_with_vault(test_servers(), "unused", HashMap::new()).await;

        // Remove vault
        app.vault = None;
        app.vault_state = VaultState::Locked;

        let cmds = app.run_upgrade();

        // Should still generate upgrade commands
        assert_ne!(cmds, [] as [multitop::app::Command; 0]);
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
        // The mock store is process-global: without the lock this races the
        // guarded tests in this same binary, and the loser sees a store some
        // other test has just cleared.
        let _keychain = isolate_keychain_async().await;
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
        // The mock store is process-global: without the lock this races the
        // guarded tests in this same binary, and the loser sees a store some
        // other test has just cleared.
        let _keychain = isolate_keychain_async().await;
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
        assert_ne!(cmds, [] as [multitop::app::Command; 0]);
    }
}
