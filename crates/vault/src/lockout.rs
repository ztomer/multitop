use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

const MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT: u32 = 10;
const HARD_LOCKOUT_DURATION_MS: u64 = 300_000; // 5 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockoutState {
    pub failed_attempts: u32,
    pub lockout_until_epoch_ms: u64,
}

impl LockoutState {
    fn lockout_path(vault_path: &Path) -> std::path::PathBuf {
        let mut p = vault_path.to_path_buf();
        let ext = format!("{}.lockout", p.extension().map(|e| e.to_str().unwrap_or("bin")).unwrap_or("bin"));
        p.set_extension(&ext);
        p
    }

    pub fn load(vault_path: &Path) -> Self {
        let path = Self::lockout_path(vault_path);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Self { failed_attempts: 0, lockout_until_epoch_ms: 0 })
    }

    pub fn save(&self, vault_path: &Path) {
        let path = Self::lockout_path(vault_path);
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(&path, &json);
        }
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

    pub fn check_lockout(&self, now_ms: u64) -> Result<(), crate::VaultError> {
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
    pub fn new(state: &'a Mutex<LockoutState>, vault_path: &'a Path, now_ms: u64) -> Self {
        Self { state, vault_path, succeeded: false, start_time_ms: now_ms }
    }

    pub fn mark_success(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for LockoutGuard<'_> {
    fn drop(&mut self) {
        let mut lockout = self.state.lock().unwrap();
        if self.succeeded {
            lockout.on_success(self.vault_path);
        } else {
            lockout.on_failure(self.vault_path, self.start_time_ms);
        }
    }
}
