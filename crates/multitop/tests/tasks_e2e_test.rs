//! Task spawning integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;
use multitop::state;
use multitop::tasks::spawn_upgrade;
use tokio::sync::mpsc;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some("echo test".to_string()),
    }
}

/// Reset the process-global mock store, holding the test guard so a
/// concurrently running test cannot be wiped out mid-run. Keep the returned
/// guard alive for the whole test body.
async fn enable_mock_store() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

#[tokio::test]
async fn test_spawn_upgrade_generation_tracking() {
    let _store_guard = enable_mock_store().await;
    let server = test_server("127.0.0.1");
    let (tx, mut rx) = mpsc::channel::<Msg>(100);

    let handle = spawn_upgrade(0, 42, server.clone(), None, tx.clone());

    let msg = rx.recv().await.unwrap();
    match msg {
        Msg::AuxBegin {
            panel: 0, gen: 42, ..
        } => {}
        other => panic!("Expected AuxBegin with gen=42, got {other:?}"),
    }

    // Not `let _ =`: a panic inside the spawned task arrives here as a join
    // error, and swallowing it leaves the test to fail later for some other
    // reason -- or to pass. The task is the thing under test.
    handle.await.expect("the spawned task must not panic");
}

#[tokio::test]
async fn test_spawn_upgrade_sets_mode_and_state() {
    let _store_guard = enable_mock_store().await;
    let mut app = App::new(vec![test_server("127.0.0.1")]);
    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(1);

    // Use password_actions::apply to trigger upgrade through the normal path
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    // Panel should be in Upgrade mode with STARTED state
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);
    assert_eq!(app.panels[0].upgrade_gen, 1);
}

#[tokio::test]
async fn test_spawn_upgrade_saves_state_file() {
    let _store_guard = enable_mock_store().await;
    let mut app = App::new(vec![test_server("127.0.0.1")]);
    let tmp_path =
        std::env::temp_dir().join(format!("multitop_test_state_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());

    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    // State file should be saved with upgrade_started_at
    let state_obj = state::load_state(&tmp_path);
    assert!(state_obj.state.upgrade_started_at.is_some());

    let _ = std::fs::remove_file(tmp_path);
}

#[tokio::test]
async fn test_task_cancellation_on_panel_switch() {
    let _store_guard = enable_mock_store().await;
    let mut app = App::new(vec![test_server("127.0.0.1"), test_server("127.0.0.2")]);
    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(2);

    // Start upgrade on panel 0
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    assert!(tasks.upgrades[0].is_some());

    // Start upgrade on panel 1 - should NOT cancel panel 0's task
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 1,
            password: "test_pass2".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    // Panel 0's task should still be running
    assert!(tasks.upgrades[0].is_some());

    // Panel 1 should have new task
    assert!(tasks.upgrades[1].is_some());

    // Saving a password again for a host that is already mid-upgrade must
    // change nothing about the run.
    //
    // This block used to assert the opposite -- "this SHOULD cancel panel 0's
    // old task" -- and so was pinning a defect rather than a requirement. The
    // spawn it was asserting replaces the panel's task and aborts what was
    // there, and every child is spawned with `kill_on_drop`, so the behaviour
    // this test protected was: saving a password kills the SSH session of a
    // running `apt upgrade`, interrupting a package transaction on the real
    // machine and leaving the remote lock file behind. `execute_cmds` refuses
    // to abort a running upgrade for exactly that reason.
    //
    // The resume path is for an upgrade that *stopped* for want of a password.
    // A run that is still going does not need resuming.
    let gen_before = app.panels[0].gen;

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "test_pass3".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    assert!(
        tasks.upgrades[0].is_some(),
        "panel 0's running upgrade must still be there"
    );
    assert_eq!(
        app.panels[0].gen, gen_before,
        "and must not have been superseded by a new generation"
    );
    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("test_pass3"),
        "the new password is still stored for the next run"
    );
}

#[tokio::test]
async fn test_concurrent_upgrade_generations_isolated() {
    let _store_guard = enable_mock_store().await;
    let server = test_server("127.0.0.1");
    let (tx, mut rx) = mpsc::channel::<Msg>(100);

    // Spawn upgrade with gen=1
    let handle1 = spawn_upgrade(0, 1, server.clone(), None, tx.clone());

    // Spawn upgrade with gen=2 (simulates panel switch)
    let handle2 = spawn_upgrade(0, 2, server.clone(), None, tx.clone());

    let mut gen1_done = false;
    let mut gen2_done = false;

    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::AuxDone { gen, success, .. } if gen == 1 && success => gen1_done = true,
            Msg::AuxDone { gen, success, .. } if gen == 2 && success => gen2_done = true,
            _ => {}
        }
        if gen1_done && gen2_done {
            break;
        }
    }

    let _ = handle1.await;
    let _ = handle2.await;

    assert!(gen1_done);
    assert!(gen2_done);
}

