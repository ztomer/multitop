//! High-level Vault API.
//!
//! Split by what a caller is doing: [`init`] creates one, [`unlock`] opens one,
//! [`password`] changes what opens it, [`unlocked`] is the handle you get back,
//! and [`biometric`] is the rule deciding whether an enclave wrapper is stale.

mod biometric;
mod enclave;
mod init;
mod password;
mod unlock;
mod unlocked;

#[cfg(test)]
#[path = "api_tests.rs"]
mod api_tests;
#[cfg(test)]
#[path = "keychain_policy_tests.rs"]
mod keychain_policy_tests;
#[cfg(test)]
#[path = "lazy_lockout_tests.rs"]
mod lazy_lockout_tests;

use crate::crypto::now_ms;
use crate::lockout::LockoutState;
use crate::{VaultConfig, VaultError};
use std::sync::Mutex as StdMutex;

pub use unlocked::{UnlockResult, UnlockedVault};

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
