//! The open vault: the handle a caller reads and writes passwords through.
//!
//! Separate from the `Vault` that produced it because the two have different
//! lifetimes and different hazards. This one holds the vault key in memory,
//! which is why it locks that memory, redacts itself in `Debug`, and zeroizes
//! on drop.

use crate::crypto::{self, now_ms};
use crate::format;
use crate::{VaultContents, VaultError};
use secrecy::SecretString;
use std::path::PathBuf;
use zeroize::Zeroize;

/// Result of vault unlock
#[derive(Debug)]
pub enum UnlockResult {
    Biometric,
    Password,
}

/// In-memory unlocked vault
pub struct UnlockedVault {
    // `pub(super)` rather than private: the unlock, rotation and re-bind paths
    // are siblings inside `api`, and each of them has to reach the key or the
    // header it just produced. Nothing outside `api` can see these.
    pub(super) vault_key: crate::crypto::VaultKey,
    /// mlock the vault key to prevent swapping
    pub(super) _key_lock: crate::mlock::LockedMemory,
    pub(super) contents: VaultContents,
    pub(super) header: crate::format::VaultHeader,
    pub(super) file_path: PathBuf,
    /// Carried from `VaultConfig` so a save cannot write a rollback counter to
    /// the OS keychain that the matching read would skip.
    pub(super) use_os_keychain: bool,
}

impl std::fmt::Debug for UnlockedVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately redacted: never print the vault key or stored secrets.
        f.debug_struct("UnlockedVault")
            .field("hosts", &self.contents.hosts())
            .field("file_path", &self.file_path)
            .finish_non_exhaustive()
    }
}

impl Drop for UnlockedVault {
    fn drop(&mut self) {
        self.vault_key.zeroize();
    }
}

impl UnlockedVault {
    /// Get password for a host
    #[must_use]
    pub fn get_password(&self, host: &str) -> Option<SecretString> {
        self.contents.get(host)
    }

    /// Set password for a host
    ///
    /// # Errors
    /// Returns `VaultError::Io` if saving the vault fails,
    /// `VaultError::Serialization` if the contents cannot be serialized,
    /// or other encryption-related errors.
    pub fn set_password(
        &mut self,
        host: String,
        password: &SecretString,
    ) -> Result<(), VaultError> {
        self.contents.set(host, password);
        self.save()?;
        Ok(())
    }

    /// Remove password for a host
    ///
    /// # Errors
    /// Returns `VaultError::Io` if saving the vault fails,
    /// or other encryption-related errors.
    pub fn remove_password(&mut self, host: &str) -> Result<bool, VaultError> {
        let removed = self.contents.remove(host);
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// List all hosts with passwords
    #[must_use]
    pub fn hosts(&self) -> Vec<String> {
        self.contents.hosts()
    }

    /// Save vault to disk (encrypts and writes atomically)
    ///
    /// # Errors
    /// Returns `VaultError::Io` if writing the vault file fails,
    /// `VaultError::Serialization` if contents cannot be serialized,
    /// or encryption-related errors.
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
            self.use_os_keychain,
        );

        Ok(())
    }

    /// Lock the vault (zeroize sensitive data)
    pub fn lock(mut self) {
        self.vault_key.zeroize();
        // VaultContents cleared on drop
    }
}
