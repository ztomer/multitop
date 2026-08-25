use super::*;

// ===========================================================================
// run.rs — event loop paths (via handle_key)
// ===========================================================================

#[test]
fn handle_key_quit_with_upgrade_arms_confirmation() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].upgrade_state = UpgradeState::STARTED;

    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    let mut tasks = Tasks::new(1);

    let key = KeyEvent::new_with_kind(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Press);
    handle_key(key, &mut a, (80, 24), Arc::new(dims_rx), &tx, &mut tasks);

    // Quit with upgrade in flight arms the confirmation.
    assert!(a.quit_armed());
}

// ===========================================================================
// panel.rs — RingLines + note + show_frame
// ===========================================================================

#[test]
fn ring_lines_wraps_and_slices() {
    let _g = isolate_keychain();
    let mut ring = RingLines::new(3);
    ring.push("a".into());
    ring.push("b".into());
    ring.push("c".into());
    ring.push("d".into()); // overwrites "a"

    assert_eq!(ring.len(), 3);
    let items: Vec<&str> = ring.iter().map(String::as_str).collect();
    assert_eq!(items, vec!["b", "c", "d"]);
}

#[test]
fn ring_lines_slice_out_of_range_yields_nothing() {
    let _g = isolate_keychain();
    let mut ring = RingLines::new(5);
    ring.push("x".into());
    assert_eq!(ring.slice(9, 3).count(), 0);
}

#[test]
fn ring_lines_set_cap_shrinks() {
    let _g = isolate_keychain();
    let mut ring = RingLines::new(10);
    for i in 0..5 {
        ring.push(format!("line{i}"));
    }
    ring.set_cap(2);
    assert_eq!(ring.len(), 2);
    assert_eq!(ring.get(0).map(String::as_str), Some("line3"));
}

#[test]
fn panel_note_dedup() {
    let _g = isolate_keychain();
    let mut p = Panel::new(test_server("h1"));
    p.note("hello".into());
    p.note("hello".into()); // duplicate
    assert_eq!(p.notes.iter().filter(|n| *n == "hello").count(), 1);
}

#[test]
fn panel_note_bounded() {
    let _g = isolate_keychain();
    let mut p = Panel::new(test_server("h1"));
    for i in 0..10 {
        p.note(format!("note{i}"));
    }
    // MAX_NOTES = 4, so only the last 4 survive.
    assert!(p.notes.len() <= 4);
}

#[test]
fn panel_show_body_reserves_row0() {
    let _g = isolate_keychain();
    let mut p = Panel::new(test_server("h1"));
    p.show_body(vec!["line1".into(), "line2".into()]);
    assert_eq!(p.view[0], "", "row 0 reserved for banner");
    assert_eq!(p.view[1], "line1");
}

#[test]
fn panel_show_last_frame_fallback() {
    let _g = isolate_keychain();
    let mut p = Panel::new(test_server("h1"));
    p.show_last_frame();
    assert!(p.view.iter().any(|l| l.contains("waiting")));
}

// ===========================================================================
// upgrade_view.rs — header rendering
// ===========================================================================

#[test]
fn upgrade_pane_header_running_state() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;

    let header = a.upgrade_pane_header(0);
    let text = header.join("\n");
    assert!(text.contains("running") || text.contains("in progress"));
}

#[test]
fn upgrade_pane_header_not_configured() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    }]);
    a.panels[0].mode = Mode::Upgrade;

    let header = a.upgrade_pane_header(0);
    let text = header.join("\n");
    assert!(text.contains("not configured"));
}

// ===========================================================================
// confirm_upgrade + run_upgrade paths
// ===========================================================================

#[test]
fn confirm_upgrade_runs_configured_hosts() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_confirm.toml"));
    a.panels[0].mode = Mode::Upgrade;

    let cmds = a.confirm_upgrade();
    assert!(!cmds.is_empty(), "upgrade commands scheduled");
    assert_eq!(a.panels[0].upgrade_state, UpgradeState::STARTED);
}

