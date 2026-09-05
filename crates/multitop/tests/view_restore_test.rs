//! Restored views must never come back dead.
//!
//! Docker/Fetch need a one-shot aux task spawned by their toggle, Upgrade
//! needs a run, Alerts is rendered once at toggle time and never repainted.
//! Persisting and restoring one of those verbatim produced panels showing the
//! initial "connecting..." body forever: no backing task was ever spawned for
//! them, and the monitor stream updates history without painting. One `d`
//! press poisoned all four panels *and* the state file, so every later launch
//! restored the dead view.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::panel::Mode;

const ALL: [Mode; 6] = [
    Mode::Monitor,
    Mode::Docker,
    Mode::Fetch,
    Mode::Upgrade,
    Mode::Graphs,
    Mode::Alerts,
];

/// Only the views fed by the monitor stream alone survive a restart.
#[test]
fn only_monitor_and_graphs_survive_restart() {
    for mode in ALL {
        assert_eq!(
            mode.survives_restart(),
            matches!(mode, Mode::Monitor | Mode::Graphs),
            "{mode:?} misclassified",
        );
    }
}

/// Task-backed views start as Monitor; the rest start as themselves.
#[test]
fn task_backed_views_start_as_monitor() {
    for mode in ALL {
        let started = mode.for_startup();
        if mode.survives_restart() {
            assert_eq!(started, mode, "{mode:?} must start as itself");
        } else {
            assert_eq!(started, Mode::Monitor, "{mode:?} must start as Monitor");
        }
    }
}

/// Whatever comes out of `for_startup` is itself restart-safe, so applying
/// it twice (persist then restore) is a fixed point, not a drift.
#[test]
fn startup_mapping_is_a_fixed_point() {
    for mode in ALL {
        let once = mode.for_startup();
        assert!(once.survives_restart(), "{mode:?} maps to dead {once:?}");
        assert_eq!(once.for_startup(), once);
    }
}

/// The persisted names are always parseable, and parsing them back never
/// yields a dead view once `for_startup` is applied -- the restore path.
#[test]
fn persisted_names_restore_to_live_views() {
    for mode in ALL {
        let name = mode.for_startup().as_str();
        assert!(
            matches!(name, "monitor" | "graphs"),
            "persisted name must be self-sustaining: {name}",
        );
        let parsed: Mode = name.parse().expect("our own names must parse");
        assert_eq!(parsed.for_startup(), parsed);
    }
}
