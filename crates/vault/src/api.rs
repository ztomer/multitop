//! High-level Vault API

use crate::crypto::{self, now_ms, WrapperType};
use crate::format;
use crate::lockout::{LockoutGuard, LockoutState};
use crate::secure_enclave;
use crate::{VaultConfig, VaultContents, VaultError};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
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
    /// Carried from `VaultConfig` so a save cannot write a rollback counter to
    /// the OS keychain that the matching read would skip.
    use_os_keychain: bool,
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

/// Vault manager
pub struct Vault {
    config: VaultConfig,
    lockout: StdMutex<LockoutState>,
    /// Serialises the one-time load of `lockout`, so that a second caller
    /// blocks until the first has finished reading rather than racing past a
    /// flag and checking an empty limiter.
    lockout_init: StdMutex<()>,
    /// Whether `lockout` has been read from the credential store yet.
    ///
    /// Loading it eagerly meant constructing a `Vault` hit the OS keychain,
    /// which happens at app startup -- exactly the launch-time credential
    /// dialog the caller defers passwords to avoid. It is loaded on the first
    /// attempt that actually needs it instead.
    lockout_loaded: std::sync::atomic::AtomicBool,
    /// Clock used for rate-limiting decisions. Injectable so that lockout tests
    /// are deterministic: a real unlock attempt costs an Argon2 KDF plus a
    /// keychain write, which together can exceed the shortest backoff tier, so
    /// a wall-clock test races against its own setup cost.
    clock: fn() -> u64,
}

/// Decide whether a Secure Enclave wrapper is orphaned and should be re-bound
/// after a successful password unlock.
///
/// Kept as a pure function because the *rule* is the part that can be got
/// wrong, and it can be pinned by tests without a real enclave. Two of these
/// conditions are load-bearing in a way that is easy to lose:
///
/// - `has_se_wrapper` must be required, so a repair never becomes an enrolment.
///   Turning biometric unlock on for a vault that never had it is the user's
///   decision; silently adding a wrapper because the hardware happens to exist
///   would make that decision for them.
/// - `keychain_allowed` must be required, so the test suite -- which runs with
///   `use_os_keychain: false` precisely to stay off real credential storage --
///   can never generate an enclave key, and so can never delete the real one.
#[cfg(target_os = "macos")]
#[must_use]
pub(crate) const fn should_rebind_biometric(
    keychain_allowed: bool,
    has_se_wrapper: bool,
    key: EnclaveKey,
    biometrics: Biometrics,
) -> bool {
    keychain_allowed
        && has_se_wrapper
        && matches!(key, EnclaveKey::Missing)
        && matches!(biometrics, Biometrics::Available)
}

/// Whether the Secure Enclave private key backing an existing wrapper still
/// loads. `Missing` covers both an absent key and one invalidated by a change
/// to the enrolled biometric set.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnclaveKey {
    Loads,
    Missing,
}

/// Whether this machine has enrolled biometrics to bind a new key to.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Biometrics {
    Available,
    Absent,
}

impl Vault {
    /// Create new vault manager
    #[must_use]
    pub fn new(config: VaultConfig) -> Self {
        Self::with_clock(config, now_ms)
    }

    /// `new` with an injected clock, for tests that drive lockout timing.
    #[must_use]
    pub fn with_clock(config: VaultConfig, clock: fn() -> u64) -> Self {
        let use_keychain = config.use_os_keychain;
        Self {
            config,
            // Carries the policy from the start, so it is never briefly wrong.
            lockout: StdMutex::new(LockoutState::new(use_keychain)),
            lockout_init: StdMutex::new(()),
            lockout_loaded: std::sync::atomic::AtomicBool::new(false),
            clock,
        }
    }

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

