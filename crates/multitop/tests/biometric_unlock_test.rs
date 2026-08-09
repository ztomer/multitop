//! One touch, or one password, and never both.
//!
//! The vault has two ways in. The rule is that the user is asked for exactly
//! one of them, and which one is decided *before* anything is drawn: a machine
//! that can open the vault with Touch ID gets the system prompt and nothing
//! else, and a machine that cannot gets the master password prompt and nothing
//! else. A biometric wait that was always going to fall through to typing is
//! the defect this file exists to stop coming back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{handle_key, spawn_biometric_unlock, Tasks};
use multitop_vault::{Vault, VaultConfig};
use ratatui::backend::TestBackend;
use tokio::sync::{mpsc, watch};

const MASTER: &str = "correct horse battery staple";

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
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

fn drawn(app: &mut App) -> String {
    let backend = TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| multitop::ui::draw(f, app))
        .expect("the frame must draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

/// A vault on disk with a password wrapper and nothing else.
async fn password_only_vault(dir: &std::path::Path) -> Arc<Vault> {
    let path = dir.join("vault.bin");
    let vault = Vault::new(multitop::vault::config_for(path));
    vault.initialize(MASTER).await.expect("initialise");
    Arc::new(vault)
}

// ------------------------------------------------------------ can it be touched

#[tokio::test]
async fn a_vault_kept_out_of_the_os_keychain_cannot_be_opened_by_touch() {
    // The enclave wrapper is written through the OS keychain. A vault that was
    // told not to use it has no wrapper to unwrap, whatever sensor the machine
    // has.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault = password_only_vault(dir.path()).await;
    assert!(!vault.biometric_available());
}

#[tokio::test]
async fn a_vault_with_no_enclave_wrapper_cannot_be_opened_by_touch_either() {
    // Keychain use permitted this time, so on a machine with a Secure Enclave
    // the platform check passes and the *header* is what decides. This is the
    // case that matters: a vault copied from another machine, or made before
    // the enclave key was bound, has an Argon2id wrapper and no other.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let made = Vault::new(multitop::vault::config_for(path.clone()));
    made.initialize(MASTER).await.expect("initialise");

    let reopened = Vault::new(VaultConfig {
        vault_path: path,
        argon2_params: None,
        use_os_keychain: true,
    });
    assert!(
        !reopened.biometric_available(),
        "a vault with no enclave wrapper offered a touch that cannot work"
    );
}

#[tokio::test]
async fn a_vault_file_that_is_gone_is_not_offered_as_touchable() {
    let _g = isolate().await;
    let vault = Vault::new(VaultConfig {
        vault_path: std::path::PathBuf::from("/no/such/vault.bin"),
        argon2_params: None,
        use_os_keychain: true,
    });
    assert!(!vault.biometric_available());
}

// --------------------------------------------------------------- the routing

#[tokio::test]
async fn a_vault_that_cannot_be_touched_asks_for_the_master_password_instead() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(password_only_vault(dir.path()).await);
    app.vault_state = VaultState::Locked;

    assert!(
        app.begin_vault_unlock().is_none(),
        "a biometric attempt was started against a vault with no enclave wrapper"
    );
    assert!(app.show_vault_password_prompt(), "no prompt went up at all");
    assert!(
        !app.vault_awaiting_biometric(),
        "the user was put in front of a touch prompt as well"
    );
}

#[tokio::test]
async fn there_is_nothing_to_unlock_when_the_vault_is_not_locked() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault = Some(password_only_vault(dir.path()).await);

    // A vault already being opened is not started again.
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: false,
    };
    assert!(app.begin_vault_unlock().is_none());
    assert!(!app.show_vault_password_prompt());

    // And an already-open one is not re-asked for.
    app.vault_state = VaultState::Unlocked {
        vault: Box::new(
            app.vault
                .as_ref()
                .unwrap()
                .unlock_with_password(MASTER)
                .expect("unlock"),
        ),
        awaiting_biometric: false,
    };
    assert!(app.begin_vault_unlock().is_none());
    assert!(!app.show_vault_password_prompt());
}

#[tokio::test]
async fn no_vault_behind_the_state_starts_nothing() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault_state = VaultState::Locked;
    assert!(app.vault.is_none());
    assert!(app.begin_vault_unlock().is_none());
    assert!(!app.show_vault_password_prompt());
}

#[tokio::test]
async fn starting_an_unlock_retires_whatever_was_in_flight() {
    // The epoch bump is load-bearing: an answer from an earlier attempt must
    // not open a vault the user has moved on from.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault = Some(password_only_vault(dir.path()).await);
    app.vault_state = VaultState::Locked;
    let before = app.vault_epoch;

    assert!(app.begin_vault_unlock().is_none());
    assert_ne!(app.vault_epoch, before, "a stale attempt was left current");
    assert!(!app.vault_epoch_current(before));
}

// ------------------------------------------------------------- through the app

#[tokio::test]
async fn pressing_u_twice_asks_for_one_credential_and_only_one() {
    // The user's own path, and the shape of the original complaint: two `u`
    // presses -- the first enters the view, the second starts -- and exactly
    // one thing to answer at the end of it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(password_only_vault(dir.path()).await);
    app.vault_state = VaultState::Locked;

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);

    assert!(app.show_vault_password_prompt(), "nothing was asked for");
    assert!(!app.vault_awaiting_biometric(), "both were asked for");
    assert!(
        !app.show_upgrade_modal(),
        "the upgrade modal opened over a locked vault"
    );

    let frame = drawn(&mut app);
    assert!(
        !frame.contains("Touch ID"),
        "a touch prompt was drawn on a machine that cannot use it:\n{frame}"
    );
}

