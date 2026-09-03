//! A real vault, created and opened by the app the way `u` does it.
//!
//! Everything else about the vault is tested against its own crate; this is
//! the seam between the two — the app creating a vault, seeding it from the
//! panels, unlocking it through the key path, and reading a host's password
//! back out. Each of those steps used to be reachable only by a person typing
//! a master password into a running TUI.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::{mpsc, watch};

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

/// The mock credential store also selects the cheap Argon2id parameters, so a
/// vault here costs milliseconds rather than a quarter of system RAM.
async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Create a vault on disk and hand back its path plus a handle to it.
async fn make_vault(dir: &tempfile::TempDir) -> (std::path::PathBuf, Arc<multitop_vault::Vault>) {
    let config_path = dir.path().join("config.toml");
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path));
    vault
        .initialize(MASTER)
        .await
        .expect("initialise the vault");
    (config_path, Arc::new(vault))
}

fn press(app: &mut App, code: KeyCode, tx: &mpsc::Sender<Msg>, tasks: &mut Tasks) {
    let (dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    std::mem::forget(dims_tx);
    handle_key(
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press),
        app,
        (80, 24),
        Arc::new(dims_rx),
        tx,
        tasks,
    );
}

fn type_str(app: &mut App, s: &str, tx: &mpsc::Sender<Msg>, tasks: &mut Tasks) {
    for c in s.chars() {
        press(app, KeyCode::Char(c), tx, tasks);
    }
}

// ---------------------------------------------------------------- create

#[tokio::test]
async fn the_creation_prompt_makes_a_vault_that_opens_with_what_was_typed() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(config_path.clone());
    assert!(app.begin_vault_creation());

    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    type_str(&mut app, MASTER, &tx, &mut tasks);
    press(&mut app, KeyCode::Enter, &tx, &mut tasks);
    assert!(
        app.vault_create_in_flight(),
        "Enter did not start the create"
    );

    let msg = tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv())
        .await
        .expect("the create must report back")
        .expect("a message must arrive");

    match msg {
        Msg::VaultCreated { .. } => {}
        Msg::VaultCreateFailed { error, .. } => panic!("create failed: {error}"),
        other => panic!("unexpected message: {other:?}"),
    }
    assert!(
        dir.path().join("vault.bin").exists(),
        "no vault file was written"
    );

    // And the file really opens with that password, not merely reports success.
    let vault =
        multitop_vault::Vault::new(multitop::vault::config_for(dir.path().join("vault.bin")));
    assert!(vault.unlock_with_password(MASTER).is_ok());
    assert!(
        vault.unlock_with_password("not the master").is_err(),
        "the vault opened with the wrong password"
    );
}

// ---------------------------------------------------------------- unlock

#[tokio::test]
async fn the_unlock_prompt_opens_the_vault_and_reports_the_result() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_path, vault) = make_vault(&dir).await;

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(config_path);
    app.vault = Some(vault);
    app.set_show_vault_password_prompt(true);

    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    type_str(&mut app, MASTER, &tx, &mut tasks);
    press(&mut app, KeyCode::Enter, &tx, &mut tasks);
    assert!(
        app.vault_verifying(),
        "the unlock was not handed off the event loop"
    );

    let msg = tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv())
        .await
        .expect("the unlock must report back")
        .expect("a message must arrive");
    let Msg::VaultUnlocked { .. } = msg else {
        panic!("expected an unlock, got {msg:?}");
    };
    assert!(app.apply(msg), "the unlock did not change the screen");
    assert!(app.vault_unlocked().is_some());
}

#[tokio::test]
async fn a_wrong_master_password_is_reported_and_leaves_the_prompt_up() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_path, vault) = make_vault(&dir).await;

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(config_path);
    app.vault = Some(vault);
    app.set_show_vault_password_prompt(true);

    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    type_str(&mut app, "wrong", &tx, &mut tasks);
    press(&mut app, KeyCode::Enter, &tx, &mut tasks);

    let msg = tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv())
        .await
        .expect("the attempt must report back")
        .expect("a message must arrive");
    let Msg::VaultUnlockFailed { .. } = msg else {
        panic!("expected a failure, got {msg:?}");
    };
    assert!(app.apply(msg));
    assert!(
        app.show_vault_password_prompt(),
        "a failed unlock left the user with no way to try again"
    );
    assert!(app.vault_password_error().is_some(), "no reason was shown");
}

// -------------------------------------------------------------- passwords

#[tokio::test]
async fn a_password_written_to_the_vault_comes_back_out_by_the_same_key() {
    // Two spellings of the host key is how a saved password becomes
    // unreachable, so the write and the read go through the app's own helper.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (_config_path, vault) = make_vault(&dir).await;

    let server = test_server("alpha");
    let key = password_store::account(&server);
    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    unlocked
        .set_password(key, &SecretString::from("hunter2".to_string()))
        .expect("store the password");
    unlocked.save().expect("save the vault");

    // A fresh panel with no password of its own picks it up.
    let mut panel = multitop::panel::Panel::new(server.clone());
    multitop::vault::try_load_vault_password(&mut panel, &unlocked);
    assert_eq!(panel.sudo_password.as_deref(), Some("hunter2"));

    // One that already has a password is left alone: a session password the
    // user just typed must not be replaced by an older stored one.
    let mut has_own = multitop::panel::Panel::new(server);
    has_own.sudo_password = Some("typed-just-now".into());
    multitop::vault::try_load_vault_password(&mut has_own, &unlocked);
    assert_eq!(has_own.sudo_password.as_deref(), Some("typed-just-now"));

    // And it survives a reopen, which is what "saved" has to mean.
    let reopened = vault.unlock_with_password(MASTER).expect("reopen");
    assert_eq!(
        reopened
            .get_password(&password_store::account(&test_server("alpha")))
            .map(|s| s.expose_secret().to_string()),
        Some("hunter2".to_string())
    );
}

