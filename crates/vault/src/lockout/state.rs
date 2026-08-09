//! The rate limiter's persisted state, and where it is kept.
//!
//! Kept in the OS credential store first and a file second: a limiter that a
//! `rm` can reset is not a limiter. The file is the fallback for hosts with no
//! usable keychain, and is written either way as a backup.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub(super) const MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT: u32 = 10;
pub(super) const HARD_LOCKOUT_DURATION_MS: u64 = 300_000; // 5 minutes
const KEYCHAIN_SERVICE: &str = "multitop-vault-lockout";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockoutState {
    pub failed_attempts: u32,
    pub lockout_until_epoch_ms: u64,
    /// Whether this state may touch the OS keychain. Carried on the value so
    /// `save` cannot disagree with `load` about it -- that exact disagreement,
    /// between the rollback counter's read and write, is what put keychain
    /// dialogs in front of the user during test runs.
    #[serde(skip)]
    // `pub(crate)`: the vault's own tests assert the policy is carried from
    // construction, and that is the whole point of the field. It stays out of
    // the crate's public surface — an accessor for it had no caller but a test,
    // which is how it ended up in the test-only baseline.
    pub(crate) use_keychain: bool,
}

impl LockoutState {
    /// An empty state that already knows whether it may use the OS keychain.
    ///
    /// Prefer this to `default()`, which cannot know: `use_keychain` is
    /// `#[serde(skip)]`, so its default is `false`. A production vault built on
    /// `default()` therefore claims it must not use the keychain until the
    /// first load replaces it -- harmless today because `unlock_with_password`
    /// always loads first, but a new caller that touched the limiter earlier
    /// would silently stop persisting it where it matters most.
    #[must_use]
    pub const fn new(use_keychain: bool) -> Self {
        Self {
            failed_attempts: 0,
            lockout_until_epoch_ms: 0,
            use_keychain,
        }
    }

    fn account_name(vault_path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let canonical =
            std::fs::canonicalize(vault_path).unwrap_or_else(|_| vault_path.to_path_buf());
        let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
        format!("lockout-{}", hex::encode(hash))
    }

    #[must_use]
    pub fn load(vault_path: &Path, use_keychain: bool) -> Self {
        // Try keychain first (preferred - can't be trivially deleted)
        if use_keychain {
            let account = Self::account_name(vault_path);
            if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &account) {
                if let Ok(stored) = entry.get_password() {
                    if let Ok(mut state) = serde_json::from_str::<Self>(&stored) {
                        state.use_keychain = use_keychain;
                        return state;
                    }
                }
            }
        }

        // Fallback to file-based storage (legacy)
        let path = Self::lockout_path(vault_path);
        let mut state: Self = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        state.use_keychain = use_keychain;
        state
    }

    fn lockout_path(vault_path: &Path) -> std::path::PathBuf {
        let mut p = vault_path.to_path_buf();
        let ext = format!(
            "{}.lockout",
            p.extension().map_or("bin", |e| e.to_str().unwrap_or("bin"))
        );
        p.set_extension(&ext);
        p
    }

    /// Persist the limiter state.
    ///
    /// # Accepted limitations
    ///
    /// Write failures are swallowed. If both the keychain and the file write
    /// fail, the limiter silently degrades rather than blocking a legitimate
    /// unlock -- failing closed on a transient keychain hiccup would lock the
    /// user out of their own vault, which is the worse outcome for a
    /// single-user desktop tool.
    ///
    /// Deleting BOTH the keychain entry and the file resets the counter. The
    /// keychain is checked first precisely because it is the harder of the two
    /// to remove, and anyone who can read it already has the stored passwords
    /// themselves, so the limiter is not the last line of defence there.
    pub fn save(&self, vault_path: &Path) {
        let Ok(json) = serde_json::to_string(self) else {
            return;
        };

        // Save to keychain (preferred - survives file deletion)
        if self.use_keychain {
            let account = Self::account_name(vault_path);
            if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &account) {
                let _ = entry.set_password(&json);
            }
        }

        // Also save to file as backup
        let path = Self::lockout_path(vault_path);
        let _ = std::fs::write(&path, &json);
    }

    /// The backoff deadline earned by the current attempt count, measured from
    /// `now_ms`.
    fn backoff_deadline(&self, now_ms: u64) -> u64 {
        if self.failed_attempts >= MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT {
            now_ms + HARD_LOCKOUT_DURATION_MS
        } else if self.failed_attempts >= 3 {
            let delay_sec = (1u64 << (self.failed_attempts - 3)).min(60);
            now_ms + (delay_sec * 1000)
        } else {
            0
        }
    }

    /// Count an attempt before it is made, persist it, and arm the backoff
    /// immediately.
    ///
    /// Both halves matter, and an earlier version did only the first. Counting
    /// up front is what makes the limiter survive the process dying: recording
    /// the failure on the way out meant an attacker who killed the process
    /// after each guess accumulated nothing at all. But a durable count with no
    /// deadline is toothless -- `check_lockout` consults the deadline, so
    /// twenty counted attempts still let the twenty-first straight through. The
    /// deadline has to be in force from the moment the attempt starts.
    pub fn on_attempt(&mut self, vault_path: &Path, now_ms: u64) {
        self.failed_attempts += 1;
        self.lockout_until_epoch_ms = self
            .backoff_deadline(now_ms)
            .max(self.lockout_until_epoch_ms);
        self.save(vault_path);
    }

    /// Re-anchor the deadline for an attempt already counted by `on_attempt`,
    /// now that the real failure time is known.
    ///
    /// The failure is necessarily later than the attempt start, so this only
    /// moves the deadline further out, never earlier. That keeps a slow KDF
    /// from consuming its own backoff window while leaving the provisional
    /// deadline in force if the process dies first.
    pub fn on_failure_recorded(&mut self, vault_path: &Path, now_ms: u64) {
        self.lockout_until_epoch_ms = self
            .backoff_deadline(now_ms)
            .max(self.lockout_until_epoch_ms);
        self.save(vault_path);
    }

    /// Clear the limiter after a correct password.
    ///
    /// Note the ordering consequence: `on_attempt` counts before the KDF runs,
    /// so a process that dies between a *successful* unlock and this call
    /// leaves one phantom failure behind. That errs toward locking rather than
    /// admitting, and a later success clears it.
    pub fn on_success(&mut self, vault_path: &Path) {
        self.failed_attempts = 0;
        self.lockout_until_epoch_ms = 0;
        self.save(vault_path);
    }

    /// Check if the vault is currently rate-limited.
    ///
    /// # Errors
    /// Returns `VaultError::RateLimited` if the lockout period has not expired.
    pub const fn check_lockout(&self, now_ms: u64) -> Result<(), crate::VaultError> {
        if now_ms < self.lockout_until_epoch_ms {
            let remaining = (self.lockout_until_epoch_ms - now_ms).div_ceil(1000);
            return Err(crate::VaultError::RateLimited(remaining));
        }
        Ok(())
    }
}