        // Try fprintd (Linux).
        //
        // Only ask for a fingerprint if there is a wrapper a fingerprint can
        // actually release. Unlike the Secure Enclave, fprintd does not hold key
        // material -- it returns a yes or a no, and the key would have to come
        // from the TPM2 wrapper. Nothing in this codebase creates a TPM2
        // wrapper, so `has_wrapper(Tpm2)` is always false.
        //
        // Previously the verifier ran in the `else` arm, which is the arm that
        // is always taken: a Linux user was prompted to present a fingerprint,
        // waited up to thirty seconds, and then reached the
        // `Err(BiometricFailed)` below no matter what happened -- succeeding,
        // failing, and timing out were indistinguishable. That failed closed,
        // which is the right direction, but the prompt was pure ceremony and it
        // delayed the password fallback by half a minute.
        #[cfg(target_os = "linux")]
        if vault_file.header.has_wrapper(WrapperType::Tpm2) {
            if let Ok(fv) = fprintd::FingerprintVerifier::new().await {
                match fv.verify(30).await {
                    Ok(crate::fprintd::FingerprintResult::Verified) => {
                        // TPM2 unwrapping would go here once it exists. Until
                        // then a verified fingerprint still releases nothing, so
                        // this falls through rather than claiming success.
                    }
                    Ok(
                        crate::fprintd::FingerprintResult::Failed
                        | crate::fprintd::FingerprintResult::Timeout,
                    ) => {
                        return Err(VaultError::BiometricFailed);
                    }
                    _ => {}
                }
            }
        }

        Err(VaultError::BiometricFailed)
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