#[tokio::test]
async fn a_new_vault_is_seeded_with_the_passwords_this_session_already_has() {
    // Otherwise the vault comes into existence empty right after the user
    // believes they have just put their passwords into it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_path, vault) = make_vault(&dir).await;

    let mut app = App::new(vec![test_server("alpha"), test_server("beta")]);
    app.config_path = Some(config_path);
    app.panels[0].sudo_password = Some("alpha-secret".into());
    // The second panel has none, so it contributes nothing rather than an
    // empty entry.
    app.vault = Some(vault.clone());
    app.vault_state = VaultState::Unlocked {
        vault: Box::new(vault.unlock_with_password(MASTER).expect("unlock")),
        awaiting_biometric: false,
    };

    app.apply(Msg::VaultCreated {
        epoch: app.vault_epoch,
        unlocked: Box::new(vault.unlock_with_password(MASTER).expect("unlock")),
    });

    let unlocked = app.vault_unlocked().expect("the vault must be open");
    let hosts = unlocked.hosts();
    assert!(
        hosts.contains(&password_store::account(&test_server("alpha"))),
        "the session password was not carried into the new vault: {hosts:?}"
    );
    assert!(
        !hosts.contains(&password_store::account(&test_server("beta"))),
        "a host with no password got an entry anyway: {hosts:?}"
    );
}

#[tokio::test]
async fn a_removed_password_is_gone_from_the_vault_and_the_rest_stay() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (_config_path, vault) = make_vault(&dir).await;

    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    for host in ["alpha", "beta"] {
        unlocked
            .set_password(
                password_store::account(&test_server(host)),
                &SecretString::from(format!("{host}-secret")),
            )
            .expect("store");
    }
    unlocked.save().expect("save");

    let removed = unlocked
        .remove_password(&password_store::account(&test_server("alpha")))
        .expect("remove");
    assert!(removed, "the entry was reported as absent");
    unlocked.save().expect("save");

    let reopened = vault.unlock_with_password(MASTER).expect("reopen");
    assert!(reopened
        .get_password(&password_store::account(&test_server("alpha")))
        .is_none());
    assert!(reopened
        .get_password(&password_store::account(&test_server("beta")))
        .is_some());

    // Removing what is not there says so rather than failing.
    let mut again = reopened;
    assert!(!again
        .remove_password(&password_store::account(&test_server("alpha")))
        .expect("remove"));
}

// ------------------------------------------------------------------ rotate

#[tokio::test]
async fn changing_the_master_password_leaves_the_contents_reachable() {
    // The vault key is unchanged by a rotation; only the password that unwraps
    // it moves. If the contents did not survive, every stored password would
    // be lost by the act of improving the master password.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (_config_path, vault) = make_vault(&dir).await;

    let key = password_store::account(&test_server("alpha"));
    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    unlocked
        .set_password(key.clone(), &SecretString::from("hunter2".to_string()))
        .expect("store");
    unlocked.save().expect("save");
    drop(unlocked);

    vault
        .change_password(MASTER, "a different master password")
        .expect("rotate");

    assert!(
        vault.unlock_with_password(MASTER).is_err(),
        "the old password still opens the vault"
    );
    let reopened = vault
        .unlock_with_password("a different master password")
        .expect("the new password must open it");
    assert_eq!(
        reopened
            .get_password(&key)
            .map(|s| s.expose_secret().to_string()),
        Some("hunter2".to_string()),
        "rotation lost the stored passwords"
    );
}

#[tokio::test]
async fn rotating_with_the_wrong_current_password_changes_nothing() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (_config_path, vault) = make_vault(&dir).await;

    assert!(vault.change_password("not the master", "new one").is_err());
    assert!(
        vault.unlock_with_password(MASTER).is_ok(),
        "a refused rotation still moved the password"
    );
}

// ---------------------------------------------------------- an open vault

#[tokio::test]
async fn an_open_vault_never_prints_what_it_holds() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let (_config_path, vault) = make_vault(&dir).await;

    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    unlocked
        .set_password(
            "alpha".to_string(),
            &SecretString::from("super-secret-value".to_string()),
        )
        .expect("store");

    let rendered = format!("{unlocked:?}");
    assert!(
        !rendered.contains("super-secret-value"),
        "the debug output leaked a stored password: {rendered}"
    );
    assert!(
        !rendered.contains(MASTER),
        "the debug output leaked the master password"
    );
    // It still says enough to be useful.
    assert!(rendered.contains("alpha"), "{rendered}");
}
