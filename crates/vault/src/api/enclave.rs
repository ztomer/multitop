//! The Secure Enclave paths: unlocking by touch, and repairing the wrapper
//! that makes it possible.
//!
//! # Why this is a file of its own
//!
//! None of it can run without Secure Enclave hardware holding a key bound to
//! *this* machine's enrolled biometric set, so no test can execute it and no
//! coverage number can describe it. Kept beside `unlock.rs` it dragged that
//! file to two thirds covered and hid which of the *password* paths were
//! genuinely untested. `secure_enclave.rs` and `fprintd.rs` are excluded from
//! the coverage gate for exactly this reason, with the reason written down;
//! this is the third file in that category and it is named there too.
//!
//! Everything here is held to the other bar instead: it is best-effort, it can
//! only ever replace a wrapper that is already present, and every failure
//! leaves the password path untouched. What *is* testable -- the decision of
//! which door to offer the user -- is `Vault::biometric_available`, and it
//! stays in `unlock.rs` where the gate measures it.

use crate::crypto::{self, WrapperType};
use crate::format;
// Only used by the Linux arm below. The enclave split lost this import: it is
// invisible on macOS, where the arm that needs it is compiled out, so nothing
// local could notice and CI was red for an unrelated reason.
#[cfg(target_os = "linux")]
use crate::fprintd;
#[cfg(target_os = "macos")]
use crate::secure_enclave;
use crate::VaultError;

#[cfg(target_os = "macos")]
use super::biometric::{should_rebind_biometric, Biometrics, EnclaveKey};
use super::{UnlockResult, UnlockedVault, Vault};

impl Vault {
    /// Unlock vault with biometric (Touch ID / fingerprint).
    ///
    /// There is no password fallback here by design -- see the body. The caller
    /// owns the terminal and therefore owns any password prompt.
    ///
    /// # Errors
    /// Returns `VaultError::BiometricFailed` if biometric is unavailable or the
    /// verification does not succeed.
    pub async fn unlock_biometric(&self) -> Result<(UnlockedVault, UnlockResult), VaultError> {
        // Try biometric first
        if let Ok(vault) = self.try_unlock_biometric().await {
            return Ok((vault, UnlockResult::Biometric));
        }

        // Biometric failed or unavailable.
        //
        // This deliberately does NOT prompt for a password. It used to call
        // `rpassword::prompt_password`, a blocking stdin read -- from a library
        // whose only caller is a full-screen TUI holding the terminal in raw
        // mode on the alternate screen. That read cannot be seen, cannot be
        // typed into, and blocks the caller forever. The caller owns the
        // terminal, so the caller owns the prompt: it asks the user and calls
        // `unlock_with_password` itself.
        Err(VaultError::BiometricFailed)
    }

    /// Try to unlock with biometric only (no password fallback)
    ///
    /// # Errors
    /// Returns `VaultError::BiometricFailed` if no biometric is available,
    /// the biometric verification fails, or the Secure Enclave key is invalidated.
    /// # Rate limiting does not apply here, by design
    ///
    /// `unlock_with_password` is rate limited; this is not. A failed Touch ID
    /// or fingerprint is not a password guess, and counting it would push a
    /// user toward the lockout backoff simply because their sensor was
    /// unavailable. `test_vault_biometric_failures_do_not_trigger_lockout`
    /// pins this.
    // `unused_async_trait_impl` exists on nightly and not yet on stable, and CI
    // runs stable: without `unknown_lints` the *name* is an error there, and
    // without the allow the lint itself is an error here. Both are needed until
    // it lands on stable, at which point the `unknown_lints` line can go.
    #[allow(unknown_lints)]
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
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
        if self.config.use_os_keychain {
            if let Some(se_wrapper) = vault_file.header.get_wrapper(WrapperType::SecureEnclave) {
                // Load-only, never create. `get_secure_enclave` falls through to
                // generating a fresh key pair, and generation deletes the
                // existing one -- so a transient lookup failure here would
                // destroy the private key that this very wrapper was encrypted
                // to, orphaning it forever. Unlocking must not be able to
                // damage what it is reading.
                if let Ok(se) = secure_enclave::get_secure_enclave_existing() {
                    match se.unwrap_key(se_wrapper) {
                        Ok(vault_key) => {
                            let unlocked = self.decrypt_and_load(vault_key, &vault_file)?;
                            // Rollback detection has to happen on this path too.
                            // It used to live only in `unlock_with_password`, so
                            // restoring an old vault file -- reverting a changed
                            // password, reinstating a revoked credential -- was
                            // refused when the user typed their password and
                            // accepted in silence when they used Touch ID. The
                            // more convenient unlock skipped the defence.
                            //
                            // It cannot simply move into `decrypt_and_load`,
                            // which both paths share: that runs before
                            // `guard.mark_success()`, so a rollback would once
                            // again be recorded as a failed authentication
                            // attempt and feed the lockout backoff. There is no
                            // lockout guard on this path, so the check goes
                            // here.
                            crate::rollback::check_counter(
                                &self.config.vault_path,
                                unlocked.header.counter,
                                unlocked.header.created_timestamp_ms,
                                self.config.use_os_keychain,
                            )?;
                            return Ok(unlocked);
                        }
                        Err(VaultError::BiometricFailed) => {
                            // Fall through to password
                        }
                        Err(_e) => {
                            // Key invalidated (macOS update, etc.) - fall
                            // through to the caller's password path.
                            // Deliberately silent: this library is used by a
                            // full-screen TUI, and a stray stderr write lands
                            // on top of the rendered UI.
                        }
                    }
                }
            }
        }

