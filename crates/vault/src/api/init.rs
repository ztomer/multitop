//! Creating a vault.

use crate::crypto::{self, WrapperType};
use crate::format;
#[cfg(target_os = "macos")]
use crate::secure_enclave;
use crate::{VaultContents, VaultError};
use std::path::PathBuf;

use super::Vault;

impl Vault {
    /// Check if vault file exists
    pub fn exists(&self) -> bool {
        self.config.vault_path.exists()
    }

    /// Get vault path
    pub const fn path(&self) -> &PathBuf {
        &self.config.vault_path
    }

    /// Initialize a new vault with the system password
    ///
    /// # Errors
    /// Returns `VaultError::AlreadyExists` if the vault already exists,
    /// `VaultError::Io` if directory creation or file writing fails,
    /// `VaultError::Serialization` if contents cannot be serialized,
    /// or encryption-related errors.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
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
        // `mut` only on the platform that pushes a second wrapper below.
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut wrappers = vec![crypto::Wrapper::new(
            WrapperType::Argon2id,
            argon2id_wrapper,
        )?];

        // The Secure Enclave key lives in the login keychain, so it is exactly
        // the "real credential storage" that `use_os_keychain` exists to keep
        // tests away from. It was not gated: every test that initialised a
        // vault on macOS ran `generate_new`, which begins by calling
        // `delete_existing` -- so running the suite deleted the developer's
        // actual Secure Enclave key and orphaned the wrapper in their real
        // vault, permanently disabling biometric unlock.
        #[cfg(target_os = "macos")]
        if self.config.use_os_keychain {
            if let Ok(se) = secure_enclave::get_secure_enclave() {
                if let Ok(se_wrapper) = se.wrap_key(&vault_key) {
                    wrappers.insert(0, se_wrapper); // Biometric first
                }
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
}
