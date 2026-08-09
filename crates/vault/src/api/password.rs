//! Changing what opens the vault: re-binding the enclave wrapper, and
//! replacing the master password.

use crate::crypto::{self, WrapperType};
#[cfg(target_os = "macos")]
use crate::secure_enclave;
use crate::VaultError;

use super::Vault;

impl Vault {
    /// Add a biometric wrapper to existing vault (re-bind)
    ///
    /// # Errors
    /// Returns `VaultError` if unlock fails, Secure Enclave is unavailable,
    /// or saving the vault fails.
    // `unused_async_trait_impl` exists on nightly and not yet on stable, and CI
    // runs stable: without `unknown_lints` the *name* is an error there, and
    // without the allow the lint itself is an error here. Both are needed until
    // it lands on stable, at which point the `unknown_lints` line can go.
    #[allow(unknown_lints)]
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn rebind_biometric(&self, password: &str) -> Result<(), VaultError> {
        // Refuse before the unlock, not after it. `unlock_with_password` runs
        // Argon2id at a quarter of system RAM; on a platform with no enclave to
        // rebind to, that is seconds of work spent to reach a refusal that was
        // certain from the first line.
        #[cfg(target_os = "linux")]
        {
            // A vault made before this machine had a TPM, or before this
            // feature existed, has only the password wrapper. Without this the
            // only way to gain one is to recreate the vault, which means
            // re-entering every credential in it.
            if !self.config.use_os_keychain || !crate::tpm2::is_available() {
                return Err(VaultError::PlatformNotSupported(
                    "No TPM available to seal to".into(),
                ));
            }
            let mut vault = self.unlock_with_password(password)?;
            let sealed = crate::tpm2::seal(&vault.vault_key)?;
            // `add_wrapper`, not `replace`: an existing TPM2 wrapper on this
            // vault was sealed by this same TPM and still opens it, and
            // replacing it would be a write with nothing gained.
            if vault.header.get_wrapper(WrapperType::Tpm2).is_none() {
                vault
                    .header
                    .add_wrapper(crypto::Wrapper::new(WrapperType::Tpm2, sealed)?)?;
                vault.save()?;
            }
            Ok(())
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = password;
            Err(VaultError::PlatformNotSupported(
                "No biometric hardware available".into(),
            ))
        }

        #[cfg(target_os = "macos")]
        {
            let mut vault = self.unlock_with_password(password)?;

            // Gated for the same reason as `initialize`: creating the Secure Enclave
            // key writes to the login keychain and deletes any existing key first.
            #[cfg(target_os = "macos")]
            if self.config.use_os_keychain {
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
    }

    /// Change vault password
    ///
    /// # Errors
    /// Returns `VaultError` if old password is wrong, new password encryption fails,
    /// or writing the vault fails.
    /// Synchronous on purpose: nothing here awaits. It was `async` while
    /// awaiting nothing, which reads as "this yields" and forces a caller on a
    /// blocking thread to drive a future for no reason. It does run Argon2id
    /// twice, so callers should still keep it off an event loop.
    pub fn change_password(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), VaultError> {
        let mut vault = self.unlock_with_password(old_password)?;

        // Only the wrapper changes. The vault key itself is untouched: the new
        // password wraps the same key, which is why any Secure Enclave wrapper
        // in the header stays valid and biometric unlock keeps working across a
        // rotation. Rotating the key would mean re-binding the enclave in the
        // same operation or silently breaking Touch ID.
        let salt = crypto::generate_salt();
        let params = self.config.argon2_params.unwrap_or_default();
        let argon2id_wrapper =
            crypto::wrap_argon2id(&vault.vault_key, new_password, &salt, &params)?;

        vault.header.salt = salt;
        vault.header.argon2_params = params;
        vault.header.replace_wrapper(crypto::Wrapper::new(
            WrapperType::Argon2id,
            argon2id_wrapper,
        )?)?;
        vault.header.key_version += 1;

        // Written through `save`, which re-encrypts with a fresh nonce, signs,
        // writes atomically, and advances the rollback counter.
        //
        // What used to be here re-implemented all of that except the counter,
        // and called `secure_overwrite` on the vault *before* writing the
        // replacement. That filled the file with random bytes in place and then
        // wrote the new one, so anything failing in between -- a full disk, a
        // crash, a power cut -- left the old vault shredded and the new one
        // never written, with every stored password gone and nothing to restore
        // from. Atomic write exists precisely to remove that window.
        //
        // Erasing the pre-rotation ciphertext is not attempted. `atomic_write_vault`
        // renames a new file over the old one, so the previous blocks are
        // unlinked rather than overwritten, and on a copy-on-write filesystem an
        // in-place overwrite does not reach them either -- as `secure_overwrite`
        // documents about itself. Full-disk encryption is the real mitigation.
        vault.save()?;

        Ok(())
    }
}