// --------------------------------------------------------- the touch that fails

#[tokio::test]
async fn a_refused_touch_falls_back_to_the_master_password() {
    // The other half of "one prompt": a touch that is cancelled or unavailable
    // must land somewhere the user can act, not in a state with no modal.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    app.vault_password_input_mut().push_str("stale");

    assert!(app.apply(Msg::VaultBiometricFailed {
        epoch: app.vault_epoch
    }));
    assert!(app.show_vault_password_prompt(), "a dead end was reached");
    assert!(!app.vault_awaiting_biometric());
    assert_eq!(
        app.vault_password_input(),
        "",
        "keystrokes from the touch prompt were left in the password field"
    );
}

#[tokio::test]
async fn a_touch_failure_from_a_retired_attempt_is_ignored() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let stale = app.vault_epoch.wrapping_sub(1);

    assert!(!app.apply(Msg::VaultBiometricFailed { epoch: stale }));
    assert!(
        app.vault_awaiting_biometric(),
        "a stale failure knocked down a live touch prompt"
    );
}

#[tokio::test]
async fn a_touch_prompt_that_never_answers_can_still_be_escaped() {
    // If the enclave call dies or hangs there is no message coming. Without
    // this the app could only be killed.
    let _g = isolate().await;
    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);

    for escape in [KeyCode::Esc, KeyCode::Char('q')] {
        let mut app = App::new(vec![test_server("alpha")]);
        app.vault_state = VaultState::Unlocking {
            awaiting_biometric: true,
        };
        press(&mut app, escape, &tx, &mut tasks);
        assert!(
            !app.vault_awaiting_biometric(),
            "{escape:?} did not get the user out of the touch prompt"
        );
        assert!(!app.should_quit(), "{escape:?} quit the app instead");
    }
}

#[tokio::test]
async fn keys_that_are_not_the_way_out_are_swallowed_by_the_touch_prompt() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);

    for code in [KeyCode::Char('d'), KeyCode::Char('g'), KeyCode::Enter] {
        press(&mut app, code, &tx, &mut tasks);
    }
    assert!(
        app.vault_awaiting_biometric(),
        "a stray key left the prompt"
    );
    assert!(rx.try_recv().is_err(), "a stray key started work behind it");
}

#[tokio::test]
async fn the_touch_prompt_says_what_it_is_waiting_for() {
    // Rule: a waiting state says why. A blank modal over a locked vault is
    // indistinguishable from a hung app.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };

    let frame = drawn(&mut app);
    assert!(
        frame.contains("Vault Locked"),
        "the touch prompt drew nothing that names it:\n{frame}"
    );
}

// -------------------------------------------------------------- the spawned task

#[tokio::test]
async fn a_touch_attempt_against_a_vault_that_has_no_wrapper_reports_failure() {
    // The path every machine without a bound enclave key takes. It has to end
    // in `VaultBiometricFailed`, because that is what puts the master password
    // prompt up -- anything else strands the user in front of a modal with no
    // way forward.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault = password_only_vault(dir.path()).await;
    let (tx, mut rx) = mpsc::channel::<Msg>(4);

    spawn_biometric_unlock(vault, 7, tx)
        .await
        .expect("the attempt must finish");

    let msg = rx.try_recv().expect("it must report back");
    let Msg::VaultBiometricFailed { epoch } = msg else {
        panic!("expected a biometric failure, got {msg:?}");
    };
    assert_eq!(epoch, 7, "the wrong attempt was stamped");
}

#[tokio::test]
async fn a_touch_attempt_against_a_missing_vault_file_still_reports_back() {
    // Silence here is the worst answer: the modal is up and only a message
    // takes it down.
    let _g = isolate().await;
    let vault = Arc::new(Vault::new(VaultConfig {
        vault_path: std::path::PathBuf::from("/no/such/vault.bin"),
        argon2_params: None,
        use_os_keychain: false,
    }));
    let (tx, mut rx) = mpsc::channel::<Msg>(4);

    spawn_biometric_unlock(vault, 1, tx)
        .await
        .expect("the attempt must finish");
    assert!(
        matches!(rx.try_recv(), Ok(Msg::VaultBiometricFailed { epoch: 1 })),
        "a missing vault left the touch prompt with nothing to take it down"
    );
}

#[tokio::test]
async fn a_touch_failure_hands_the_user_to_the_password_prompt_end_to_end() {
    // The two halves joined: the task reports, `apply` acts on it, and what the
    // user is left looking at is the master password prompt.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.vault = Some(password_only_vault(dir.path()).await);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let epoch = app.vault_epoch;

    let (tx, mut rx) = mpsc::channel::<Msg>(4);
    spawn_biometric_unlock(app.vault.clone().unwrap(), epoch, tx)
        .await
        .expect("the attempt must finish");
    let msg = rx.try_recv().expect("it must report back");
    assert!(app.apply(msg));

    let frame = drawn(&mut app);
    assert!(
        frame.contains("Master Password") || frame.contains("Vault"),
        "the user was left with no prompt after the touch failed:\n{frame}"
    );
    assert!(app.show_vault_password_prompt());
}
