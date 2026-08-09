//! Opening a vault: the biometric path, the password path, and the decryption
//! both of them end in.

use crate::crypto::{self, WrapperType};
use crate::format;
use crate::lockout::LockoutGuard;
use crate::{VaultContents, VaultError};
use zeroize::Zeroize;

use super::{UnlockedVault, Vault};

impl Vault {
    /// Whether this vault can be opened with one touch on this machine.
    ///
    /// Three things have to be true at once, and any of them can be false on a
    /// machine that has a working Touch ID sensor:
    ///
    /// - the platform offers biometrics at all (Linux does not, yet -- see the
    ///   TPM2 note in the roadmap: `fprintd` can verify a finger but cannot
    ///   release a key, so it cannot unlock anything);
    /// - the vault carries a wrapper bound to *this* machine's enclave key,
    ///   which a vault created elsewhere or before the key was rebound does
    ///   not;
    /// - the config permits the OS keychain, which a test vault does not.
    ///
    /// Anything less and the caller asks for the master password instead. One
    /// prompt either way, never both -- which is the whole point: this is
    /// asked *before* a prompt goes up, so the user is never shown a biometric
    /// wait that was always going to fall through to typing.
    ///
    /// Reading the header rather than remembering the answer: the vault file
    /// can be replaced under a running program, and a cached "yes" would put up
    /// a Touch ID prompt for a wrapper that is no longer there.
    #[must_use]
    pub fn biometric_available(&self) -> bool {
        if !self.config.use_os_keychain || !crate::secure_enclave::is_available() {
            return false;
        }
        format::read_vault_file(&self.config.vault_path)
            .is_ok_and(|f| f.header.has_wrapper(WrapperType::SecureEnclave))
    }

    /// Unlock vault with password
    ///
    /// # Errors
    /// Returns `VaultError::Corrupted` if signature verification fails,
    /// `VaultError::LockedOut` if the vault is rate-limited,
    /// `VaultError::Argon2Error` if key unwrapping fails,
    /// `VaultError::DecryptionFailed` if decryption fails,
    /// `VaultError::Serialization` if contents cannot be parsed,
    /// `VaultError::Corrupted` if canary verification fails,
    /// or `VaultError::RollbackDetected` if rollback is detected.
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
        self.ensure_lockout_loaded()?;
        let now = (self.clock)();
        {
            let lockout = self
                .lockout
                .lock()
                .map_err(|_| VaultError::Other("lockout mutex poisoned".into()))?;
            lockout.check_lockout(now)?;
        }

        // Count this attempt BEFORE the KDF runs, and persist it now. If the
        // process dies at any point after this -- including a SIGKILL aimed at
        // dodging the limiter -- the attempt still stands.
        {
            let mut lockout = self
                .lockout
                .lock()
                .map_err(|_| VaultError::Other("lockout mutex poisoned".into()))?;
            lockout.on_attempt(&self.config.vault_path, now);
        }

        // The guard finalises the attempt: it anchors the backoff deadline on
        // failure, or clears the counter on success.
        let mut guard =
            LockoutGuard::with_clock(&self.lockout, &self.config.vault_path, self.clock);

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

        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut unlocked = self.decrypt_and_load(vault_key, &vault_file)?;

        // The password was correct: decryption and the canary both passed.
        // Mark success BEFORE the rollback check, because a rollback is not a
        // failed authentication attempt. Recording it as one fed the
        // exponential backoff and the ten-attempt hard lockout, so restoring a
        // backup locked the user out while telling them they were being rate
        // limited for guessing.
        guard.mark_success();

        // Check rollback: ensure counter hasn't regressed.
        crate::rollback::check_counter(
            &self.config.vault_path,
            unlocked.header.counter,
            unlocked.header.created_timestamp_ms,
            self.config.use_os_keychain,
        )?;

        // Repairing an orphaned Secure Enclave wrapper needs exactly the
        // authorisation just presented -- the master password -- so it happens
        // here rather than waiting for a prompt that does not exist. The repair
        // itself lives with the rest of the enclave code.
        self.rebind_enclave_wrapper(&mut unlocked);

        Ok(unlocked)
    }

    /// Decrypt vault contents with key
    ///
    /// # Errors
    /// Returns `VaultError::DecryptionFailed` if decryption fails,
    /// `VaultError::Serialization` if contents cannot be parsed,
    /// or `VaultError::Corrupted` if canary verification fails.
    pub(super) fn decrypt_and_load(
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
        let key_lock = match crate::mlock::LockedMemory::new(vault_key.as_bytes()) {
            Ok(lock) => lock,
            Err(_e) => {
                // Best-effort only, and silent: see the note above about stderr
                // and the TUI. The vault works without the memory lock.
                crate::mlock::LockedMemory::noop()
            }
        };

        Ok(UnlockedVault {
            vault_key,
            _key_lock: key_lock,
            contents,
            header,
            file_path,
            use_os_keychain: self.config.use_os_keychain,
        })
    }
}
