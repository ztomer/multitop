//! High-level Vault API

use crate::crypto::{self, WrapperType, now_ms};
use crate::format;
use crate::lockout::{LockoutGuard, LockoutState};
use crate::secure_enclave;
use crate::{VaultConfig, VaultError, VaultContents};
use secrecy::SecretString;
use zeroize::Zeroize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;


/// Result of vault unlock
#[derive(Debug)]
pub enum UnlockResult {
    Biometric,
    Password,
}

/// In-memory unlocked vault
pub struct UnlockedVault {
    vault_key: crate::crypto::VaultKey,
    contents: VaultContents,
    header: crate::format::VaultHeader,
    file_path: PathBuf,
}

impl Drop for UnlockedVault {
    fn drop(&mut self) {
        self.vault_key.zeroize();
    }
}

impl UnlockedVault {
    /// Get password for a host
    pub fn get_password(&self, host: &str) -> Option<SecretString> {
        self.contents.get(host)
    }

    /// Set password for a host
    pub fn set_password(&mut self, host: String, password: SecretString) -> Result<(), VaultError> {
        self.contents.set(host, password);
        self.save()?;
        Ok(())
    }

    /// Remove password for a host
    pub fn remove_password(&mut self, host: &str) -> Result<bool, VaultError> {
        let removed = self.contents.remove(host);
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// List all hosts with passwords
    pub fn hosts(&self) -> Vec<String> {
        self.contents.hosts()
    }

    /// Save vault to disk (encrypts and writes atomically)
    pub fn save(&mut self) -> Result<(), VaultError> {
        self.header.counter += 1;
        self.header.created_timestamp_ms = now_ms();

        let plaintext = serde_json::to_vec(&self.contents)
            .map_err(|e| VaultError::Serialization(e.to_string()))?;

        let (ciphertext, nonce) = crypto::encrypt_vault(&self.vault_key, &plaintext)?;
        self.header.nonce = nonce;

        // Sign the vault (signs header + ciphertext)
        self.header.signature = crypto::sign_vault(&self.vault_key, &self.header.signed_data(&ciphertext));

        // Write atomically
        format::atomic_write_vault(&self.file_path, &self.header, &ciphertext)?;
        Ok(())
    }

    /// Lock the vault (zeroize sensitive data)
    pub fn lock(mut self) {
        self.vault_key.zeroize();
        // VaultContents cleared on drop
    }
}

/// Vault manager
pub struct Vault {
    config: VaultConfig,
    unlocked: Arc<Mutex<Option<UnlockedVault>>>,
    lockout: StdMutex<LockoutState>,
}

impl Vault {
    /// Create new vault manager
    pub fn new(config: VaultConfig) -> Self {
        let lockout = LockoutState::load(&config.vault_path);
        Self {
            config,
            unlocked: Arc::new(Mutex::new(None)),
            lockout: StdMutex::new(lockout),
        }
    }

    /// Check if vault file exists
    pub fn exists(&self) -> bool {
        self.config.vault_path.exists()
    }

    /// Get vault path
    pub fn path(&self) -> &PathBuf {
        &self.config.vault_path
    }

    /// Initialize a new vault with the system password
    pub async fn initialize(&self, system_password: &str) -> Result<(), VaultError> {
        if self.exists() {
            return Err(VaultError::AlreadyExists("Vault already exists".into()));
        }

        // Create vault directory
        if let Some(parent) = self.config.vault_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(VaultError::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                    .map_err(VaultError::Io)?;
            }
        }

        // Generate vault key
        let vault_key = crypto::VaultKey::new();

        // Create empty contents
        let contents = VaultContents::default();

        // Wrap with Argon2id
        let salt = crypto::generate_salt();
        let params = self.config.argon2_params.unwrap_or_default();
        let argon2id_wrapper = crypto::wrap_argon2id(&vault_key, system_password, &salt, &params)?;

        // Try to create Secure Enclave wrapper (macOS)
        let mut wrappers = vec![crypto::Wrapper::new(WrapperType::Argon2id, argon2id_wrapper)?];

        #[cfg(target_os = "macos")]
        if let Ok(se) = secure_enclave::get_secure_enclave() {
            if let Ok(se_wrapper) = se.wrap_key(&vault_key) {
                wrappers.insert(0, se_wrapper); // Biometric first
            }
        }

        // Build header
        let header = format::VaultHeader::new(
            crypto::Ed25519PublicKey(vault_key.derive_verifying_key().to_bytes()),
            salt,
            params,
            wrappers,
        )?;

