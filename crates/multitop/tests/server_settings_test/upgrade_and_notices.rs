use super::*;

/// Saving a password must not kill an upgrade already running on that host.
///
/// Found by the audit item 3 asks for. `mode == Upgrade` holds for the whole
/// session once `u` has been pressed, so any password save while the upgrade
/// view was showing took the "resume the upgrade" branch -- which replaces the
/// panel's task and aborts what was there. Every child is spawned with
/// `kill_on_drop`, so that killed the SSH session of a running `apt upgrade`,
/// interrupting a package transaction on the real machine and leaving the
/// remote lock file behind. `execute_cmds` refuses to abort a running upgrade
/// for exactly this reason; this path disagreed with it.
#[tokio::test]
async fn test_saving_a_password_does_not_restart_a_running_upgrade() {
    let _store_guard = setup_mock_store_async().await;

    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;
    app.panels[0].upgrade_state = multitop::panel::UpgradeState::STARTED;
    app.panels[0].upgrade_gen = app.panels[0].gen;
    let gen_before = app.panels[0].gen;

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "mypassword".to_string(),
            resume_upgrade: false,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    assert_eq!(
        app.panels[0].gen, gen_before,
        "no new generation -- a running upgrade must not be superseded"
    );
    assert!(
        tasks.upgrades[0].is_none(),
        "and no replacement task may be spawned over it"
    );
    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("mypassword"),
        "the password is still saved; only the restart is refused"
    );
}

/// The resume itself must keep working: an upgrade that stopped for want of a
/// password is exactly what it is for.
#[tokio::test]
async fn test_saving_a_password_resumes_a_finished_upgrade() {
    let _store_guard = setup_mock_store_async().await;

    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;
    app.panels[0].upgrade_state = multitop::panel::UpgradeState::DONE;

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "mypassword".to_string(),
            resume_upgrade: false,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    assert_eq!(
        app.panels[0].upgrade_state,
        multitop::panel::UpgradeState::STARTED,
        "a run that already finished must resume with the new password"
    );
    assert!(
        tasks.upgrades[0].is_some(),
        "and a task must be spawned for it"
    );
}

/// A server edit that could not be written must say so, and must keep saying so.
///
/// `ApplyServerEdit` is two operations: write the server list, then store the
/// password typed in the same editor. Both report through the one `notice`
/// line, and the second wrote over the first unconditionally -- so when
/// `save_servers` failed, "Could not save server configuration: ..." was
/// replaced by "Password saved securely in system credential store." before
/// anything drew it. The edit was discarded and the screen said success.
///
/// Class H: a failure reported as something else -- here, as the wrong
/// operation's success.
#[test]
fn a_failed_server_edit_is_not_reported_as_a_saved_password() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1")]);
    passwords::open(&mut app, 0, false);

    // A config path whose parent is a regular file: `save_servers` calls
    // `create_dir_all` on that parent and cannot succeed. This is the shape of
    // a read-only config directory or a full disk, without needing either.
    let blocker = std::env::temp_dir().join(format!(
        "multitop_blocker_{}_{:?}",
        std::process::id(),
        "edit"
    ));
    std::fs::write(&blocker, b"not a directory").unwrap();
    app.config_path = Some(blocker.join("config.toml"));

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::ApplyServerEdit {
            servers: vec![test_server("host1-renamed")],
            target_idx: 0,
            password: Some("new_password".to_string()),
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    let notice = app
        .password_manager
        .as_ref()
        .unwrap()
        .notice
        .clone()
        .unwrap_or_default();

    let _ = std::fs::remove_file(&blocker);

    assert!(
        notice.contains("Could not save server configuration"),
        "the edit failed and the panel must say so; it said: {notice:?}"
    );
    assert_eq!(
        app.panels[0].server.host, "host1",
        "a config write that failed must not leave the panels claiming the edit landed"
    );
}

