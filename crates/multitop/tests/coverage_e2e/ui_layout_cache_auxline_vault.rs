use super::*;

// ===========================================================================
// ui.rs — visible / pane_window / notice_split
// ===========================================================================

#[test]
fn visible_shows_everything_when_it_fits() {
    let _g = isolate_keychain();

    let lines = vec!["a".into(), "b".into(), "c".into()];
    let (out, badge) = visible(&lines, 10, 1, 80, 0);
    assert_eq!(out.len(), 3);
    assert_eq!(badge, 0);
}

#[test]
fn visible_clamps_to_height() {
    let _g = isolate_keychain();

    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, _badge) = visible(&lines, 5, 1, 80, 0);
    assert!(out.len() <= 5);
}

#[test]
fn visible_scrolls_backwards() {
    let _g = isolate_keychain();

    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, badge) = visible(&lines, 10, 1, 80, 5);
    assert!(badge > 0, "scrolled back returns a badge");
    assert!(out.len() <= 10);
}

#[test]
fn visible_upgrade_composes_header_body_tail() {
    let _g = isolate_keychain();

    let header = vec!["Status: ready".into(), "Command: true".into()];
    let mut body = RingLines::new(100);
    body.push("output line 1".into());
    body.push("output line 2".into());
    let tail = vec!["notice: done".into()];

    let (out, badge) = visible_upgrade(&header, 2, &body, &tail, 10, 80, 0);
    assert_ne!(out, [] as [std::string::String; 0]);
    assert_eq!(badge, 0);
}

#[test]
fn visible_handles_zero_height() {
    let _g = isolate_keychain();

    let lines = vec!["a".into(), "b".into()];
    let (out, badge) = visible(&lines, 0, 1, 80, 0);
    assert_eq!(out, [] as [std::string::String; 0]);
    assert_eq!(badge, 0);
}

#[test]
fn visible_with_pinned_header() {
    let _g = isolate_keychain();

    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, badge) = visible(&lines, 5, 2, 80, 0);
    // Pinned header takes 2 rows, body gets the rest.
    assert!(out.len() <= 5);
    assert_eq!(badge, 0);
}

#[test]
fn visible_upgrade_with_empty_body() {
    let _g = isolate_keychain();

    let header = vec!["Status: ready".into()];
    let body = RingLines::new(100);
    let tail: Vec<String> = vec![];
    let (out, badge) = visible_upgrade(&header, 1, &body, &tail, 5, 80, 0);
    assert_ne!(out, [] as [std::string::String; 0]);
    assert_eq!(badge, 0);
}

#[test]
fn regions_splits_screen() {
    let _g = isolate_keychain();

    let area = Rect::new(0, 0, 80, 24);
    let (panel_areas, keybar) = regions(area, 4);
    assert_eq!(panel_areas.len(), 4);
    assert_eq!(keybar.height, 1);
}

#[test]
fn regions_single_panel() {
    let _g = isolate_keychain();

    let area = Rect::new(0, 0, 80, 24);
    let (panel_areas, _keybar) = regions(area, 1);
    assert_eq!(panel_areas.len(), 1);
    // Single panel gets the full width.
    assert_eq!(panel_areas[0].width, 80);
}

#[test]
fn agent_dims_computes_size() {
    let _g = isolate_keychain();

    let dims = agent_dims(
        Size {
            width: 120,
            height: 40,
        },
        2,
    );
    assert!(dims.0 >= multitop::ui::MIN_AGENT_COLS);
    assert!(dims.1 >= multitop::ui::MIN_AGENT_ROWS);
}

// ===========================================================================
// config.rs + fmt.rs (exercised via app methods)
// ===========================================================================

#[test]
fn toggle_fetch_then_stats_then_fetch_uses_cache() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    a.panels[0].last_fetch = Some(FetchSnapshot {
        user_host: "cached".into(),
        ..FetchSnapshot::default()
    });

    a.toggle_fetch((80, 24));
    assert!(a.panels[0].view.len() > 1, "fetch rendered from cache");

    a.switch_stats();
    assert_eq!(a.panels[0].mode, Mode::Monitor);

    // Re-enter fetch — should use cache again.
    a.toggle_fetch((80, 24));
    assert!(
        a.panels[0].view.len() > 1,
        "fetch rendered from cache on re-entry"
    );
}

#[test]
fn toggle_docker_then_stats_then_docker_uses_cache() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    a.panels[0].last_docker = Some(multitop_agent::proto::Payload::Docker {
        host: "h1".into(),
        rows: vec![multitop_agent::docker::Row {
            name: "c1".into(),
            status: "Up".into(),
            image: "nginx:latest".into(),
            cpu: "1%".into(),
            cpu_pct: 1.0,
            mem: "32M".into(),
            mem_bytes: 33_554_432,
        }],
    });

    a.toggle_docker((80, 24));
    assert!(a.panels[0].view.iter().any(|l| l.contains("c1")));

    a.switch_stats();
    a.toggle_docker((80, 24));
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("c1")),
        "cache reused"
    );
}

// ===========================================================================
// AuxLine handler — belongs to current view (fetch/docker error lines)
// ===========================================================================

#[test]
fn auxline_for_current_view_goes_to_view() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Fetch;
    a.panels[0].gen = 3;

    let msg = Msg::AuxLine {
        panel: 0,
        gen: 3,
        line: "fetch error".into(),
    };
    let dirty = a.apply(msg);
    assert!(dirty, "visible view AuxLine repaints");
    assert!(a.panels[0].view.iter().any(|l| l.contains("fetch error")));
}

#[test]
fn auxline_for_stale_gen_ignored() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Fetch;
    a.panels[0].gen = 3;

    let msg = Msg::AuxLine {
        panel: 0,
        gen: 99, // stale
        line: "stale".into(),
    };
    let dirty = a.apply(msg);
    assert!(!dirty, "stale gen AuxLine ignored");
}

// ===========================================================================
// Vault creation flow (app.rs)
// ===========================================================================

#[test]
fn begin_vault_creation_sets_state() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.vault = None;
    a.config_path = Some(std::env::temp_dir().join("cov_vault_create.toml"));

    let started = a.begin_vault_creation();
    assert!(started, "vault creation begins when no vault exists");
    assert!(a.vault_creating());
}

#[test]
fn begin_vault_creation_false_when_vault_exists() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.vault = Some(std::sync::Arc::new(multitop_vault::Vault::new(
        multitop_vault::VaultConfig {
            vault_path: std::env::temp_dir().join("cov_vault.bin"),
            argon2_params: None,
            use_os_keychain: false,
        },
    )));

    let started = a.begin_vault_creation();
    assert!(!started, "creation skipped when vault already exists");
}

#[test]
fn cancel_vault_biometrics_returns_to_locked() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };

    a.cancel_vault_biometric();
    assert!(matches!(a.vault_state, VaultState::Locked));
}

#[test]
fn cancel_vault_verify_returns_to_locked() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.vault_state = VaultState::Unlocking {
        awaiting_biometric: false,
    };

    a.cancel_vault_verify();
    assert!(matches!(a.vault_state, VaultState::Locked));
}
