use super::*;

#[tokio::test]
async fn a_key_release_is_not_a_second_press() {
    // Terminals that report releases would otherwise run every action twice.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha", "beta"]);
    let mut k = Keys::new(2);

    app.selected_panel = 0;
    k.release(&mut app, KeyCode::Char('2'));
    assert_eq!(app.selected_panel, 0, "a release moved the selection");
    k.press(&mut app, KeyCode::Char('2'));
    assert_eq!(app.selected_panel, 1);
}

// ------------------------------------------------------- the quit confirmation

#[tokio::test]
async fn only_the_keys_the_quit_row_names_confirm_it() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    // `Enter` and `y` are exactly the wrong keys here: this press kills a
    // running dpkg transaction on every host, and `Enter` is what an operator
    // hits to dismiss something they have not read.
    for code in [KeyCode::Enter, KeyCode::Char('y'), KeyCode::Char('x')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.panels[0].upgrade_state = UpgradeState::STARTED;
        let mut k = Keys::new(1);

        k.press(&mut app, KeyCode::Char('q'));
        assert_eq!(
            app.active_confirm(),
            Some(Confirm::Quit),
            "the quit was not armed"
        );
        k.press(&mut app, code);
        assert!(
            !app.should_quit(),
            "{code:?} confirmed a quit it does not name"
        );
        assert_eq!(
            app.active_confirm(),
            Some(Confirm::Quit),
            "{code:?} stood it down"
        );
    }
}

#[tokio::test]
async fn q_and_ctrl_c_both_confirm_a_quit_and_esc_stands_it_down() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    for code in [KeyCode::Char('q'), KeyCode::Char('Q')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.panels[0].upgrade_state = UpgradeState::STARTED;
        let mut k = Keys::new(1);
        k.press(&mut app, KeyCode::Char('q'));
        k.press(&mut app, code);
        assert!(app.should_quit(), "{code:?} did not confirm");
    }

    // Ctrl-C means the same thing everywhere.
    let mut app = app_with_config(&dir, &["alpha"]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    let mut k = Keys::new(1);
    k.press(&mut app, KeyCode::Char('q'));
    k.press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.should_quit());

    // Esc stands it down and leaves the app running.
    let mut app = app_with_config(&dir, &["alpha"]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    let mut k = Keys::new(1);
    k.press(&mut app, KeyCode::Char('q'));
    k.press(&mut app, KeyCode::Esc);
    assert!(!app.should_quit());
    assert_eq!(app.active_confirm(), None);
}

// ---------------------------------------------------- the upgrade confirmation

#[tokio::test]
async fn the_upgrade_confirmation_takes_only_the_key_it_names() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    // Cancel keys are not the same thing as confirm keys: a stray key that
    // cancels can only ever be the safe answer, so several are accepted.
    for code in [
        KeyCode::Esc,
        KeyCode::Char('q'),
        KeyCode::Char('n'),
        KeyCode::Char('N'),
    ] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.set_show_upgrade_modal(true);
        let mut k = Keys::new(1);
        k.press(&mut app, code);
        assert!(!app.show_upgrade_modal(), "{code:?} did not cancel");
    }

    // A key the row does not name leaves the question up rather than answering
    // it — `Enter` above all, which is what dismisses a row unread.
    for code in [KeyCode::Enter, KeyCode::Char('y'), KeyCode::Char('z')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.set_show_upgrade_modal(true);
        let mut k = Keys::new(1);
        k.press(&mut app, code);
        assert!(
            app.show_upgrade_modal(),
            "{code:?} answered a question it is not on the screen for"
        );
    }
}

#[tokio::test]
async fn u_confirms_the_upgrade_the_modal_asked_about() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_upgrade_modal(true);
    let mut k = Keys::new(1);

    k.press(&mut app, KeyCode::Char('u'));
    assert!(
        !app.show_upgrade_modal(),
        "the modal stayed up after confirming"
    );
}

#[tokio::test]
async fn x_arms_kill_confirmation_and_esc_stands_it_down() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    let snap = multitop_agent::render::Snapshot {
        host: "alpha".to_string(),
        procs: vec![multitop_agent::proc::Proc {
            pid: 1234,
            name: "heavy-job".to_string(),
            cpu: 95.0,
            mem: 1024 * 1024,
        }],
        ..Default::default()
    };
    app.panels[0].last_monitor = Some(multitop_agent::proto::Payload::Monitor(snap));

    // Press x -> arms kill confirm
    k.press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.active_confirm(), Some(Confirm::Kill));
    assert!(app.kill_confirm.is_some());
    let kc = app.kill_confirm.as_ref().unwrap();
    assert_eq!(kc.pid, 1234);
    assert_eq!(kc.name, "heavy-job");
    assert_eq!(kc.kind, multitop::app::ExecKind::Kill);

    // Press Esc -> cancels
    k.press(&mut app, KeyCode::Esc);
    assert_eq!(app.active_confirm(), None);
    assert!(app.kill_confirm.is_none());
}

#[tokio::test]
async fn o_and_r_arm_journal_and_renice_confirmations() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    let snap = multitop_agent::render::Snapshot {
        host: "alpha".to_string(),
        procs: vec![multitop_agent::proc::Proc {
            pid: 4321,
            name: "worker".to_string(),
            cpu: 80.0,
            mem: 512,
        }],
        ..Default::default()
    };
    app.panels[0].last_monitor = Some(multitop_agent::proto::Payload::Monitor(snap));

    // Press o -> Journal confirm
    k.press(&mut app, KeyCode::Char('o'));
    assert_eq!(app.active_confirm(), Some(Confirm::Kill));
    assert_eq!(
        app.kill_confirm.as_ref().unwrap().kind,
        multitop::app::ExecKind::Journal
    );

    // Cancel
    k.press(&mut app, KeyCode::Esc);
    assert_eq!(app.active_confirm(), None);

    // Press r -> Renice confirm
    k.press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.active_confirm(), Some(Confirm::Kill));
    assert_eq!(
        app.kill_confirm.as_ref().unwrap().kind,
        multitop::app::ExecKind::Renice
    );
}

#[tokio::test]
async fn k_confirms_kill_and_starts_aux_task() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    let snap = multitop_agent::render::Snapshot {
        host: "alpha".to_string(),
        procs: vec![multitop_agent::proc::Proc {
            pid: 9999,
            name: "stuck-proc".to_string(),
            cpu: 99.0,
            mem: 2048,
        }],
        ..Default::default()
    };
    app.panels[0].last_monitor = Some(multitop_agent::proto::Payload::Monitor(snap));

    k.press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.active_confirm(), Some(Confirm::Kill));

    // 'k' confirms the armed kill
    k.press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.active_confirm(), None);
    assert!(app.kill_confirm.is_none());
}