/// The other half of the same class, on the path that succeeds.
///
/// Editing a server row while an upgrade is running interrupts that upgrade --
/// `write_servers` aborts it, because every generation just moved and nothing
/// could report on it any more. The panel says so, naming the host and the lock
/// file it may have left behind, which is the standing rule for anything that
/// interrupts a package transaction. Then the password half of the same edit
/// assigned `notice` and that warning was gone before anything drew it.
#[test]
fn a_server_edit_that_interrupts_an_upgrade_still_says_so_after_saving_the_password() {
    let _store_guard = setup_mock_store();
    let mut app = App::new(vec![test_server("host1")]);
    passwords::open(&mut app, 0, false);
    app.panels[0].upgrade_state = multitop::panel::UpgradeState::STARTED;

    let tmp_path = std::env::temp_dir().join(format!(
        "multitop_test_cfg_interrupt_{}.toml",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp_path);
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::ApplyServerEdit {
            servers: vec![test_server("host1")],
            target_idx: 0,
            password: Some("new_password".to_string()),
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    let notice = app
        .password_manager
        .as_ref()
        .unwrap()
        .notice
        .clone()
        .unwrap_or_default();

    let _ = std::fs::remove_file(&tmp_path);

    assert!(
        notice.contains("was interrupted"),
        "the interrupted upgrade must survive the password half of the same edit; \
         the panel said: {notice:?}"
    );
    assert!(
        notice.contains("upgrade.lock"),
        "and it must still name the lock file to remove; it said: {notice:?}"
    );
    assert!(
        notice.contains("Password saved"),
        "without losing what the password half had to say; it said: {notice:?}"
    );
}

/// A run started by "save the password and resume" must record that it started.
///
/// The confirmation modal wrote each host's `started_at`; this path set only
/// the global `upgrade_started_at` and hand-rolled its own state write, cloning
/// the per-host records unchanged. So a resumed run cut short by a crash or a
/// power loss left that host's record showing whatever was there before it --
/// a previous success, reported afterwards as `Ok`, or nothing at all,
/// reported as "never upgraded". The one record interrupted-run detection is
/// built on was the one record this path did not write.
#[tokio::test]
async fn a_resumed_upgrade_records_that_it_started() {
    let _store_guard = setup_mock_store_async().await;
    let server = test_server("host1");
    let mut app = App::new(vec![server.clone()]);
    app.panels[0].mode = Mode::Upgrade;

    let tmp_path = std::env::temp_dir().join(format!(
        "multitop_test_cfg_resume_{}.toml",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp_path);
    app.config_path = Some(tmp_path.clone());

    // A previous run that succeeded. If the new run is not recorded, this is
    // what the panel goes on saying while an upgrade is actually under way.
    let key = password_store::account(&server);
    app.host_updates.insert(
        key.clone(),
        multitop::state::HostUpdate {
            started_at: Some(1),
            finished_at: Some(2),
            success: true,
        },
    );

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "mypassword".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    let record = app.host_updates.get(&key).copied().unwrap_or_default();
    assert!(
        record.started_at.is_some(),
        "the resumed run must leave a started_at behind"
    );
    assert_eq!(
        record.finished_at, None,
        "and no finish time, which is what makes an interruption detectable"
    );
    assert_eq!(
        record.outcome(),
        multitop::state::Outcome::Interrupted,
        "a run that started and has not reported back is interrupted, \
         not the previous run's success"
    );

    // And it reached the disk, not just memory: a crash reads the file.
    let loaded = multitop::state::load_state(&tmp_path);
    assert_eq!(
        loaded
            .state
            .hosts
            .get(&key)
            .map(multitop::state::HostUpdate::outcome),
        Some(multitop::state::Outcome::Interrupted),
        "persisted, or a power loss has nothing to find"
    );

    let _ = std::fs::remove_file(&tmp_path);
    let _ = std::fs::remove_file(multitop::state::state_file_path(&tmp_path));
}
