//! High-level Vault API

use crate::crypto::{self, now_ms, WrapperType};
use crate::format;
use crate::lockout::{LockoutGuard, LockoutState};
use crate::secure_enclave;
use crate::{VaultConfig, VaultContents, VaultError};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;
use zeroize::Zeroize;

/// Result of vault unlock
#[derive(Debug)]
pub enum UnlockResult {
    Biometric,
    Password,
}

/// In-memory unlocked vault
pub struct UnlockedVault {
    vault_key: crate::crypto::VaultKey,
    _key_lock: crate::mlock::LockedMemory, // mlock the vault key to prevent swapping
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

        let mut plaintext = serde_json::to_vec(&self.contents)
            .map_err(|e| VaultError::Serialization(e.to_string()))?;

        let (ciphertext, nonce) = crypto::encrypt_vault(&self.vault_key, &plaintext)?;

        // Zeroize plaintext after encryption
        plaintext.zeroize();

        self.header.nonce = nonce;

        // Sign the vault (signs header + ciphertext)
        self.header.signature =
            crypto::sign_vault(&self.vault_key, &self.header.signed_data(&ciphertext));

        // Write atomically
        format::atomic_write_vault(&self.file_path, &self.header, &ciphertext)?;

        // Update rollback counter in keychain
        crate::rollback::store_counter(
            &self.file_path,
            self.header.counter,
            self.header.created_timestamp_ms,
        );

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
            std::fs::create_dir_all(parent).map_err(VaultError::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                    .map_err(VaultError::Io)?;
            }
        }

        // Generate vault key
        let vault_key = crypto::VaultKey::new();

        // Generate canary first (will be used in both header and contents)
        let canary = format::VaultHeader::generate_canary();

        // Create empty contents with canary
        let mut contents = VaultContents::default();
        contents.set_canary(canary.clone());

        // Wrap with Argon2id
        let salt = crypto::generate_salt();
        let params = self.config.argon2_params.unwrap_or_default();
        let argon2id_wrapper = crypto::wrap_argon2id(&vault_key, system_password, &salt, &params)?;

        // Try to create Secure Enclave wrapper (macOS)
        let mut wrappers = vec![crypto::Wrapper::new(
            WrapperType::Argon2id,
            argon2id_wrapper,
        )?];

        #[cfg(target_os = "macos")]
        if let Ok(se) = secure_enclave::get_secure_enclave() {
            if let Ok(se_wrapper) = se.wrap_key(&vault_key) {
                wrappers.insert(0, se_wrapper); // Biometric first
            }
        }

        // Build header with canary
        let header = format::VaultHeader::new_with_canary(
            crypto::Ed25519PublicKey(vault_key.derive_verifying_key().to_bytes()),
            salt,
            params,
            wrappers,
            canary,
        )?;

        // Encrypt contents
        let plaintext =
            serde_json::to_vec(&contents).map_err(|e| VaultError::Serialization(e.to_string()))?;

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
    pub async fn unlock_biometric(
        &self,
        password_fallback: bool,
    ) -> Result<(UnlockedVault, UnlockResult), VaultError> {
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
        crypto::verify_vault_signature(
            &vault_file.header.ed25519_pk,
            &vault_file.header.signed_data(&vault_file.ciphertext),
            &vault_file.header.signature,
        )
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
        crypto::verify_vault_signature(
            &vault_file.header.ed25519_pk,
            &vault_file.header.signed_data(&vault_file.ciphertext),
            &vault_file.header.signature,
        )
        .map_err(|_| VaultError::Corrupted("signature verification failed".into()))?;

        // Check rate limiting before attempting password
        let now = now_ms();
        {
            let lockout = self
                .lockout
                .lock()
                .map_err(|_| VaultError::Other("lockout mutex poisoned".into()))?;
            lockout.check_lockout(now)?;
        }

        // Create guard that records failures on drop
        let mut guard = LockoutGuard::new(&self.lockout, &self.config.vault_path, now);

        // Find Argon2id wrapper
        let argon2id_wrapper = vault_file
            .header
            .get_wrapper(WrapperType::Argon2id)
            .ok_or_else(|| VaultError::Corrupted("no Argon2id wrapper found".into()))?;

        let params = &vault_file.header.argon2_params;
        let vault_key = crypto::unwrap_argon2id(
            &argon2id_wrapper.data,
            password,
            &vault_file.header.salt,
            params,
        )?;

        let unlocked = self.decrypt_and_load(vault_key, &vault_file)?;

        // Check rollback: ensure counter hasn't regressed
        crate::rollback::check_counter(
            &self.config.vault_path,
            unlocked.header.counter,
            unlocked.header.created_timestamp_ms,
        )?;

        guard.mark_success();
        Ok(unlocked)
    }

    /// Decrypt vault contents with key
    fn decrypt_and_load(
        &self,
        vault_key: crypto::VaultKey,
        vault_file: &format::VaultFile,
    ) -> Result<UnlockedVault, VaultError> {
        let mut plaintext =
            crypto::decrypt_vault(&vault_key, &vault_file.header.nonce, &vault_file.ciphertext)?;

        let contents: VaultContents = serde_json::from_slice(&plaintext).map_err(|e| {
            plaintext.zeroize();
            VaultError::Serialization(e.to_string())
        })?;

        // Zeroize the plaintext after parsing
        plaintext.zeroize();

        // Verify canary
        if !contents.verify_canary(&vault_file.header.canary) {
            return Err(VaultError::Corrupted(
                "canary mismatch - wrong password or corrupted".into(),
            ));
        }

        let header = vault_file.header.clone();
        let file_path = self.config.vault_path.clone();

        // Lock vault key in memory to prevent swapping (best-effort)
        let key_lock = crate::mlock::LockedMemory::new(vault_key.as_bytes()).unwrap_or_else(|e| {
            eprintln!("vault: mlock warning: {}", e);
            crate::mlock::LockedMemory::noop()
        });

        Ok(UnlockedVault {
            vault_key,
            _key_lock: key_lock,
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

        Err(VaultError::PlatformNotSupported(
            "No biometric hardware available".into(),
        ))
    }

    /// Change vault password
    pub async fn change_password(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), VaultError> {
        let mut vault = self.unlock_with_password(old_password)?;

        // Create new Argon2id wrapper with new password
        let salt = crypto::generate_salt();
        let params = self.config.argon2_params.unwrap_or_default();
        let argon2id_wrapper =
            crypto::wrap_argon2id(&vault.vault_key, new_password, &salt, &params)?;

        // Update header
        vault.header.salt = salt;
        vault.header.argon2_params = params;
        vault.header.replace_wrapper(crypto::Wrapper::new(
            WrapperType::Argon2id,
            argon2id_wrapper,
        )?)?;
        vault.header.key_version += 1;

        // Re-encrypt with new nonce
        let plaintext = serde_json::to_vec(&vault.contents)
            .map_err(|e| VaultError::Serialization(e.to_string()))?;
        let (ciphertext, nonce) = crypto::encrypt_vault(&vault.vault_key, &plaintext)?;
        vault.header.nonce = nonce;
        vault.header.signature =
            crypto::sign_vault(&vault.vault_key, &vault.header.signed_data(&ciphertext));

        // Securely overwrite old vault file before writing new one
        crypto::secure_overwrite(&vault.file_path).ok(); // best-effort, ignore errors

        // Write
        format::atomic_write_vault(&vault.file_path, &vault.header, &ciphertext)?;

        Ok(())
    }

    /// Delete vault file
    pub fn delete(&self) -> Result<(), VaultError> {
        if self.exists() {
            std::fs::remove_file(&self.config.vault_path).map_err(VaultError::Io)?;
        }
        Ok(())
    }

    /// Complete pending migration after unlock
    pub async fn complete_migration(&self, password: &str) -> Result<(), VaultError> {
        let migration_flag = self.config.vault_path.with_extension("bin.migrate");
        if !migration_flag.exists() {
            return Ok(());
        }

        let old_version = std::fs::read_to_string(&migration_flag)
            .map_err(VaultError::Io)?
            .parse::<u8>()
            .map_err(|e| VaultError::ParseError(format!("invalid migration flag: {e}")))?;

        // Unlock with old password
        let mut vault = self.unlock_with_password(password)?;

        // Update version
        vault.header.version = 2;

        // Re-save (this will re-encrypt with current format)
        vault.save()?;

        // Remove migration flag
        std::fs::remove_file(&migration_flag).map_err(VaultError::Io)?;

        eprintln!("vault: migration from v{old_version} to v2 complete");
        Ok(())
    }
}

/// Run upgrade/migration if needed
pub async fn migrate_if_needed(vault_path: &std::path::Path) -> Result<(), VaultError> {
    if !vault_path.exists() {
        return Ok(());
    }

    let vault_file = format::read_vault_file(vault_path)?;

    match vault_file.header.version {
        0 | 1 => {
            // Migrate v1 -> v2: Add canary field if missing
            // v1 vaults don't have the canary field, so we need to re-encrypt
            eprintln!("vault: migrating from v{} to v2", vault_file.header.version);

            // For now, we can't migrate without the password
            // The migration will happen on next unlock with password
            // Store a flag that migration is needed
            let migration_flag = vault_path.with_extension("bin.migrate");

            // Write with restrictive permissions
            use std::fs::OpenOptions;
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;

            #[allow(unused_mut)]
            let mut open_opts = OpenOptions::new();
            open_opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            open_opts.mode(0o600);

            let mut file = open_opts.open(&migration_flag).map_err(VaultError::Io)?;
            std::io::Write::write_all(
                &mut file,
                format!("{}", vault_file.header.version).as_bytes(),
            )
            .map_err(VaultError::Io)?;

            Ok(())
        }
        2 => Ok(()), // Current version, no migration needed
        v => Err(VaultError::UnsupportedVersion(v)),
    }
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
            argon2_params: Some(Argon2Params {
                t: 1,
                m_kib: 32768,
                p: 1,
            }),
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

        unlocked
            .set_password("server1:22".into(), SecretString::from("pass123"))
            .unwrap();
        assert_eq!(
            unlocked.get_password("server1:22").unwrap().expose_secret(),
            "pass123"
        );

        unlocked.lock();
        let unlocked2 = vault.unlock_with_password(password).unwrap();
        assert_eq!(
            unlocked2
                .get_password("server1:22")
                .unwrap()
                .expose_secret(),
            "pass123"
        );
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

        assert!(vault.unlock_with_password(old_pass).is_err());

        let unlocked = vault.unlock_with_password(new_pass).unwrap();
        assert!(unlocked.get_password("server1:22").is_none());
    }

    #[tokio::test]
    async fn test_rate_limiting_lockout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        let password = "correct-password";
        vault.initialize(password).await.unwrap();

        for _ in 0..3 {
            assert!(vault.unlock_with_password("wrong").is_err());
        }

        assert!(matches!(
            vault.unlock_with_password("wrong"),
            Err(VaultError::RateLimited(_))
        ));

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let unlocked = vault.unlock_with_password(password).unwrap();
        assert!(unlocked.get_password("test").is_none());

        let result = vault.unlock_with_password("wrong");
        assert!(result.is_err());
        assert!(!matches!(result, Err(VaultError::RateLimited(_))));
    }

    #[tokio::test]
    async fn test_vault_exists_and_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        assert!(!vault.exists());
        assert_eq!(vault.path(), &path);

        vault.initialize("password").await.unwrap();
        assert!(vault.exists());
    }

    #[tokio::test]
    async fn test_vault_initialize_already_exists() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();
        let result = vault.initialize("password").await;
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn test_vault_unlock_wrong_password() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("correct-password").await.unwrap();
        let result = vault.unlock_with_password("wrong-password");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_vault_unlock_biometric_fallback() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        // Biometric will fail, should fall back to password prompt
        // Since we can't mock stdin, this will fail with IO error
        let result = vault.unlock_biometric(true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_vault_unlock_biometric_no_fallback() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        // Biometric will fail, no fallback
        let result = vault.unlock_biometric(false).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::BiometricFailed)));
    }

    #[tokio::test]
    async fn test_vault_get_unlocked() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        // get_unlocked will try biometric, then fall back to password prompt
        // Since we can't mock stdin, this will fail
        let result = vault.get_unlocked().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_vault_lock() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        // Lock should work even when nothing is unlocked
        vault.lock().await;
    }

    #[tokio::test]
    async fn test_vault_delete() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();
        assert!(vault.exists());

        vault.delete().unwrap();
        assert!(!vault.exists());
    }

    #[tokio::test]
    async fn test_vault_delete_nonexistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        // Should not error
        vault.delete().unwrap();
    }

    #[tokio::test]
    async fn test_unlocked_vault_remove_password() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();
        let mut unlocked = vault.unlock_with_password("password").unwrap();

        unlocked
            .set_password("server1:22".into(), SecretString::from("pass1"))
            .unwrap();
        assert!(unlocked.get_password("server1:22").is_some());

        let removed = unlocked.remove_password("server1:22").unwrap();
        assert!(removed);
        assert!(unlocked.get_password("server1:22").is_none());
    }

    #[tokio::test]
    async fn test_unlocked_vault_remove_nonexistent_password() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();
        let mut unlocked = vault.unlock_with_password("password").unwrap();

        let removed = unlocked.remove_password("nonexistent").unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_unlocked_vault_hosts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();
        let mut unlocked = vault.unlock_with_password("password").unwrap();

        assert!(unlocked.hosts().is_empty());

        unlocked
            .set_password("server1:22".into(), SecretString::from("pass1"))
            .unwrap();
        unlocked
            .set_password("server2:22".into(), SecretString::from("pass2"))
            .unwrap();

        let mut hosts = unlocked.hosts();
        hosts.sort();
        assert_eq!(hosts, vec!["server1:22", "server2:22"]);
    }

    #[tokio::test]
    async fn test_unlocked_vault_persists_after_save() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        {
            let mut unlocked = vault.unlock_with_password("password").unwrap();
            unlocked
                .set_password("server1:22".into(), SecretString::from("pass1"))
                .unwrap();
        }

        // Unlock again and check password persisted
        let unlocked = vault.unlock_with_password("password").unwrap();
        assert_eq!(
            unlocked.get_password("server1:22").unwrap().expose_secret(),
            "pass1"
        );
    }

    #[tokio::test]
    async fn test_vault_multiple_servers() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();
        let mut unlocked = vault.unlock_with_password("password").unwrap();

        // Add multiple server passwords
        for i in 0..10 {
            let host = format!("server{i}:22");
            let pass = format!("pass{i}");
            unlocked
                .set_password(host.into(), SecretString::from(pass.as_str()))
                .unwrap();
        }

        // Verify all passwords
        for i in 0..10 {
            let host = format!("server{i}:22");
            let pass = format!("pass{i}");
            assert_eq!(unlocked.get_password(&host).unwrap().expose_secret(), pass);
        }

        assert_eq!(unlocked.hosts().len(), 10);
    }

    #[tokio::test]
    async fn test_vault_concurrent_access() {
        use std::sync::Arc;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Arc::new(Vault::new(config));

        vault.initialize("password").await.unwrap();

        let mut handles = vec![];
        for i in 0..5 {
            let vault = vault.clone();
            handles.push(tokio::spawn(async move {
                let mut unlocked = vault.unlock_with_password("password").unwrap();
                let host = format!("server{i}:22");
                let pass = format!("pass{i}");
                unlocked
                    .set_password(host.into(), SecretString::from(pass.as_str()))
                    .unwrap();
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all passwords were saved (last writer wins for each host)
        let unlocked = vault.unlock_with_password("password").unwrap();
        assert_eq!(unlocked.hosts().len(), 5);
    }

    #[tokio::test]
    async fn test_vault_migration_flag() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        // Simulate migration needed by creating flag file
        let migration_flag = path.with_extension("bin.migrate");
        std::fs::write(&migration_flag, "1").unwrap();
        assert!(migration_flag.exists());

        // Complete migration
        vault.complete_migration("password").await.unwrap();
        assert!(!migration_flag.exists());
    }

    #[tokio::test]
    async fn test_vault_migration_not_needed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());
        let vault = Vault::new(config);

        vault.initialize("password").await.unwrap();

        // No migration flag
        let result = vault.complete_migration("password").await;
        assert!(result.is_ok());
    }
}