#[test]
fn confirm_upgrade_skips_hosts_without_cmd() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    }]);
    a.config_path = Some(std::env::temp_dir().join("cov_skip.toml"));
    a.panels[0].mode = Mode::Upgrade;

    let cmds = a.confirm_upgrade();
    assert!(cmds.is_empty(), "no commands for unconfigured hosts");
    assert_eq!(a.panels[0].upgrade_state, UpgradeState::DONE);
}

#[test]
fn note_nothing_to_upgrade_says_so() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    }]);

    a.note_nothing_to_upgrade();
    assert!(
        a.panels[0]
            .last_upgrade
            .iter()
            .any(|l| l.contains("nothing to run")),
        "note says what the problem is"
    );
}

// ===========================================================================
// theme + banner cycling
// ===========================================================================

#[test]
fn cycle_theme_wraps() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    let n = multitop_agent::color::THEMES.len();
    for _ in 0..=n {
        a.cycle_theme();
    }
    // Didn't panic, index wrapped.
    assert!(a.theme_idx < n);
}

#[test]
fn cycle_banner_style_wraps() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);
    for _ in 0..10 {
        a.cycle_banner_style();
    }
    // Didn't panic.
}

// ===========================================================================
// password_actions.rs — apply() dispatch
// ===========================================================================

#[test]
fn password_action_apply_servers_replaces_panels() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_apply.toml"));

    let new_servers = vec![test_server("h2"), test_server("h3")];
    let action = PasswordAction::ApplyServers(new_servers);

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut tasks = Tasks::new(1);
    apply(action, &mut a, &tx, &mut tasks);

    assert_eq!(a.panels.len(), 2);
    assert_eq!(a.panels[0].server.host, "h2");
}

#[test]
fn password_action_delete_removes_password() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.panels[0].sudo_password = Some("secret".into());
    a.panels[0].password_saved = true;

    let action = PasswordAction::Delete { panel: 0 };
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut tasks = Tasks::new(1);
    apply(action, &mut a, &tx, &mut tasks);

    assert!(a.panels[0].sudo_password.is_none());
    assert!(!a.panels[0].password_saved);
}

#[test]
fn password_action_save_stores_password() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);
    a.config_path = Some(std::env::temp_dir().join("cov_save.toml"));

    let action = PasswordAction::Save {
        panel: 0,
        password: "my-password".into(),
        resume_upgrade: false,
    };
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut tasks = Tasks::new(1);
    apply(action, &mut a, &tx, &mut tasks);

    assert_eq!(a.panels[0].sudo_password.as_deref(), Some("my-password"));
    assert!(a.panels[0].password_saved);
    // When stored, the vault also gets the password (if unlocked).
    // No vault here — keychain only.
    let _ = SecretString::from("unused");
}

// ===========================================================================
// modals.rs — Waiting is constructed and used by the app; verify it exists
// and has the right Debug representation.
// ===========================================================================

#[test]
fn waiting_variants_exist() {
    let _g = isolate_keychain();

    let bio = Waiting::Biometric;
    let verifying = Waiting::Verifying;
    let creating = Waiting::Creating;

    // Waiting implements Debug.
    assert!(format!("{bio:?}").contains("Biometric"));
    assert!(format!("{verifying:?}").contains("Verifying"));
    assert!(format!("{creating:?}").contains("Creating"));
}

// ===========================================================================
// tasks.rs — tested via spawn_upgrade integration + public exports
// (painted_states, marker, is_sudo_help are private; tested through their
//  effects in the upgrade_loop_e2e tests and via the public test exports below)
// ===========================================================================

#[test]
fn sudo_sentinels_are_exported() {
    let _g = isolate_keychain();
    // The sentinels themselves are public and used by the upgrade handshake.
    assert_eq!(
        multitop::ssh::SUDO_FAILED_SENTINEL,
        "__multitop_sudo_failed__"
    );
    assert_eq!(multitop::ssh::LOCK_HELD_SENTINEL, "__multitop_lock_held__");
}

#[test]
fn upgrade_lock_code_is_exported() {
    let _g = isolate_keychain();
    // Exit codes that the upgrade wrapper can return.
    assert_eq!(multitop::ssh::SUDO_FAILED_CODE, 111);
    assert_eq!(multitop::ssh::LOCK_HELD_CODE, 125);
}
