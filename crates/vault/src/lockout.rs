use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

const MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT: u32 = 10;
const HARD_LOCKOUT_DURATION_MS: u64 = 300_000; // 5 minutes
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
    use_keychain: bool,
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

/// Guard that finalises an attempt on drop.
///
/// The attempt is already counted by `on_attempt` before the KDF runs, so this
/// only anchors the backoff deadline (on failure) or clears the counter (on
/// success). Dying before this drops therefore leaves the attempt counted, not
/// forgiven.
pub struct LockoutGuard<'a> {
    state: &'a Mutex<LockoutState>,
    vault_path: &'a Path,
    succeeded: bool,
    /// Read when the guard drops, so the backoff is anchored to the moment the
    /// attempt FAILED. Injectable so tests do not depend on the wall clock.
    now: fn() -> u64,
}

impl<'a> LockoutGuard<'a> {
    pub fn new(state: &'a Mutex<LockoutState>, vault_path: &'a Path) -> Self {
        Self {
            state,
            vault_path,
            succeeded: false,
            now: crate::crypto::now_ms,
        }
    }

    /// `new` with a fixed clock, for tests that assert on exact deadlines.
    pub const fn with_clock(
        state: &'a Mutex<LockoutState>,
        vault_path: &'a Path,
        now: fn() -> u64,
    ) -> Self {
        Self {
            state,
            vault_path,
            succeeded: false,
            now,
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
                // Anchor the backoff to NOW (the failure), not to when the
                // attempt began. The Argon2 KDF can take longer than the delay
                // itself, and anchoring to the start would let the whole
                // backoff window elapse before the guesser could even retry —
                // the early tiers would impose no delay at all.
                lockout.on_failure_recorded(self.vault_path, (self.now)());
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
        let state = LockoutState::load(&path, false);
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
            use_keychain: false,
        };
        state.save(&path);

        let loaded = LockoutState::load(&path, false);
        assert_eq!(loaded.failed_attempts, 5);
        assert_eq!(loaded.lockout_until_epoch_ms, 12345);
    }

    #[test]
    fn test_lockout_state_load_nonexistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.bin");

        let state = LockoutState::load(&path, false);
        assert_eq!(state.failed_attempts, 0);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }

    #[test]
    fn test_lockout_on_failure_no_delay() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let mut state = LockoutState::load(&path, false);

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
        let mut state = LockoutState::load(&path, false);

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
        let mut state = LockoutState::load(&path, false);

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
        let mut state = LockoutState::load(&path, false);

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
            use_keychain: false,
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
            use_keychain: false,
        };
        assert!(state.check_lockout(1000).is_ok());
    }

    #[test]
    fn test_check_lockout_locked() {
        let state = LockoutState {
            failed_attempts: 3,
            lockout_until_epoch_ms: 5000,
            use_keychain: false,
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
        let state = Mutex::new(LockoutState::load(&path, false));

        // Real usage: the caller counts the attempt before the KDF, the guard
        // finalises it. The guard no longer increments on its own.
        state.lock().unwrap().on_attempt(&path, 1000);
        {
            let _guard = LockoutGuard::with_clock(&state, &path, || 1000);
            // Guard drops without marking success
        }

        let state = state.lock().unwrap();
        assert_eq!(state.failed_attempts, 1);
    }

    /// The backoff must start when the attempt FAILS, not when it began.
    ///
    /// A password attempt is dominated by the Argon2 KDF, which can take longer
    /// than the first backoff tiers (1s, 2s). Anchoring the deadline to the
    /// start of the attempt meant the window could already be over by the time
    /// the guesser was able to retry, so the early tiers imposed no delay at
    /// all — the rate limiter silently did nothing on slower machines and
    /// under load.
    #[test]
    fn guard_anchors_backoff_to_failure_time_not_attempt_start() {
        // The attempt starts at t=1000 and fails at t=9000 — a KDF far slower
        // than the 1s tier this third failure earns.
        const ATTEMPT_START_MS: u64 = 1000;
        const FAILURE_MS: u64 = 9000;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let state = Mutex::new(LockoutState {
            failed_attempts: 2,
            lockout_until_epoch_ms: 0,
            use_keychain: false,
        });

        // The attempt is counted up front (durable), then finalised on drop.
        state.lock().unwrap().on_attempt(&path, ATTEMPT_START_MS);
        {
            let _guard = LockoutGuard::with_clock(&state, &path, || FAILURE_MS);
        }

        let (attempts, until, locked_on_retry) = {
            let locked = state.lock().unwrap();
            (
                locked.failed_attempts,
                locked.lockout_until_epoch_ms,
                locked.check_lockout(FAILURE_MS).is_err(),
            )
        };

        assert_eq!(attempts, 3);
        assert_eq!(
            until,
            FAILURE_MS + 1000,
            "third failure earns a 1s backoff measured from the failure"
        );
        // The whole point: a retry right after the slow attempt is still locked.
        assert!(
            locked_on_retry,
            "an immediate retry must still be rate limited"
        );
        assert!(
            until > ATTEMPT_START_MS + 1000,
            "anchoring to the attempt start would have already expired"
        );
    }

    #[test]
    fn test_lockout_guard_records_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        let state = Mutex::new(LockoutState {
            failed_attempts: 3,
            lockout_until_epoch_ms: 5000,
            use_keychain: false,
        });

        {
            let mut guard = LockoutGuard::with_clock(&state, &path, || 1000);
            guard.mark_success();
        }

        let state = state.lock().unwrap();
        assert_eq!(state.failed_attempts, 0);
        assert_eq!(state.lockout_until_epoch_ms, 0);
    }
}

