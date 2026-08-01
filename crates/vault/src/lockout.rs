use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

const MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT: u32 = 10;
const HARD_LOCKOUT_DURATION_MS: u64 = 300_000; // 5 minutes
const KEYCHAIN_SERVICE: &str = "multitop-vault-lockout";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockoutState {
    pub failed_attempts: u32,
    pub lockout_until_epoch_ms: u64,
}

impl LockoutState {
    fn account_name(vault_path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let canonical =
            std::fs::canonicalize(vault_path).unwrap_or_else(|_| vault_path.to_path_buf());
        let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
        format!("lockout-{}", hex::encode(hash))
    }

    #[must_use]
    pub fn load(vault_path: &Path) -> Self {
        // Try keychain first (preferred - can't be trivially deleted)
        let account = Self::account_name(vault_path);
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &account) {
            if let Ok(stored) = entry.get_password() {
                if let Ok(state) = serde_json::from_str(&stored) {
                    return state;
                }
            }
        }

        // Fallback to file-based storage (legacy)
        let path = Self::lockout_path(vault_path);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Self {
                failed_attempts: 0,
                lockout_until_epoch_ms: 0,
            })
    }

    fn lockout_path(vault_path: &Path) -> std::path::PathBuf {
        let mut p = vault_path.to_path_buf();
        let ext = format!(
            "{}.lockout",
            p.extension()
                .map_or("bin", |e| e.to_str().unwrap_or("bin"))
        );
        p.set_extension(&ext);
        p
    }

    pub fn save(&self, vault_path: &Path) {
        let json = match serde_json::to_string(self) {
            Ok(j) => j,
            Err(_) => return,
        };

        // Save to keychain (preferred - survives file deletion)
        let account = Self::account_name(vault_path);
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &account) {
            let _ = entry.set_password(&json);
        }

        // Also save to file as backup
        let path = Self::lockout_path(vault_path);
        let _ = std::fs::write(&path, &json);
    }

    pub fn on_failure(&mut self, vault_path: &Path, now_ms: u64) {
        self.failed_attempts += 1;

        if self.failed_attempts >= MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT {
            self.lockout_until_epoch_ms = now_ms + HARD_LOCKOUT_DURATION_MS;
        } else if self.failed_attempts >= 3 {
            let delay_sec = (1u64 << (self.failed_attempts - 3)).min(60);
            self.lockout_until_epoch_ms = now_ms + (delay_sec * 1000);
        }

        self.save(vault_path);
    }

    pub fn on_success(&mut self, vault_path: &Path) {
        self.failed_attempts = 0;
        self.lockout_until_epoch_ms = 0;
        self.save(vault_path);
    }

    pub const fn check_lockout(&self, now_ms: u64) -> Result<(), crate::VaultError> {
        if now_ms < self.lockout_until_epoch_ms {
            let remaining = (self.lockout_until_epoch_ms - now_ms).div_ceil(1000);
            return Err(crate::VaultError::RateLimited(remaining));
        }
        Ok(())
    }
}

/// Guard that records failures in the lockout state on drop (unless marked success).
pub struct LockoutGuard<'a> {
    state: &'a Mutex<LockoutState>,
    vault_path: &'a Path,
    succeeded: bool,
    start_time_ms: u64,
}

impl<'a> LockoutGuard<'a> {
    pub const fn new(state: &'a Mutex<LockoutState>, vault_path: &'a Path, now_ms: u64) -> Self {
        Self {
            state,
            vault_path,
            succeeded: false,
            start_time_ms: now_ms,
        }
    }

    pub const fn mark_success(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for LockoutGuard<'_> {
    fn drop(&mut self) {
        // Ignore poison errors - we still want to update state
        if let Ok(mut lockout) = self.state.lock() {
            if self.succeeded {
                lockout.on_success(self.vault_path);
            } else {
                lockout.on_failure(self.vault_path, self.start_time_ms);
            }
        }
    }
}

#[cfg(test)]
mod tests {
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::VaultError;
    use tempfile::TempDir;

    fn make_test_lockout() -> (LockoutState, tempfile::TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let state = LockoutState::load(&path);
        (state, dir)
    }

    #[test]
    fn test_lockout_state_default() {
        let (state, _dir) = make_test_lockout();
        assert_eq!(state.failed_attempts, 0);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }

    #[test]
    fn test_lockout_state_load_save() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let state = LockoutState {
            failed_attempts: 5,
            lockout_until_epoch_ms: 12345,
        };
        state.save(&path);