        Ok(unlocked)
    }

    /// Read the persisted lockout state, once, on first use.
    ///
    /// # Errors
    /// Returns `VaultError::Other` if the lockout mutex is poisoned.
    fn ensure_lockout_loaded(&self) -> Result<(), VaultError> {
        use std::sync::atomic::Ordering;
        // The init lock is held ACROSS the load, deliberately.
        //
        // An earlier version flipped the flag with `swap(true)` and loaded
        // afterwards. A second caller arriving in that window saw the flag
        // already set, returned immediately, and checked the limiter against
        // `LockoutState::default()` -- zero attempts, no deadline, the rate
        // limiter simply absent. Two concurrent unlocks are reachable: the UI
        // can spawn one, have it cancelled, and spawn another.
        let _init = self
            .lockout_init
            .lock()
            .map_err(|_| VaultError::Other("lockout init mutex poisoned".into()))?;
        if self.lockout_loaded.load(Ordering::SeqCst) {
            return Ok(());
        }
        let loaded = LockoutState::load(&self.config.vault_path, self.config.use_os_keychain);
        {
            let mut lockout = self
                .lockout
                .lock()
                .map_err(|_| VaultError::Other("lockout mutex poisoned".into()))?;
            *lockout = loaded;
        }
        self.lockout_loaded.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Decrypt vault contents with key
    ///
    /// # Errors
    /// Returns `VaultError::DecryptionFailed` if decryption fails,
    /// `VaultError::Serialization` if contents cannot be parsed,
    /// or `VaultError::Corrupted` if canary verification fails.
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

    /// Add a biometric wrapper to existing vault (re-bind)
    ///
    /// # Errors
    /// Returns `VaultError` if unlock fails, Secure Enclave is unavailable,
    /// or saving the vault fails.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn rebind_biometric(&self, password: &str) -> Result<(), VaultError> {
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

    /// Delete vault file
    ///
    /// # Errors
    /// Returns `VaultError::Io` if the vault file cannot be deleted.
    pub fn delete(&self) -> Result<(), VaultError> {
        if self.exists() {
            std::fs::remove_file(&self.config.vault_path).map_err(VaultError::Io)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::crypto::Argon2Params;
    use secrecy::ExposeSecret;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Biometric re-bind decision (macOS-only, like the code path it guards)
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn rebinds_only_when_an_existing_wrapper_is_orphaned() {
        // The one case that should repair: a wrapper is present, its enclave key
        // is gone, and the hardware is there to make a new one.
        assert!(should_rebind_biometric(
            true,
            true,
            EnclaveKey::Missing,
            Biometrics::Available
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn never_rebinds_without_an_existing_wrapper() {
        // A repair must not become an enrolment. Enabling biometric unlock for a
        // vault that never had it is the user's decision.
        assert!(
            !should_rebind_biometric(true, false, EnclaveKey::Missing, Biometrics::Available),
            "no wrapper present: adding one would enable biometric unlock the user never chose"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn never_rebinds_when_keychain_access_is_withheld() {
        // This is what keeps the test suite from generating an enclave key --
        // and therefore from deleting the developer's real one, since
        // generation deletes first.
        assert!(
            !should_rebind_biometric(false, true, EnclaveKey::Missing, Biometrics::Available),
            "use_os_keychain=false must never reach Secure Enclave key generation"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn never_rebinds_while_the_existing_key_still_works() {
        assert!(
            !should_rebind_biometric(true, true, EnclaveKey::Loads, Biometrics::Available),
            "the enclave key loads fine: nothing is orphaned, so nothing to repair"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn never_rebinds_without_biometric_hardware() {
        assert!(
            !should_rebind_biometric(true, true, EnclaveKey::Missing, Biometrics::Absent),
            "no enrolled biometrics: generating a key would produce another dead wrapper"
        );
    }

    // A controllable clock for lockout tests. Thread-local because the test
    // harness gives each test its own thread, so tests cannot disturb one
    // another's time even when run in parallel.
    thread_local! {
        static TEST_CLOCK_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    fn test_clock() -> u64 {
        TEST_CLOCK_MS.with(std::cell::Cell::get)
    }

    fn set_clock(ms: u64) {
        TEST_CLOCK_MS.with(|c| c.set(ms));
    }

    fn advance_clock(ms: u64) {
        TEST_CLOCK_MS.with(|c| c.set(c.get() + ms));
    }

    fn fast_vault_config(path: std::path::PathBuf) -> VaultConfig {
        VaultConfig {
            vault_path: path,
            argon2_params: Some(Argon2Params {
                t: 1,
                m_kib: 32768,
                p: 1,
            }),
            // Tests never touch the real login keychain.
            use_os_keychain: false,
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
            .set_password("server1:22".into(), &SecretString::from("pass123"))
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
        vault.change_password(old_pass, new_pass).unwrap();

        assert!(vault.unlock_with_password(old_pass).is_err());

        let unlocked = vault.unlock_with_password(new_pass).unwrap();
        assert!(unlocked.get_password("server1:22").is_none());
    }

    #[tokio::test]
    async fn test_rate_limiting_lockout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let config = fast_vault_config(path.clone());

        // Drive time explicitly. A real attempt costs an Argon2 KDF plus a
        // keychain write — together often more than the 1s first backoff tier —
        // so a wall-clock version of this test races its own setup cost and
        // flaps between "rate limited" and "window already expired".
        let vault = Vault::with_clock(config, test_clock);
        set_clock(1_000_000);

        let password = "correct-password";
        vault.initialize(password).await.unwrap();

        for _ in 0..3 {
            assert!(vault.unlock_with_password("wrong").is_err());
        }

        // The third failure earns a 1s backoff; a retry inside it is refused.
        assert!(matches!(
            vault.unlock_with_password("wrong"),
            Err(VaultError::RateLimited(_))
        ));

        // Past the backoff window, the correct password works again.
        advance_clock(2000);
        let unlocked = vault.unlock_with_password(password).unwrap();
        assert!(unlocked.get_password("test").is_none());

        // A success resets the counter, so the next failure is a plain
        // wrong-password error rather than another rate limit.
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
        let result = vault.unlock_biometric().await;
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
        let result = vault.unlock_biometric().await;
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::BiometricFailed)));
    }

    /// `Vault` must not grow a second home for an unlocked vault.
    ///
    /// It had one: an `Option<UnlockedVault>` cache that nothing ever wrote to.
    /// `get_unlocked` therefore always missed and always did a fresh biometric
    /// unlock despite its name, and `Vault::lock` -- documented as "clear
    /// memory" -- took from a field that was permanently `None`, so it was a
    /// no-op that read as a security control. Its test asserted nothing and
    /// passed; `get_unlocked`'s asserted the miss.
    ///
    /// The unlocked vault belongs to the caller (multitop holds it in
    /// `VaultState::Unlocked`). Two owners of one key, with independent
    /// lifetimes, is the state this asserts cannot come back.
    #[test]
    fn the_vault_holds_no_second_copy_of_an_unlocked_one() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let vault = Vault::new(fast_vault_config(path));
        // Every field, spelled out: a cache added here would have to be added
        // to this list too, which is the point at which someone asks why.
        let Vault {
            config: _,
            lockout: _,
            lockout_init: _,
            lockout_loaded: _,
            clock: _,
        } = vault;
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
        let config = fast_vault_config(path);
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
            .set_password("server1:22".into(), &SecretString::from("pass1"))
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
            .set_password("server1:22".into(), &SecretString::from("pass1"))
            .unwrap();
        unlocked
            .set_password("server2:22".into(), &SecretString::from("pass2"))
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
                .set_password("server1:22".into(), &SecretString::from("pass1"))
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
                .set_password(host, &SecretString::from(pass.as_str()))
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
                    .set_password(host, &SecretString::from(pass.as_str()))
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
}

#[cfg(test)]
mod lazy_lockout_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::lockout::LockoutState;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn fast(path: std::path::PathBuf) -> VaultConfig {
        VaultConfig {
            vault_path: path,
            argon2_params: Some(crate::crypto::Argon2Params {
                t: 1,
                m_kib: 32768,
                p: 1,
            }),
            // Tests never touch the real login keychain.
            use_os_keychain: false,
        }
    }

    /// A lockout already on disk must be honoured by a freshly constructed
    /// `Vault`, even though the state is now loaded lazily rather than in the
    /// constructor.
    #[tokio::test]
    async fn a_persisted_lockout_is_honoured_after_lazy_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let vault = Vault::new(fast(path.clone()));
        vault.initialize("pw").await.unwrap();

        // Someone was locked out earlier in a previous run of the app.
        let mut state = LockoutState::load(&path, false);
        for _ in 0..12 {
            state.on_attempt(&path, crate::crypto::now_ms());
        }

        // A brand new Vault -- the constructor reads nothing.
        let fresh = Vault::new(fast(path.clone()));
        let err = fresh.unlock_with_password("pw").unwrap_err();
        assert!(
            matches!(err, VaultError::RateLimited(_)),
            "the persisted lockout must survive lazy loading, got {err:?}"
        );
    }

    /// Concurrent first-use must not let anyone through unlimited.
    ///
    /// The lazy load used to set its "loaded" flag before actually loading, so
    /// a second caller arriving in that window checked the limiter against a
    /// default (empty) state and was never rate limited at all.
    #[tokio::test]
    async fn concurrent_first_use_cannot_bypass_the_limiter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let vault = Vault::new(fast(path.clone()));
        vault.initialize("pw").await.unwrap();

        let mut state = LockoutState::load(&path, false);
        for _ in 0..12 {
            state.on_attempt(&path, crate::crypto::now_ms());
        }

        // Several threads race into the very first use of one Vault.
        let shared = Arc::new(Vault::new(fast(path.clone())));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let v = Arc::clone(&shared);
            handles.push(std::thread::spawn(move || {
                matches!(
                    v.unlock_with_password("pw"),
                    Err(VaultError::RateLimited(_))
                )
            }));
        }
        let limited: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            limited.iter().all(|b| *b),
            "every racing caller must see the lockout, got {limited:?}"
        );
    }
}

#[cfg(test)]
mod keychain_policy_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn cfg(use_os_keychain: bool) -> VaultConfig {
        VaultConfig {
            vault_path: std::path::PathBuf::from("/tmp/multitop-policy/vault.bin"),
            argon2_params: None,
            use_os_keychain,
        }
    }

    /// The limiter must carry the vault's keychain policy from the moment the
    /// vault exists, not from the first time someone happens to load it.
    ///
    /// `LockoutState::default()` cannot know the answer -- `use_keychain` is
    /// `#[serde(skip)]`, so it defaults to false. Building a production vault
    /// on that made it claim, briefly, that it must not persist to the
    /// keychain. Nothing exploited the window today because
    /// `unlock_with_password` loads first, but the value was wrong and a new
    /// caller would have inherited it.
    #[test]
    fn a_vault_carries_its_keychain_policy_from_construction() {
        let real = Vault::new(cfg(true));
        assert!(
            real.lockout.lock().unwrap().uses_keychain(),
            "a production vault must intend to persist the limiter to the keychain \
             before anything is loaded"
        );

        let isolated = Vault::new(cfg(false));
        assert!(
            !isolated.lockout.lock().unwrap().uses_keychain(),
            "and a vault told not to must never intend to"
        );
    }

    /// The trap itself, pinned so nobody reintroduces `default()` here.
    #[test]
    fn default_lockout_state_does_not_know_the_policy() {
        assert!(
            !crate::lockout::LockoutState::default().uses_keychain(),
            "default() answers false because it cannot know; construct with new()"
        );
        assert!(crate::lockout::LockoutState::new(true).uses_keychain());
    }

    /// `VaultConfig::default()` is the one place a caller can avoid stating the
    /// policy. It answers `true` on purpose: a test that slips through gets the
    /// real keychain and is noticed immediately, whereas `false` would let
    /// production quietly stop persisting the limiter.
    #[test]
    fn the_config_default_fails_loudly_rather_than_silently() {
        assert!(
            VaultConfig::default().use_os_keychain,
            "the default must be the fail-loud direction"
        );
    }
}