        // Try fprintd, then the TPM (Linux).
        //
        // Only ask for a fingerprint if there is a wrapper a fingerprint can
        // actually release. Unlike the Secure Enclave, `fprintd` holds no key
        // material -- it answers yes or no, and the key comes from the TPM2
        // wrapper. Asking without one is the ceremony this used to perform: a
        // prompt, the verifier's own timeout, and then a refusal no matter what
        // the user did.
        //
        // The order matters, and not for security. The TPM will unseal whether
        // or not a finger was presented -- it cannot know, which is the whole
        // caveat in `tpm2` -- so the fingerprint is asked first because it is
        // what the *user* was promised, and a refusal must stop the unlock even
        // though nothing cryptographic enforces it.
        #[cfg(target_os = "linux")]
        if let Some(tpm_wrapper) = vault_file.header.get_wrapper(WrapperType::Tpm2) {
            if let Ok(fv) = fprintd::FingerprintVerifier::new().await {
                match fv.verify().await {
                    Ok(crate::fprintd::FingerprintResult::Verified) => {
                        // The finger was accepted, so release the key.
                        let vault_key = crate::tpm2::unseal(&tpm_wrapper.data)?;
                        let unlocked = self.decrypt_and_load(vault_key, &vault_file)?;
                        // Rollback detection belongs on every path that opens a
                        // vault, not only the password one -- restoring an old
                        // file is how a deleted credential comes back.
                        crate::rollback::check_counter(
                            &self.config.vault_path,
                            unlocked.header.counter,
                            unlocked.header.created_timestamp_ms,
                            self.config.use_os_keychain,
                        )?;
                        return Ok(unlocked);
                    }
                    Ok(
                        crate::fprintd::FingerprintResult::Failed
                        | crate::fprintd::FingerprintResult::Timeout,
                    ) => {
                        return Err(VaultError::BiometricFailed);
                    }
                    // No reader, no enrolled finger, or the daemon went away:
                    // fall through to the caller's password prompt rather than
                    // unsealing, which would open the vault with no check at
                    // all in front of it.
                    _ => {}
                }
            }
        }

        Err(VaultError::BiometricFailed)
    }

    /// Repair an orphaned Secure Enclave wrapper, if there is one and it can be
    /// repaired. Best-effort and silent: every failure leaves the vault exactly
    /// as it opened.
    #[allow(
        unused_variables,
        clippy::unused_self,
        clippy::needless_pass_by_ref_mut,
        clippy::missing_const_for_fn,
        reason = "the body is macOS-only; on every other platform this is empty \
                  and the parameters are untouched"
    )]
    pub(super) fn rebind_enclave_wrapper(&self, unlocked: &mut UnlockedVault) {
        // Repair an orphaned Secure Enclave wrapper.
        //
        // `kSecAccessControlBiometryCurrentSet` is the right access control to
        // have chosen, but it means the enclave key is invalidated whenever the
        // enrolled biometric set changes -- adding one fingerprint is enough.
        // The key can also simply be absent. Either way the wrapper still in the
        // file is encrypted to a private key that no longer exists, so biometric
        // unlock is dead, and it stays dead: `rebind_biometric` is the only cure
        // and no UI path reaches it. The user is asked for their password
        // forever with nothing saying why, which reads as the feature quietly
        // breaking rather than as something recoverable.
        //
        // Re-binding requires exactly the authorisation just presented a few
        // lines above -- the master password -- so the repair happens here
        // instead of waiting for a prompt that does not exist. It is
        // deliberately narrow: it only ever *replaces* a wrapper that is already
        // present. It never adds biometric unlock to a vault that had none,
        // because enabling it is the user's decision, not a repair.
        //
        // Best-effort and silent. If any step fails the password path is
        // untouched and the vault still opens.
        #[cfg(target_os = "macos")]
        {
            let keychain_allowed = self.config.use_os_keychain;
            let has_se_wrapper =
                keychain_allowed && unlocked.header.has_wrapper(WrapperType::SecureEnclave);
            // Each check is guarded by the previous one so the expensive ones --
            // a keychain search, then a `bioutil` subprocess -- never run on an
            // ordinary unlock.
            let key = if has_se_wrapper && secure_enclave::get_secure_enclave_existing().is_err() {
                EnclaveKey::Missing
            } else {
                EnclaveKey::Loads
            };
            let biometrics = if matches!(key, EnclaveKey::Missing) && secure_enclave::is_available()
            {
                Biometrics::Available
            } else {
                Biometrics::Absent
            };

            if should_rebind_biometric(keychain_allowed, has_se_wrapper, key, biometrics) {
                if let Ok(se) = secure_enclave::get_secure_enclave() {
                    if let Ok(wrapper) = se.wrap_key(&unlocked.vault_key) {
                        if unlocked.header.replace_wrapper(wrapper).is_ok() {
                            let _ = unlocked.save();
                        }
                    }
                }
            }
        }
    }
}