        let loaded = LockoutState::load(&path);
        assert_eq!(loaded.failed_attempts, 5);
        assert_eq!(loaded.lockout_until_epoch_ms, 12345);
    }

    #[test]
    fn test_lockout_state_load_nonexistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.bin");

        let state = LockoutState::load(&path);
        assert_eq!(state.failed_attempts, 0);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }

    #[test]
    fn test_lockout_on_failure_no_delay() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let mut state = LockoutState::load(&path);

        // First 2 failures should not trigger delay
        state.on_failure(&path, 1000);
        assert_eq!(state.failed_attempts, 1);
        assert_eq!(state.lockout_until_epoch_ms, 0);

        state.on_failure(&path, 1000);
        assert_eq!(state.failed_attempts, 2);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }

    #[test]
    fn test_lockout_on_failure_exponential_backoff() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let mut state = LockoutState::load(&path);

        // 3 failures: 1s delay
        state.on_failure(&path, 1000);
        state.on_failure(&path, 1000);
        state.on_failure(&path, 1000);
        assert_eq!(state.failed_attempts, 3);
        assert_eq!(state.lockout_until_epoch_ms, 2000); // 1000 + 1000ms

        // 4 failures: 2s delay
        state.on_failure(&path, 2000);
        assert_eq!(state.failed_attempts, 4);
        assert_eq!(state.lockout_until_epoch_ms, 4000); // 2000 + 2000ms

        // 5 failures: 4s delay
        state.on_failure(&path, 4000);
        assert_eq!(state.failed_attempts, 5);
        assert_eq!(state.lockout_until_epoch_ms, 8000); // 4000 + 4000ms
    }

    #[test]
    fn test_lockout_on_failure_max_delay_capped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let mut state = LockoutState::load(&path);

        // Simulate many failures
        for i in 0..20 {
            state.on_failure(&path, i * 1000);
        }

        // Delay should be capped at 60 seconds
        // After 10 failures, we're in hard lockout (5 minutes = 300,000 ms)
        // The test checks that after many failures, the delay is reasonable
        assert!(state.lockout_until_epoch_ms > 0);
        // Hard lockout is 300,000 ms from the last failure
        assert!(state.lockout_until_epoch_ms >= 19 * 1000 + 300_000 - 1000);
    }

    #[test]
    fn test_lockout_on_failure_hard_lockout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let mut state = LockoutState::load(&path);

        // 10 failures: hard lockout (5 minutes)
        for i in 0..10 {
            state.on_failure(&path, i * 1000);
        }

        assert_eq!(state.failed_attempts, 10);
        // Should be locked out for 5 minutes (300,000 ms)
        assert!(state.lockout_until_epoch_ms > 9 * 1000 + 300_000 - 1000);
    }

    #[test]
    fn test_lockout_on_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let mut state = LockoutState {
            failed_attempts: 5,
            lockout_until_epoch_ms: 99999,
        };

        state.on_success(&path);
        assert_eq!(state.failed_attempts, 0);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }

    #[test]
    fn test_check_lockout_not_locked() {
        let state = LockoutState {
            failed_attempts: 0,
            lockout_until_epoch_ms: 0,
        };
        assert!(state.check_lockout(1000).is_ok());
    }

    #[test]
    fn test_check_lockout_locked() {
        let state = LockoutState {
            failed_attempts: 3,
            lockout_until_epoch_ms: 5000,
        };

        // Before lockout expires
        assert!(state.check_lockout(4000).is_err());
        assert!(matches!(
            state.check_lockout(4000),
            Err(VaultError::RateLimited(_))
        ));

        // After lockout expires
        assert!(state.check_lockout(5000).is_ok());
        assert!(state.check_lockout(6000).is_ok());
    }

    #[test]
    fn test_lockout_guard_records_failure() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let state = Mutex::new(LockoutState::load(&path));

        {
            let _guard = LockoutGuard::new(&state, &path, 1000);
            // Guard drops without marking success
        }

        let state = state.lock().unwrap();
        assert_eq!(state.failed_attempts, 1);
    }

    #[test]
    fn test_lockout_guard_records_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let state = Mutex::new(LockoutState {
            failed_attempts: 3,
            lockout_until_epoch_ms: 5000,
        });

        {
            let mut guard = LockoutGuard::new(&state, &path, 1000);
            guard.mark_success();
        }

        let state = state.lock().unwrap();
        assert_eq!(state.failed_attempts, 0);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }
}