        // Encrypt contents
        let plaintext = serde_json::to_vec(&contents)
            .map_err(|e| VaultError::Serialization(e.to_string()))?;

        let (ciphertext, nonce) = crypto::encrypt_vault(&vault_key, &plaintext)?;
        let mut header = header;
        header.nonce = nonce;
        header.signature = crypto::sign_vault(&vault_key, &header.signed_data(&ciphertext));

        // Write atomically
        format::atomic_write_vault(&self.config.vault_path, &header, &ciphertext)?;

        Ok(())
    }

    /// Unlock vault with biometric (Touch ID / fingerprint)
    /// Falls back to password if biometric fails or unavailable
    pub async fn unlock_biometric(&self, password_fallback: bool) -> Result<(UnlockedVault, UnlockResult), VaultError> {
        // Try biometric first
        if let Ok(vault) = self.try_unlock_biometric().await {
            return Ok((vault, UnlockResult::Biometric));
        }

        // Biometric failed or unavailable
        if !password_fallback {
            return Err(VaultError::BiometricFailed);
        }

        // Prompt for password
        let password = rpassword::prompt_password("Enter sudo password to unlock vault: ")
            .map_err(|e| VaultError::Io(std::io::Error::other(e)))?;

        let vault = self.unlock_with_password(&password)?;
        Ok((vault, UnlockResult::Password))
    }

    /// Try to unlock with biometric only (no password fallback)
    async fn try_unlock_biometric(&self) -> Result<UnlockedVault, VaultError> {
        // Load vault file
        let vault_file = format::read_vault_file(&self.config.vault_path)?;

        // Verify signature BEFORE decrypting
        crypto::verify_vault_signature(&vault_file.header.ed25519_pk, &vault_file.header.signed_data(&vault_file.ciphertext), &vault_file.header.signature)
            .map_err(|_| VaultError::Corrupted("signature verification failed".into()))?;

        // Try Secure Enclave (macOS)
        #[cfg(target_os = "macos")]
        if let Some(se_wrapper) = vault_file.header.get_wrapper(WrapperType::SecureEnclave) {
            if let Ok(se) = secure_enclave::get_secure_enclave() {
                match se.unwrap_key(se_wrapper) {
                    Ok(vault_key) => {
                        return self.decrypt_and_load(vault_key, &vault_file);
                    }
                    Err(VaultError::BiometricFailed) => {
                        // Fall through to password
                    }
                    Err(e) => {
                        // Key invalidated (macOS update, etc.) - fall through
                        eprintln!("Secure Enclave error: {:?}", e);
                    }
                }
            }
        }

        // Try fprintd (Linux)
        #[cfg(target_os = "linux")]
        if vault_file.header.has_wrapper(WrapperType::Tpm2) {
            // TPM2 wrapper would go here
        } else if let Ok(fv) = fprintd::FingerprintVerifier::new().await {
            match fv.verify(30).await {
                Ok(crate::fprintd::FingerprintResult::Verified) => {
                    // If we had a TPM2 wrapper, we'd use it
                    // For now, fall through to password
                }
                Ok(crate::fprintd::FingerprintResult::Failed) => {
                    return Err(VaultError::BiometricFailed);
                }
                Ok(crate::fprintd::FingerprintResult::Timeout) => {
                    return Err(VaultError::BiometricFailed);
                }
                _ => {}
            }
        }

        Err(VaultError::BiometricFailed)
    }

    /// Unlock vault with password
    pub fn unlock_with_password(&self, password: &str) -> Result<UnlockedVault, VaultError> {
        let vault_file = format::read_vault_file(&self.config.vault_path)?;

        // Verify signature
        crypto::verify_vault_signature(&vault_file.header.ed25519_pk, &vault_file.header.signed_data(&vault_file.ciphertext), &vault_file.header.signature)
            .map_err(|_| VaultError::Corrupted("signature verification failed".into()))?;

        // Check rate limiting before attempting password
        let now = now_ms();
        {
            let lockout = self.lockout.lock().unwrap();
            lockout.check_lockout(now)?;
        }

        // Create guard that records failures on drop
        let mut guard = LockoutGuard::new(&self.lockout, &self.config.vault_path, now);

        // Find Argon2id wrapper
        let argon2id_wrapper = vault_file.header.get_wrapper(WrapperType::Argon2id)
            .ok_or_else(|| VaultError::Corrupted("no Argon2id wrapper found".into()))?;

        let params = &vault_file.header.argon2_params;
        let vault_key = crypto::unwrap_argon2id(&argon2id_wrapper.data, password, &vault_file.header.salt, params)?;

        let unlocked = self.decrypt_and_load(vault_key, &vault_file)?;

        guard.mark_success();
        Ok(unlocked)
    }

