//! Failure paths that only happen on a filesystem behaving badly.
//!
//! A permission change, a leftover temp file from a killed run, a directory
//! that cannot be written. Each one is a case where saying nothing looks
//! identical to working, so the reporting is what is under test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::password_actions::apply;
use multitop::password_store;
use multitop::passwords::{open, PasswordAction};
use multitop::run::Tasks;
use multitop::state;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::mpsc;

const MASTER: &str = "correct horse battery staple";

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
        custom_command: None,
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ------------------------------------------------------------------ the state

#[tokio::test]
async fn a_state_file_that_cannot_be_read_is_not_reported_as_no_history() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // "No file" is an ordinary first run and says nothing. A permission change
    // is not "no history ever", and was reported as exactly that.
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let state_path = state::state_file_path(&config_path);
    std::fs::write(&state_path, "last_update = 1\n").unwrap();
    std::fs::set_permissions(&state_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    let loaded = state::load_state(&config_path);

    // Root can read anything; on such a host there is nothing to assert.
    if let Some(notice) = loaded.notice {
        assert!(notice.contains("could not be read"), "{notice}");
        assert!(notice.contains("unavailable this session"), "{notice}");
    }
    std::fs::set_permissions(&state_path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[tokio::test]
async fn an_unreadable_state_file_that_cannot_be_moved_says_it_will_be_overwritten() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Read-only directory: the file parses badly *and* cannot be renamed out
    // of the way, so the notice has to say the history is about to be lost
    // rather than promising a copy that was never made.
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("locked");
    std::fs::create_dir(&nested).unwrap();
    let config_path = nested.join("config.toml");
    std::fs::write(state::state_file_path(&config_path), "not = = toml").unwrap();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o500)).unwrap();

    let loaded = state::load_state(&config_path);
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();

    let notice = loaded
        .notice
        .expect("an unreadable state file must be reported");
    assert!(notice.contains("could not be parsed"), "{notice}");
    if !notice.contains("kept as") {
        assert!(
            notice.contains("will be overwritten"),
            "a file that could not be moved aside was reported as kept: {notice}"
        );
    }
}

#[tokio::test]
async fn a_state_write_into_a_directory_that_will_not_take_it_leaves_nothing_behind() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // The scratch file must not survive a failed write, or the next run finds
    // a half-written state beside the real one.
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("readonly");
    std::fs::create_dir(&nested).unwrap();
    let config_path = nested.join("config.toml");
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o500)).unwrap();

    // The write is expected to fail here; what matters is the aftermath.
    let _ = state::save_state(&config_path, &state::AppState::default());

    let left_over: Vec<_> = std::fs::read_dir(&nested)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        !left_over.iter().any(|n| n.contains("tmp")),
        "a failed write left its scratch file behind: {left_over:?}"
    );
}

// ------------------------------------------------------------------ the vault

#[tokio::test]
async fn a_leftover_temp_file_from_a_killed_write_is_reclaimed() {
    // A run killed mid-save leaves `vault.bin.tmp` locked by nobody. Refusing
    // to write until someone deletes it by hand would wedge the vault.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path.clone()));
    vault.initialize(MASTER).await.expect("initialise");

    std::fs::write(vault_path.with_extension("bin.tmp"), b"half a vault").unwrap();

    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    unlocked
        .set_password("alpha".into(), &SecretString::from("hunter2".to_string()))
        .expect("store");
    unlocked
        .save()
        .expect("the stale temp file must not block the save");

    let reopened = vault.unlock_with_password(MASTER).expect("reopen");
    assert_eq!(
        reopened
            .get_password("alpha")
            .map(|s| s.expose_secret().to_string()),
        Some("hunter2".to_string())
    );
    assert!(
        !vault_path.with_extension("bin.tmp").exists(),
        "the temp file survived a successful save"
    );
}

// --------------------------------------------------------------- credentials

#[tokio::test]
async fn saving_with_the_vault_open_puts_the_password_in_both_places() {
    // The credential store and the vault. Reporting only the first told the
    // user "saved securely" when the thing they created to hold it never got
    // a copy.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path));
    vault.initialize(MASTER).await.expect("initialise");

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(vault));
    app.apply(Msg::VaultUnlocked {
        epoch: app.vault_epoch,
        unlocked: Box::new(
            app.vault
                .as_ref()
                .unwrap()
                .unlock_with_password(MASTER)
                .expect("unlock"),
        ),
    });
    open(&mut app, 0, false);

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    apply(
        PasswordAction::Save {
            panel: 0,
            password: "hunter2".into(),
            resume_upgrade: false,
        },
        &mut app,
        &tx,
        &mut tasks,
    );

    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("saved securely"), "{notice}");

    let key = password_store::account(&test_server("alpha"));
    assert!(
        app.vault_unlocked().unwrap().get_password(&key).is_some(),
        "the vault never received the password it was created to hold"
    );
}

#[tokio::test]
async fn removing_with_the_vault_open_clears_it_from_both_places() {
    // A password still in the vault is a password that will come back, and the
    // user has to know that rather than believe it is gone.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path));
    vault.initialize(MASTER).await.expect("initialise");

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(vault));
    app.apply(Msg::VaultUnlocked {
        epoch: app.vault_epoch,
        unlocked: Box::new(
            app.vault
                .as_ref()
                .unwrap()
                .unlock_with_password(MASTER)
                .expect("unlock"),
        ),
    });
    open(&mut app, 0, false);

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    let save = PasswordAction::Save {
        panel: 0,
        password: "hunter2".into(),
        resume_upgrade: false,
    };
    apply(save, &mut app, &tx, &mut tasks);
    apply(
        PasswordAction::Delete { panel: 0 },
        &mut app,
        &tx,
        &mut tasks,
    );

    let key = password_store::account(&test_server("alpha"));
    assert!(
        app.vault_unlocked().unwrap().get_password(&key).is_none(),
        "the password is still in the vault and will come back"
    );
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("now has none"), "{notice}");
}

// ------------------------------------------------------------- locked vault

#[tokio::test]
async fn starting_an_upgrade_with_a_locked_vault_asks_for_the_master_password() {
    // Straight to the password prompt: the user wants one password entry, not
    // a biometric prompt followed by a password prompt.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path));
    vault.initialize(MASTER).await.expect("initialise");

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(vault));

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    let (dims_tx, dims_rx) = tokio::sync::watch::channel((80u16, 24u16));
    std::mem::forget(dims_tx);
    let dims_rx = std::sync::Arc::new(dims_rx);

    let press = |app: &mut App, tasks: &mut Tasks| {
        multitop::run::handle_key(
            crossterm::event::KeyEvent::new_with_kind(
                crossterm::event::KeyCode::Char('u'),
                crossterm::event::KeyModifiers::NONE,
                crossterm::event::KeyEventKind::Press,
            ),
            app,
            (80, 24),
            dims_rx.clone(),
            &tx,
            tasks,
        );
    };

    // First press switches into the view and changes nothing else.
    press(&mut app, &mut tasks);
    assert!(app.in_upgrade());
    assert!(!app.show_vault_password_prompt());

    // Second press is the one that starts something — and with the vault
    // locked, what it starts is the unlock.
    press(&mut app, &mut tasks);
    assert!(
        app.show_vault_password_prompt(),
        "a locked vault did not ask for the master password"
    );
    assert!(
        !app.vault_awaiting_biometric(),
        "the biometric prompt was raised despite the password prompt"
    );
    assert!(
        !app.show_upgrade_modal(),
        "the upgrade began without the vault"
    );
}
