#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Argon2Params;
    use secrecy::ExposeSecret;
    use tempfile::TempDir;

    fn fast_vault_config(path: std::path::PathBuf) -> VaultConfig {
        VaultConfig {
            vault_path: path,
            argon2_params: Some(Argon2Params { t: 1, m_kib: 32768, p: 1 }),
        }
    }

    #[tokio::test]
    async fn test_vault_init_unlock() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        let password = "test-sudo-password-123";
        vault.initialize(password).await.unwrap();
        assert!(vault.exists());

        let mut unlocked = vault.unlock_with_password(password).unwrap();

        // Add a password
        unlocked.set_password("server1:22".into(), SecretString::from("pass123")).unwrap();
        assert_eq!(unlocked.get_password("server1:22").unwrap().expose_secret(), "pass123");

        // Lock and unlock again
        unlocked.lock();
        let unlocked2 = vault.unlock_with_password(password).unwrap();
        assert_eq!(unlocked2.get_password("server1:22").unwrap().expose_secret(), "pass123");
    }

    #[tokio::test]
    async fn test_vault_change_password() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        let old_pass = "old-password";
        let new_pass = "new-password-456";

        vault.initialize(old_pass).await.unwrap();
        vault.change_password(old_pass, new_pass).await.unwrap();

        // Old password should fail
        assert!(vault.unlock_with_password(old_pass).is_err());

        // New password should work
        let unlocked = vault.unlock_with_password(new_pass).unwrap();
        assert!(unlocked.get_password("server1:22").is_none()); // empty vault
    }

    #[tokio::test]
    async fn test_rate_limiting_lockout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        let password = "correct-password";
        vault.initialize(password).await.unwrap();

        // 3 wrong attempts
        for _ in 0..3 {
            assert!(vault.unlock_with_password("wrong").is_err());
        }

        // 4th should return RateLimited
        assert!(matches!(vault.unlock_with_password("wrong"), Err(VaultError::RateLimited(_))));

        // Wait past the 1-second backoff window
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Correct password resets lockout
        let unlocked = vault.unlock_with_password(password).unwrap();
        assert!(unlocked.get_password("test").is_none());

        // Counter is reset; wrong attempt should NOT be rate limited
        let result = vault.unlock_with_password("wrong");
        assert!(result.is_err());
        assert!(!matches!(result, Err(VaultError::RateLimited(_))));
    }
}