    /// Decrypt vault contents with key
    fn decrypt_and_load(&self, vault_key: crypto::VaultKey, vault_file: &format::VaultFile) -> Result<UnlockedVault, VaultError> {
        let plaintext = crypto::decrypt_vault(&vault_key, &vault_file.header.nonce, &vault_file.ciphertext)?;

        let contents: VaultContents = serde_json::from_slice(&plaintext)
            .map_err(|e| VaultError::Serialization(e.to_string()))?;

        // Verify canary
        if !contents.verify_canary(&vault_file.header.canary) {
            return Err(VaultError::Corrupted("canary mismatch - wrong password or corrupted".into()));
        }

        let header = vault_file.header.clone();
        let file_path = self.config.vault_path.clone();

        Ok(UnlockedVault {
            vault_key,
            contents,
            header,
            file_path,
        })
    }

    /// Get cached unlocked vault or unlock
    pub async fn get_unlocked(&self) -> Result<UnlockedVault, VaultError> {
        let mut unlocked = self.unlocked.lock().await;
        if let Some(vault) = unlocked.take() {
            return Ok(vault);
        }
        drop(unlocked);

        // Try biometric with password fallback
        let (vault, _) = self.unlock_biometric(true).await?;
        Ok(vault)
    }

    /// Lock the vault (clear memory)
    pub async fn lock(&self) {
        let mut unlocked = self.unlocked.lock().await;
        if let Some(vault) = unlocked.take() {
            vault.lock();
        }
    }

    /// Add a biometric wrapper to existing vault (re-bind)
    pub async fn rebind_biometric(&self, password: &str) -> Result<(), VaultError> {
        let mut vault = self.unlock_with_password(password)?;

        #[cfg(target_os = "macos")]
        {
            if let Ok(se) = secure_enclave::get_secure_enclave() {
                let se_wrapper = se.wrap_key(&vault.vault_key)?;
                vault.header.add_wrapper(se_wrapper)?;
                vault.save()?;
                return Ok(());
            }
        }

        Err(VaultError::PlatformNotSupported("No biometric hardware available".into()))
    }

    /// Change vault password
    pub async fn change_password(&self, old_password: &str, new_password: &str) -> Result<(), VaultError> {
        let mut vault = self.unlock_with_password(old_password)?;

        // Create new Argon2id wrapper with new password
        let salt = crypto::generate_salt();
        let params = self.config.argon2_params.unwrap_or_default();
        let argon2id_wrapper = crypto::wrap_argon2id(&vault.vault_key, new_password, &salt, &params)?;

        // Update header
        vault.header.salt = salt;
        vault.header.argon2_params = params;
        vault.header.replace_wrapper(crypto::Wrapper::new(WrapperType::Argon2id, argon2id_wrapper)?)?;
        vault.header.key_version += 1;

        // Re-encrypt with new nonce
        let plaintext = serde_json::to_vec(&vault.contents)
            .map_err(|e| VaultError::Serialization(e.to_string()))?;
        let (ciphertext, nonce) = crypto::encrypt_vault(&vault.vault_key, &plaintext)?;
        vault.header.nonce = nonce;
        vault.header.signature = crypto::sign_vault(&vault.vault_key, &vault.header.signed_data(&ciphertext));

        // Write
        format::atomic_write_vault(&vault.file_path, &vault.header, &ciphertext)?;

        Ok(())
    }

    /// Delete vault file
    pub fn delete(&self) -> Result<(), VaultError> {
        if self.exists() {
            std::fs::remove_file(&self.config.vault_path)
                .map_err(VaultError::Io)?;
        }
        Ok(())
    }
}

/// Run upgrade/migration if needed
pub async fn migrate_if_needed(vault_path: &std::path::Path) -> Result<(), VaultError> {
    if !vault_path.exists() {
        return Ok(());
    }

    let vault_file = format::read_vault_file(vault_path)?;
    if vault_file.header.version < 2 {
        // Migrate v1 -> v2
        // For now, just rebuild with v2 format
    }
    Ok(())
}

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