use super::*;

// ===========================================================================
// app.rs — apply() state machine paths
// ===========================================================================

#[test]
fn apply_monitor_packet_in_monitor_mode() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    let payload = multitop_agent::proto::Payload::Monitor(Snapshot {
        host: "h1".into(),
        ..Snapshot::default()
    });
    let msg = Msg::Packet {
        panel: 0,
        gen: 0,
        epoch: 0,
        payload,
        dims: (80, 24),
    };
    let dirty = a.apply(msg);
    assert!(dirty, "monitor packet in monitor mode repaints");
    assert!(a.panels[0].last_monitor.is_some());
}

#[test]
fn apply_docker_packet_only_when_in_docker_mode() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    let payload = multitop_agent::proto::Payload::Docker {
        host: "h1".into(),
        rows: vec![],
    };
    let msg = Msg::Packet {
        panel: 0,
        gen: 0,
        epoch: 0,
        payload,
        dims: (80, 24),
    };
    // Not in Docker mode → not shown, but stored.
    let dirty = a.apply(msg);
    assert!(!dirty, "docker packet not shown when not in docker mode");
    assert!(a.panels[0].last_docker.is_some(), "but stored");
}

#[test]
fn apply_fetch_packet_only_when_in_fetch_mode() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    let payload = multitop_agent::proto::Payload::Fetch(FetchSnapshot::default());
    let msg = Msg::Packet {
        panel: 0,
        gen: 0,
        epoch: 0,
        payload,
        dims: (80, 24),
    };
    let dirty = a.apply(msg);
    assert!(!dirty, "fetch packet not shown when not in fetch mode");
    assert!(a.panels[0].last_fetch.is_some(), "but stored");
}

#[test]
fn apply_rejects_stale_epoch() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels_epoch = 5;

    let payload = multitop_agent::proto::Payload::Monitor(Snapshot::default());
    let msg = Msg::Packet {
        panel: 0,
        gen: 0,
        epoch: 3, // stale
        payload,
        dims: (80, 24),
    };
    let dirty = a.apply(msg);
    assert!(!dirty, "stale epoch rejected");
}

#[test]
fn apply_auxdone_completes_upgrade() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_test.toml"));

    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 7;

    let msg = Msg::AuxDone {
        panel: 0,
        gen: 7,
        note: Some("done".into()),
        success: true,
    };
    let dirty = a.apply(msg);
    assert!(dirty);
    assert_eq!(a.panels[0].upgrade_state, UpgradeState::DONE);
    assert!(
        a.panels[0].last_upgrade.iter().any(|l| l.contains("done")),
        "completion note in ring"
    );
}

#[test]
fn apply_auxbegin_in_upgrade_mode_does_not_replace_header() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Upgrade;

    let msg = Msg::AuxBegin {
        panel: 0,
        gen: 1,
        header: Some("Upgrade on h1".into()),
    };
    let dirty = a.apply(msg);
    // In Upgrade mode, AuxBegin returns false (header is composed by renderer).
    assert!(!dirty);
}

#[test]
fn apply_auxbegin_in_other_modes_shows_header() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Monitor;
    a.panels[0].gen = 1; // gen must match for accepts()

    let msg = Msg::AuxBegin {
        panel: 0,
        gen: 1,
        header: Some("Fetching...".into()),
    };
    let dirty = a.apply(msg);
    assert!(dirty, "AuxBegin in monitor mode shows header");
}

#[test]
fn apply_status_in_upgrade_mode_goes_to_ring() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Upgrade;

    let msg = Msg::Status {
        panel: 0,
        gen: 0,
        text: "sudo ready".into(),
    };
    let dirty = a.apply(msg);
    assert!(dirty);
    assert!(
        a.panels[0]
            .last_upgrade
            .iter()
            .any(|l| l.contains("sudo ready")),
        "status goes to ring in upgrade mode"
    );
}

#[test]
fn apply_status_in_monitor_mode_shows_body() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Monitor;

    let msg = Msg::Status {
        panel: 0,
        gen: 0,
        text: "connecting...".into(),
    };
    let dirty = a.apply(msg);
    assert!(dirty);
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("connecting")),
        "status shows body in monitor mode"
    );
}

#[tokio::test]
async fn vault_unlocked_sets_upgrade_modal() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let mut a = App::new(vec![test_server("h1")]);

    let dir = tempfile::tempdir().unwrap();
    let cfg = VaultConfig {
        vault_path: dir.path().join("vault.bin"),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        use_os_keychain: false,
    };
    let vault = Vault::new(cfg);
    vault.initialize("pw").await.unwrap();
    a.vault = Some(std::sync::Arc::new(vault));
    a.vault_state = VaultState::Locked;

    assert!(a.begin_password_unlock(), "locked");
    let epoch = a.vault_epoch;
    let unlocked = a
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password("pw")
        .unwrap();
    a.apply(Msg::VaultUnlocked {
        epoch,
        unlocked: Box::new(unlocked),
    });

    assert!(a.show_upgrade_modal(), "unlocked → upgrade modal");
}

#[test]
fn vault_unlock_failed_returns_to_prompt() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.vault_state = VaultState::Locked;

    a.apply(Msg::VaultUnlockFailed {
        epoch: 0,
        error: "wrong password".into(),
    });

    assert!(a.vault_password_error().is_some(), "error shown on prompt");
}

#[test]
fn previous_upgrade_interrupted_detects_interruption() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    a.last_update = Some(100);
    a.upgrade_started_at = Some(200); // started after last update
    assert!(a.previous_upgrade_interrupted());

    a.upgrade_started_at = Some(50); // started before last update
    assert!(!a.previous_upgrade_interrupted());
}

#[test]
fn running_upgrade_hosts_lists_in_flight() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1"), test_server("h2")]);
    a.panels[0].upgrade_state = UpgradeState::STARTED;

    let hosts = a.running_upgrade_hosts();
    assert_eq!(hosts, vec!["h1"]);
}

#[test]
fn filtered_indices_respects_query() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("web-01"), test_server("db-01")]);

    a.filter_query = "web".into();
    let idx = a.filtered_indices();
    assert_eq!(idx, vec![0]);

    a.filter_query.clear();
    let idx = a.filtered_indices();
    assert_eq!(idx, vec![0, 1]);
}