#[cfg(test)]
mod write_ahead_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use tempfile::TempDir;

    /// The limiter must survive the process dying mid-attempt.
    ///
    /// Failures used to be recorded only when `LockoutGuard` dropped. Under
    /// SIGKILL or `panic = "abort"` that drop never runs, so an attacker who
    /// killed the process after each guess accumulated no attempts at all --
    /// no backoff, no hard lockout, unlimited guesses at the vault.
    #[test]
    fn an_attempt_is_durable_before_the_kdf_runs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let mut state = LockoutState::load(&path, false);
        state.on_attempt(&path, 1_000);

        // Nothing else runs: this is the process being killed mid-attempt.
        let reloaded = LockoutState::load(&path, false);
        assert_eq!(
            reloaded.failed_attempts, 1,
            "the attempt must already be on disk before the KDF is run"
        );
    }

    /// Repeated kills must still reach the hard lockout.
    #[test]
    fn killing_the_process_each_time_still_reaches_the_hard_lockout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        for _ in 0..MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT {
            // Each iteration is a fresh process that dies before its guard drops.
            let mut state = LockoutState::load(&path, false);
            state.on_attempt(&path, 1_000);
        }

        let mut state = LockoutState::load(&path, false);
        assert_eq!(state.failed_attempts, MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT);

        // The next completed failure anchors the hard lockout.
        state.on_failure_recorded(&path, 10_000);
        assert!(
            state.check_lockout(10_000).is_err(),
            "ten counted attempts must lock the vault out"
        );
    }

    /// Counting arms the backoff immediately (a durable count with no deadline
    /// is toothless), and the failure may only push it further out.
    #[test]
    fn counting_arms_the_backoff_and_failure_only_extends_it() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let mut state = LockoutState::load(&path, false);
        state.on_attempt(&path, 1_000);
        state.on_attempt(&path, 1_000);
        state.on_attempt(&path, 1_000);
        assert_eq!(
            state.lockout_until_epoch_ms, 2_000,
            "the third attempt must block from the moment it starts"
        );

        // Re-anchored at the failure, which is much later than the start.
        state.on_failure_recorded(&path, 9_000);
        assert_eq!(state.lockout_until_epoch_ms, 10_000);

        // And never moved earlier by a late or duplicate call.
        state.on_failure_recorded(&path, 1_000);
        assert_eq!(state.lockout_until_epoch_ms, 10_000);
    }

    /// A success clears everything, so a legitimate user is never penalised for
    /// attempts they got right.
    #[test]
    fn a_success_clears_counted_attempts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let mut state = LockoutState::load(&path, false);
        state.on_attempt(&path, 1_000);
        state.on_attempt(&path, 1_000);
        state.on_success(&path);

        let reloaded = LockoutState::load(&path, false);
        assert_eq!(reloaded.failed_attempts, 0);
        assert_eq!(reloaded.lockout_until_epoch_ms, 0);
    }
}

#[cfg(test)]
mod kill_resistance_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use tempfile::TempDir;

    /// An attacker who kills the process after every guess must still be rate
    /// limited.
    ///
    /// Two earlier versions of this failed. First the attempt was recorded only
    /// in `LockoutGuard::drop`, which a kill skips, so nothing accumulated at
    /// all. Then the count was made durable but the deadline was still written
    /// only on the way out, so twenty counted attempts left `check_lockout`
    /// waving every one of them through. Both are covered here because the
    /// assertions are about what the attacker gets, not about the fields.
    #[test]
    fn killing_the_process_every_attempt_is_still_rate_limited() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let mut allowed = 0;
        let mut refused = 0;
        let mut t = 1_000u64;

        // Retry as fast as a script can, always dying before the guard runs.
        for _ in 0..40 {
            let mut state = LockoutState::load(&path, false);
            if state.check_lockout(t).is_ok() {
                allowed += 1;
                state.on_attempt(&path, t);
            } else {
                refused += 1;
            }
            t += 50;
        }

        // The exact count depends on where backoff tiers fall in the window;
        // what matters is that the attacker is throttled hard, not the number.
        assert!(
            refused >= 30,
            "the limiter must refuse most of a process-killing attacker's tries, \
             refused {refused} of 40"
        );
        assert!(
            allowed <= 5,
            "only a handful should get through in a 2s window, got {allowed}"
        );
    }

    /// The hard lockout is reachable the same way over a longer campaign, so
    /// patience does not defeat it either.
    #[test]
    fn a_patient_process_killing_attacker_still_hits_the_hard_lockout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let mut t = 1_000u64;
        let mut allowed = 0;
        for _ in 0..400 {
            let mut state = LockoutState::load(&path, false);
            if state.check_lockout(t).is_ok() {
                allowed += 1;
                state.on_attempt(&path, t);
            }
            t += 1_000;
        }

        let state = LockoutState::load(&path, false);
        assert!(
            state.failed_attempts >= MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT,
            "attempts must accumulate across kills"
        );
        assert!(
            state.check_lockout(t).is_err(),
            "the hard lockout must be in force at the end"
        );
        assert!(
            allowed < 40,
            "a 400-second campaign must not yield 400 guesses, got {allowed}"
        );
    }

    /// The limiter must not punish a legitimate user: a correct password clears
    /// everything, including a provisional deadline armed by its own attempt.
    #[test]
    fn a_correct_password_clears_the_provisional_deadline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");

        let mut state = LockoutState::load(&path, false);
        for _ in 0..4 {
            state.on_attempt(&path, 1_000);
        }
        assert!(state.check_lockout(1_000).is_err(), "armed");

        state.on_success(&path);
        assert!(
            LockoutState::load(&path, false)
                .check_lockout(1_000)
                .is_ok(),
            "a success must leave the user unblocked"
        );
    }
}
