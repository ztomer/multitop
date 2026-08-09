//! Tests that an attempt is recorded before it is made.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::lockout::state::MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT;
use crate::lockout::LockoutState;
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
