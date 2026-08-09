//! The async runtime: terminal event loop plus one SSH task per panel.
//!
//! Split into submodules by concern:
//! - [`terminal`] — signal handling, terminal modes, guard
//! - [`tasks`] — per-panel task handles (monitors, aux, upgrades)
//! - [`dims`] — agent render size computation
//! - [`event_loop`] — the core `select!` loop + `panel_at_pos`
//! - [`handle_key`] — key dispatch + `execute_cmds`
//! - [`spawn`] — monitor spawn (re-exported)

pub(super) mod dims;
mod entry;
pub(super) mod event_loop;
pub(super) mod handle_key;
pub(super) mod tasks;
pub(super) mod terminal;

pub mod spawn;

pub use entry::run;
pub use event_loop::{event_loop, panel_at_pos, LoopOutcome};
pub use handle_key::handle_key;
pub use spawn::spawn_biometric_unlock;
pub use tasks::Tasks;
pub use terminal::SignalAction;
pub use terminal::TerminalGuard;

const RESIZE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(30);
pub(super) const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];

/// What one monitor session achieved, from the reconnect loop's point of view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SessionOutcome {
    /// The connection was never established.
    NeverConnected,
    /// It connected, and ended without ever delivering a frame.
    NoData,
    /// It delivered at least one frame before ending.
    Delivered,
}

/// How long to wait before reconnecting, and how the failure count moves.
///
/// The count used to be reset the moment `connect` returned -- which is before
/// anything has been received. A host that *accepts* the connection and then
/// fails therefore reset the backoff on every round and was retried at the
/// shortest interval forever: a login banner where the protocol should be, or
/// an agent whose version mismatch cannot be resolved because the upload keeps
/// failing, meant one `ssh` process every two seconds indefinitely -- and in the
/// second case a multi-megabyte agent upload with it.
///
/// Only delivered data says the connection is worth trusting again.
pub(super) fn reconnect_wait(outcome: SessionOutcome, failures: &mut usize) -> u64 {
    if outcome == SessionOutcome::Delivered {
        *failures = 0;
    }
    let wait = RECONNECT_BACKOFF[(*failures).min(RECONNECT_BACKOFF.len() - 1)];
    *failures = failures.saturating_add(1);
    wait
}

#[cfg(test)]
mod reconnect_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Only delivered data says the connection is worth trusting again.
    ///
    /// The count used to be reset the moment `connect` returned — before
    /// anything had been received — so a host that accepts the connection and
    /// then fails reset the backoff every round and was retried at the
    /// shortest interval forever: one `ssh` process every two seconds, and in
    /// the version-mismatch case a multi-megabyte agent upload with each.
    #[test]
    fn a_connection_that_never_delivers_backs_off_further_every_round() {
        let mut failures = 0;
        let waits: Vec<u64> = (0..6)
            .map(|_| reconnect_wait(SessionOutcome::NoData, &mut failures))
            .collect();
        assert_eq!(waits, vec![2, 5, 10, 20, 20, 20]);
    }

    #[test]
    fn a_connection_that_was_never_established_backs_off_the_same_way() {
        let mut failures = 0;
        assert_eq!(
            reconnect_wait(SessionOutcome::NeverConnected, &mut failures),
            2
        );
        assert_eq!(
            reconnect_wait(SessionOutcome::NeverConnected, &mut failures),
            5
        );
    }

    #[test]
    fn delivering_a_frame_is_what_earns_the_short_interval_back() {
        let mut failures = 0;
        for _ in 0..5 {
            reconnect_wait(SessionOutcome::NoData, &mut failures);
        }
        assert_eq!(
            reconnect_wait(SessionOutcome::Delivered, &mut failures),
            RECONNECT_BACKOFF[0],
            "a delivered frame did not reset the backoff"
        );
    }

    /// The index is clamped, so a long-dead host cannot walk off the end of
    /// the table however many times it has failed.
    #[test]
    fn a_failure_count_past_the_table_still_answers() {
        let mut failures = usize::MAX;
        assert_eq!(
            reconnect_wait(SessionOutcome::NoData, &mut failures),
            RECONNECT_BACKOFF[RECONNECT_BACKOFF.len() - 1]
        );
        assert_eq!(
            failures,
            usize::MAX,
            "the count wrapped instead of saturating"
        );
    }
}
