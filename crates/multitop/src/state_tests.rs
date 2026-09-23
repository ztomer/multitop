use super::*;

#[test]
fn state_save_and_load_roundtrip() {
    let temp_dir = std::env::temp_dir().join("multitop_test_state");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let state = AppState {
        last_update: Some(1_722_000_000),
        upgrade_started_at: None,
        hosts: BTreeMap::new(),
        selected_host: None,
        filter_query: None,
        sort: None,
        views: BTreeMap::new(),
        saved_filters: Vec::new(),
    };

    save_state(&config_path, &state).unwrap();
    let loaded = load_state(&config_path);

    assert_eq!(loaded.state, state);
    assert_eq!(loaded.notice, None, "a clean load says nothing");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn per_host_records_roundtrip() {
    let temp_dir = std::env::temp_dir().join("multitop_test_state_hosts");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let mut hosts = BTreeMap::new();
    hosts.insert(
        "admin@web-01:22".to_string(),
        HostUpdate {
            started_at: Some(1_722_000_000),
            finished_at: Some(1_722_000_072),
            success: true,
        },
    );
    // An interrupted run: started, never finished.
    hosts.insert(
        "admin@db-02:22".to_string(),
        HostUpdate {
            started_at: Some(1_722_000_000),
            finished_at: None,
            success: false,
        },
    );

    let state = AppState {
        last_update: Some(1_722_000_072),
        upgrade_started_at: None,
        hosts,
        selected_host: None,
        filter_query: None,
        sort: None,
        views: BTreeMap::new(),
        saved_filters: Vec::new(),
    };
    save_state(&config_path, &state).unwrap();
    let loaded = load_state(&config_path);

    assert_eq!(loaded.state, state);
    assert_eq!(loaded.notice, None, "a clean load says nothing");
    assert_eq!(loaded.state.hosts["admin@web-01:22"].outcome(), Outcome::Ok);
    assert_eq!(
        loaded.state.hosts["admin@web-01:22"].duration_secs(),
        Some(72)
    );
    assert_eq!(
        loaded.state.hosts["admin@db-02:22"].outcome(),
        Outcome::Interrupted
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// A state.toml written before per-host records existed must still load.
#[test]
fn legacy_state_file_without_hosts_still_loads() {
    let temp_dir = std::env::temp_dir().join("multitop_test_state_legacy");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");
    std::fs::write(
        state_file_path(&config_path),
        "last_update = 1722000000\nupgrade_started_at = 1723000000\n",
    )
    .unwrap();

    let loaded = load_state(&config_path);
    assert_eq!(loaded.state.last_update, Some(1_722_000_000));
    assert_eq!(loaded.state.upgrade_started_at, Some(1_723_000_000));
    assert!(loaded.state.hosts.is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn outcome_classifies_every_combination() {
    assert_eq!(HostUpdate::default().outcome(), Outcome::Never);
    assert_eq!(
        HostUpdate {
            started_at: Some(1),
            finished_at: None,
            success: false
        }
        .outcome(),
        Outcome::Interrupted
    );
    assert_eq!(
        HostUpdate {
            started_at: Some(1),
            finished_at: Some(2),
            success: true
        }
        .outcome(),
        Outcome::Ok
    );
    assert_eq!(
        HostUpdate {
            started_at: Some(1),
            finished_at: Some(2),
            success: false
        }
        .outcome(),
        Outcome::Failed
    );
}

#[test]
fn upgrade_started_at_roundtrip() {
    let temp_dir = std::env::temp_dir().join("multitop_test_started");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let state = AppState {
        last_update: None,
        upgrade_started_at: Some(1_723_000_000),
        hosts: BTreeMap::new(),
        selected_host: None,
        filter_query: None,
        sort: None,
        views: BTreeMap::new(),
        saved_filters: Vec::new(),
    };

    save_state(&config_path, &state).unwrap();
    let loaded = load_state(&config_path);

    assert_eq!(loaded.state, state);
    assert_eq!(loaded.notice, None, "a clean load says nothing");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn layout_state_roundtrip() {
    let temp_dir = std::env::temp_dir().join("multitop_test_layout");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("config.toml");

    let mut views = BTreeMap::new();
    views.insert("ztomer@192.168.0.33:22".to_string(), "docker".to_string());
    views.insert("ztomer@192.168.0.90:22".to_string(), "fetch".to_string());

    let state = AppState {
        last_update: None,
        upgrade_started_at: None,
        hosts: BTreeMap::new(),
        selected_host: Some("ztomer@192.168.0.33:22".to_string()),
        filter_query: Some("beelink".to_string()),
        sort: Some("mem".to_string()),
        views,
        saved_filters: Vec::new(),
    };

    save_state(&config_path, &state).unwrap();
    let loaded = load_state(&config_path);

    assert_eq!(loaded.state, state);
    assert_eq!(
        loaded.state.selected_host.as_deref(),
        Some("ztomer@192.168.0.33:22")
    );
    assert_eq!(loaded.state.filter_query.as_deref(), Some("beelink"));
    assert_eq!(loaded.state.sort.as_deref(), Some("mem"));
    assert_eq!(loaded.state.views.len(), 2);
    assert_eq!(loaded.state.views["ztomer@192.168.0.33:22"], "docker");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