/// Switching views during an upgrade must not lose the upgrade's handle.
///
/// The two used to share one slot, with a flag saying which of them a view
/// switch may abort. The flag was obeyed -- the upgrade kept running -- and the
/// handle was dropped anyway, because `replace` hands back what was there and
/// the "do not abort" branch simply let it fall. Nothing tracked the upgrade
/// after that: `abort_all` could not reach it when the user quit, so the SSH
/// session it owns survived the quit that promised to stop it.
#[tokio::test]
async fn a_view_switch_during_an_upgrade_keeps_the_upgrade_tracked() {
    let _store_guard = enable_mock_store().await;
    let servers = vec![test_server("127.0.0.1")];
    let mut app = App::new(servers);
    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(1);

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "pw".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );
    assert!(tasks.upgrades[0].is_some(), "the upgrade must have started");

    // `f` -- Fetch -- while that upgrade is running.
    let (_dims_tx, dims_rx) = tokio::sync::watch::channel((80u16, 24u16));
    multitop::run::handle_key(
        crossterm::event::KeyEvent::new_with_kind(
            crossterm::event::KeyCode::Char('f'),
            crossterm::event::KeyModifiers::NONE,
            crossterm::event::KeyEventKind::Press,
        ),
        &mut app,
        (80, 24),
        std::sync::Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );

    assert!(
        tasks.aux[0].is_some(),
        "the fetch must have started in the view slot"
    );
    assert!(
        tasks.upgrades[0].is_some(),
        "and the running upgrade must still be tracked, or nothing can stop it"
    );
}

/// stderr is read to its own end, not to stdout's.
///
/// The two pipes close together when the child exits, so which of them
/// `select!` polled first decided whether the contents of the stderr pipe were
/// read or dropped -- and stderr is where the reason lives. Here stdout is
/// closed first on purpose, which turns that race into a certainty: the old
/// loop stopped at the first `Ok(None)` from stdout and never came back for
/// what stderr had to say.
#[tokio::test]
async fn stderr_is_still_read_after_stdout_has_closed() {
    let _store_guard = enable_mock_store().await;
    let server = Server {
        host: "localhost".to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: Some(
            "exec 1>&-; sleep 0.2; printf 'the actual reason\\n' >&2; exit 3".to_string(),
        ),
    };
    let (tx, mut rx) = mpsc::channel::<Msg>(100);

    let handle = spawn_upgrade(0, 1, server, None, tx);
    let mut saw_reason = false;
    let mut note = None;
    let collect = async {
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::AuxLine { line, .. } => {
                    if line.contains("the actual reason") {
                        saw_reason = true;
                    }
                }
                Msg::AuxDone { note: n, .. } => {
                    note = n;
                    break;
                }
                _ => {}
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), collect)
        .await
        .expect("the upgrade task must finish");
    handle.abort();

    assert!(
        saw_reason,
        "the command's stderr must reach the panel; the run reported only {note:?}"
    );
}

/// Editing the server list moves every generation, so nothing a running upgrade
/// sends is ever accepted again. Leaving it running means the transaction
/// carries on with nothing able to show it, and is then killed without a word
/// when the app quits. It is stopped here instead, and said out loud.
#[tokio::test]
async fn a_server_edit_stops_running_upgrades_and_says_so() {
    let _store_guard = enable_mock_store().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![test_server("127.0.0.1"), test_server("127.0.0.2")];
    let mut app = App::new(servers.clone());
    app.config_path = Some(dir.path().join("config.toml"));
    multitop::passwords::open(&mut app, 0, false);

    let (tx, _rx) = mpsc::channel::<Msg>(100);
    let mut tasks = multitop::run::Tasks::new(2);

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "pw".to_string(),
            resume_upgrade: true,
        },
        &mut app,
        &tx,
        &mut tasks,
    );
    assert!(tasks.upgrades[0].is_some(), "the upgrade must have started");
    assert_eq!(app.panels[0].upgrade_state, UpgradeState::STARTED);

    // The question asked before the key that does it must name the run.
    let armed = multitop::passwords::handle_key(&mut app, crossterm::event::KeyCode::Char('d'));
    assert_eq!(armed, multitop::passwords::PasswordAction::None);
    let asked = app
        .password_manager
        .as_ref()
        .unwrap()
        .notice
        .clone()
        .unwrap_or_default();
    assert!(
        asked.contains("interrupts the upgrade"),
        "the confirmation must say what it is about to interrupt, got {asked:?}"
    );

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::ApplyServers(vec![servers[1].clone()]),
        &mut app,
        &tx,
        &mut tasks,
    );

    assert!(
        tasks.upgrades.iter().all(Option::is_none),
        "no upgrade may still be running once nothing can report on it"
    );
    let told = app
        .password_manager
        .as_ref()
        .unwrap()
        .notice
        .clone()
        .unwrap_or_default();
    assert!(
        told.contains("was interrupted") && told.contains("upgrade.lock"),
        "and the operator must be told which run stopped and what it may have \
         left behind, got {told:?}"
    );
}
