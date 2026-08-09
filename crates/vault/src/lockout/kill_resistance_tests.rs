//! Tests that the limiter survives the process dying mid-attempt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::lockout::state::MAX_ATTEMPTS_BEFORE_HARD_LOCKOUT;
use crate::lockout::LockoutState;
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
