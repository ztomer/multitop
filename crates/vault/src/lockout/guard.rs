//! The guard that records how an unlock attempt ended.
//!
//! The attempt is already counted before the KDF runs, so dying before this
//! drops leaves it counted rather than forgiven.

use std::path::Path;
use std::sync::Mutex;

use super::LockoutState;

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
