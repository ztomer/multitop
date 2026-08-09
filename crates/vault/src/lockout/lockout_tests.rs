//! Tests for the rate limiter.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::lockout::{LockoutGuard, LockoutState};
use crate::VaultError;
use std::sync::Mutex;
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

// These drive `on_attempt`, the function the vault actually calls. They
// used to drive `on_failure`, a near-copy with no production call sites
// that re-implemented `backoff_deadline` inline -- so the tiers were pinned
// only on a dead duplicate, and the live limiter could have drifted without
// a single test noticing.
#[test]
fn the_first_two_attempts_impose_no_delay() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let mut state = LockoutState::load(&path, false);

    // First 2 failures should not trigger delay
    state.on_attempt(&path, 1000);
    assert_eq!(state.failed_attempts, 1);
    assert_eq!(state.lockout_until_epoch_ms, 0);

    state.on_attempt(&path, 1000);
    assert_eq!(state.failed_attempts, 2);
    assert_eq!(state.lockout_until_epoch_ms, 0);
}

#[test]
fn the_backoff_doubles_from_the_third_attempt() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let mut state = LockoutState::load(&path, false);

    // 3 failures: 1s delay
    state.on_attempt(&path, 1000);
    state.on_attempt(&path, 1000);
    state.on_attempt(&path, 1000);
    assert_eq!(state.failed_attempts, 3);
    assert_eq!(state.lockout_until_epoch_ms, 2000); // 1000 + 1000ms

    // 4 failures: 2s delay
    state.on_attempt(&path, 2000);
    assert_eq!(state.failed_attempts, 4);
    assert_eq!(state.lockout_until_epoch_ms, 4000); // 2000 + 2000ms

    // 5 failures: 4s delay
    state.on_attempt(&path, 4000);
    assert_eq!(state.failed_attempts, 5);
    assert_eq!(state.lockout_until_epoch_ms, 8000); // 4000 + 4000ms
}

#[test]
fn many_attempts_end_in_the_hard_lockout() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let mut state = LockoutState::load(&path, false);

    // Simulate many failures
    for i in 0..20 {
        state.on_attempt(&path, i * 1000);
    }

    // Delay should be capped at 60 seconds
    // After 10 failures, we're in hard lockout (5 minutes = 300,000 ms)
    // The test checks that after many failures, the delay is reasonable
    assert!(state.lockout_until_epoch_ms > 0);
    // Hard lockout is 300,000 ms from the last failure
    assert!(state.lockout_until_epoch_ms >= 19 * 1000 + 300_000 - 1000);
}

#[test]
fn ten_attempts_trigger_the_hard_lockout() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let mut state = LockoutState::load(&path, false);

    // 10 failures: hard lockout (5 minutes)
    for i in 0..10 {
        state.on_attempt(&path, i * 1000);
    }

    assert_eq!(state.failed_attempts, 10);
    // Should be locked out for 5 minutes (300,000 ms)
    assert!(state.lockout_until_epoch_ms > 9 * 1000 + 300_000 - 1000);
}

#[test]
fn a_recorded_failure_never_pulls_the_deadline_earlier() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let mut state = LockoutState::load(&path, false);

    // Three attempts, the last at a late clock, so the deadline is far out.
    state.on_attempt(&path, 1000);
    state.on_attempt(&path, 1000);
    state.on_attempt(&path, 100_000);
    assert_eq!(state.lockout_until_epoch_ms, 101_000);

    // Recording the failure against an earlier clock must not shorten it.
    // The deleted `on_failure` assigned the deadline outright instead of
    // taking the max, so it could hand back time an attacker had spent.
    state.on_failure_recorded(&path, 1000);
    assert_eq!(
        state.lockout_until_epoch_ms, 101_000,
        "the backoff deadline must never move earlier"
    );
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

/// The trap itself, pinned so nobody reintroduces `default()` here.
#[test]
fn default_lockout_state_does_not_know_the_policy() {
    // keychain-safe: constructs a `LockoutState` and reads the flag back. The
    // state is never loaded or saved, and loading is the only thing that
    // touches the credential store.
    assert!(
        !LockoutState::default().use_keychain,
        "default() answers false because it cannot know; construct with new()"
    );
    assert!(LockoutState::new(true).use_keychain);
}
