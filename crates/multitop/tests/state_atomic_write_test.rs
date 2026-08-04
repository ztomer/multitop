//! `state.toml` must survive an interrupted write.
//!
//! It used to be written with `fs::write`, which truncates before writing. An
//! interruption there does not just fail to save the new value, it destroys the
//! previous one -- and the file records `upgrade_started_at`, which exists so an
//! upgrade cut short by a power loss can be reported afterwards. That value is
//! written at the moment an upgrade starts, so a power cut during the write
//! erased exactly the record power-loss detection needs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::state::{load_state, save_state, AppState, HostUpdate};
use std::collections::BTreeMap;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mt_state_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.toml")
}

fn populated() -> AppState {
    let mut hosts = BTreeMap::new();
    hosts.insert(
        "ztomer@192.168.0.33:22".to_string(),
        HostUpdate {
            started_at: Some(1_722_000_000),
            finished_at: Some(1_722_000_600),
            success: true,
        },
    );
    AppState {
        last_update: Some(1_722_000_600),
        upgrade_started_at: Some(1_722_000_000),
        hosts,
    }
}

#[test]
fn a_save_leaves_no_temporary_file_behind() {
    let cfg = scratch("tmp");
    save_state(&cfg, &populated()).unwrap();

    let leftovers: Vec<String> = std::fs::read_dir(cfg.parent().unwrap())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "a scratch file was left beside the config: {leftovers:?}"
    );

    let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
}

#[test]
fn a_leftover_temp_file_does_not_block_saving() {
    let cfg = scratch("stale");
    // Debris of the shape a killed process leaves.
    let stale = cfg.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&stale, b"garbage from a killed run").unwrap();

    save_state(&cfg, &populated()).expect("a stale scratch file must not stop a save");
    assert_eq!(load_state(&cfg).state.last_update, Some(1_722_000_600));

    let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
}

#[test]
fn a_second_save_replaces_the_first_without_an_empty_window() {
    let cfg = scratch("replace");
    save_state(&cfg, &populated()).unwrap();

    let mut next = populated();
    next.last_update = Some(1_722_999_999);
    next.upgrade_started_at = None;
    save_state(&cfg, &next).unwrap();

    let loaded = load_state(&cfg);
    assert_eq!(loaded.state.last_update, Some(1_722_999_999));
    assert_eq!(loaded.state.upgrade_started_at, None);
    assert_eq!(
        loaded.state.hosts.len(),
        1,
        "host records must survive a rewrite"
    );

    let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
}

/// Each save must publish a *new* file, not overwrite the old one in place.
///
/// This is the test that actually distinguishes the fix, and it is deterministic
/// rather than a race. The three above pass against the old `fs::write` too --
/// they check tidiness and round-tripping, not atomicity. A concurrent-reader
/// test does not work either: the atomic path is slower, so the writer finishes
/// before a reader samples the window often enough to be reliable.
///
/// Inode identity settles it. `fs::write` truncates and fills the same file, so
/// the inode is unchanged and a reader can see the gap. A rename publishes a
/// different file over the name, which is what makes the swap atomic on POSIX,
/// and the inode necessarily changes.
#[cfg(unix)]
#[test]
fn each_save_publishes_a_new_file_rather_than_truncating_in_place() {
    use std::os::unix::fs::MetadataExt;

    let cfg = scratch("inode");
    let state_file = cfg.parent().unwrap().join("state.toml");

    save_state(&cfg, &populated()).unwrap();
    let first = std::fs::metadata(&state_file).unwrap().ino();

    let mut next = populated();
    next.last_update = Some(1_722_999_999);
    save_state(&cfg, &next).unwrap();
    let second = std::fs::metadata(&state_file).unwrap().ino();

    assert_ne!(
        first, second,
        "the state file was rewritten in place, so a reader can observe it empty \
         between the truncate and the fill"
    );
    assert_eq!(load_state(&cfg).state.last_update, Some(1_722_999_999));

    let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
}
