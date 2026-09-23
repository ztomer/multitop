use super::{load_state, state_file_path, AppState};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("multitop_state_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.toml")
}

/// A corrupt state file must not read as a first run.
///
/// It did, and that was worse than ignoring it: the next `persist_state`
/// wrote a fresh file straight over it, so the history was destroyed rather
/// than merely unread. `write_atomic` exists so an interrupted write cannot
/// lose `upgrade_started_at`; the loader threw it away anyway.
#[test]
fn a_corrupt_state_file_is_kept_rather_than_overwritten() {
    let cfg = scratch("corrupt");
    let path = state_file_path(&cfg);
    std::fs::write(&path, "this is not = = toml [[[").unwrap();

    let loaded = load_state(&cfg);

    assert_eq!(
        loaded.state,
        AppState::default(),
        "nothing usable can be recovered from it"
    );
    let notice = loaded
        .notice
        .expect("a file that could not be parsed is not silence");
    assert!(
        notice.contains("could not be parsed"),
        "the notice must say what happened: {notice}"
    );

    let kept = path.with_extension("toml.unreadable");
    assert!(
        kept.exists(),
        "the unreadable file must be moved aside, or the next write destroys it"
    );
    assert!(
        !path.exists(),
        "and out of the way of that write: {}",
        path.display()
    );
    assert_eq!(
        std::fs::read_to_string(&kept).unwrap(),
        "this is not = = toml [[[",
        "kept verbatim, so a human can still look at it"
    );

    let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
}

/// A first run is the one case that legitimately has no state, and it must
/// stay silent -- a notice on every fresh install would train the user to
/// ignore the line that matters.
#[test]
fn a_missing_state_file_says_nothing() {
    let cfg = scratch("missing");
    let loaded = load_state(&cfg);
    assert_eq!(loaded.state, AppState::default());
    assert_eq!(loaded.notice, None);
    let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
}
