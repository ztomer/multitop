//! The master password can be changed from the Configuration panel.
//!
//! `Vault::change_password` was implemented and tested with no UI path to it, so
//! the password protecting every stored credential could not be retired without
//! deleting the vault and re-entering everything.
//!
//! These cover the keystroke state machine only. Whether the vault actually
//! re-wraps is covered in the vault crate; what is easy to get wrong here is the
//! sequencing -- collecting two passwords in a row, carrying the first to the
//! second, and not leaving a half-finished prompt behind.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;
use multitop::app::App;
use multitop::config::Server;
use multitop::passwords::{handle_key, PasswordAction, PasswordManager};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// Driving an `App` reaches `password_store` several calls down, and an
/// integration binary is compiled without `cfg(test)`, so the mock is not in
/// force unless it is asked for. Without this these tests query the real OS
/// keychain: every rebuild changes the binary's code signature, so macOS raises
/// an access dialog and the suite stops until a human dismisses it -- and a test
/// can read, overwrite or delete credentials the user depends on.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// `isolate_keychain` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        upgrade_cmd: None,
    }
}

/// The parts of the fixture that do not need a runtime: a temp directory, an
/// app pointed at it, and an uninitialised vault beside its config.
fn app_and_vault(tag: &str) -> (App, multitop_vault::Vault, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mt_rot_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut app = App::new(vec![server("host-a")]);
    app.config_path = Some(dir.join("config.toml"));
    let vault = multitop_vault::Vault::new(multitop_vault::VaultConfig {
        vault_path: dir.join("vault.bin"),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        }),
        use_os_keychain: false,
    });
    (app, vault, dir)
}

fn finish(mut app: App, vault: multitop_vault::Vault) -> App {
    app.vault = Some(std::sync::Arc::new(vault));
    app.password_manager = Some(PasswordManager::new(0, false));
    app
}

/// An app with a real vault file present, so rotation is offered.
fn app_with_vault(tag: &str) -> (App, std::path::PathBuf) {
    let (app, vault, dir) = app_and_vault(tag);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize("old-master"))
        .unwrap();
    (finish(app, vault), dir)
}

/// The same fixture from inside a `#[tokio::test]`.
///
/// `block_on` cannot be called from a thread already driving a runtime, so the
/// synchronous version panics there rather than building the vault.
async fn app_with_vault_async(tag: &str) -> (App, std::path::PathBuf) {
    let (app, vault, dir) = app_and_vault(tag);
    vault.initialize("old-master").await.unwrap();
    (finish(app, vault), dir)
}

fn type_in(app: &mut App, text: &str) -> PasswordAction {
    let mut last = PasswordAction::None;
    for c in text.chars() {
        last = handle_key(app, KeyCode::Char(c));
    }
    last
}

#[test]
fn r_collects_the_current_password_then_the_new_one() {
    let _keychain = isolate_keychain();
    let (mut app, dir) = app_with_vault("flow");

    assert!(matches!(
        handle_key(&mut app, KeyCode::Char('r')),
        PasswordAction::None
    ));
    let m = app.password_manager.as_ref().unwrap();
    assert!(m.editing(), "a prompt must be open");
    assert!(
        m.notice.as_deref().unwrap_or_default().contains("master"),
        "and it must be the rotation prompt"
    );
    assert!(
        m.notice.as_deref().unwrap_or_default().contains("CURRENT"),
        "the first prompt must ask for the current password, got {:?}",
        m.notice
    );

    type_in(&mut app, "old-master");
    let action = handle_key(&mut app, KeyCode::Enter);
    assert!(
        matches!(action, PasswordAction::None),
        "the first Enter must not act yet -- it only advances to the second prompt"
    );
    let m = app.password_manager.as_ref().unwrap();
    assert!(m.editing(), "still prompting after the first password");
    assert!(
        m.notice.as_deref().unwrap_or_default().contains("NEW"),
        "the second prompt must ask for the new password, got {:?}",
        m.notice
    );

    type_in(&mut app, "new-master");
    match handle_key(&mut app, KeyCode::Enter) {
        PasswordAction::RotateVaultPassword { current, new } => {
            assert_eq!(current, "old-master", "the first answer must be carried");
            assert_eq!(new, "new-master");
        }
        other => panic!("expected a rotation action, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn escape_abandons_the_rotation_without_acting() {
    let _keychain = isolate_keychain();
    let (mut app, dir) = app_with_vault("esc");

    handle_key(&mut app, KeyCode::Char('r'));
    type_in(&mut app, "old-master");
    handle_key(&mut app, KeyCode::Enter);
    // Now at the second prompt, holding the current password.
    let action = handle_key(&mut app, KeyCode::Esc);

    assert!(matches!(action, PasswordAction::None));
    let m = app.password_manager.as_ref().unwrap();
    assert!(!m.editing(), "the prompt must be closed");
    assert!(!m.editing(), "and the carried password released");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_password_does_not_rotate() {
    let _keychain = isolate_keychain();
    let (mut app, dir) = app_with_vault("empty");

    handle_key(&mut app, KeyCode::Char('r'));
    let action = handle_key(&mut app, KeyCode::Enter);

    assert!(
        matches!(action, PasswordAction::None),
        "an empty current password must not start a rotation"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rotation_is_not_offered_without_a_vault() {
    let _keychain = isolate_keychain();
    let mut app = App::new(vec![server("host-a")]);
    app.password_manager = Some(PasswordManager::new(0, false));
    assert!(app.vault.is_none());

    let action = handle_key(&mut app, KeyCode::Char('r'));

    assert!(matches!(action, PasswordAction::None));
    let m = app.password_manager.as_ref().unwrap();
    assert!(
        !m.editing(),
        "no vault means no prompt -- it would collect a password with nowhere to put it"
    );
    assert!(
        m.notice.as_deref().unwrap_or_default().contains("No vault"),
        "the user must be told why nothing happened, got {:?}",
        m.notice
    );
}

/// `r` must not be swallowed while another prompt is open.
#[test]
fn r_is_ordinary_text_while_a_password_is_being_typed() {
    let _keychain = isolate_keychain();
    let (mut app, dir) = app_with_vault("text");

    // Open the row editor and move to its password field.
    handle_key(&mut app, KeyCode::Enter);
    for _ in 0..4 {
        handle_key(&mut app, KeyCode::Tab);
    }
    type_in(&mut app, "supersecret");
    let m = app.password_manager.as_ref().unwrap();
    assert!(!m.editing(), "typing r must not start a rotation");
    assert_eq!(
        m.draft.as_ref().unwrap().password,
        "supersecret",
        "the r in 'supersecret' must land in the field"
    );

    let _ = std::fs::remove_dir_all(dir);
}

/// A second `r` cannot start a second rotation over the first.
///
/// Found by the audit item 3 asks for: every keypress that spawns work, asked
/// "what does the second press do?". The rotation prompt closes the instant
/// Enter is pressed, because the work happens off-thread, so the panel went
/// straight back to accepting `r` with only a one-line notice to say why it
/// should not be pressed. `change_password` reads the vault, rewraps the key
/// and writes it back; two of those overlapping both unlock with the *old*
/// password and both write, so the last one silently wins while both report
/// success -- and a mistyped current password spends two of the kill-resistant
/// limiter's tries instead of one.
#[tokio::test]
async fn a_second_rotation_cannot_start_while_one_is_running() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, dir) = app_with_vault_async("no-double-rotation").await;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<multitop::app::Msg>(16);
    let servers = vec![server("host-a")];
    let mut tasks = multitop::run::Tasks::new(1);

    handle_key(&mut app, KeyCode::Char('r'));
    type_in(&mut app, "old-master");
    handle_key(&mut app, KeyCode::Enter);
    type_in(&mut app, "new-master");
    let action = handle_key(&mut app, KeyCode::Enter);
    multitop::password_actions::apply(action, &mut app, &servers, &tx, &mut tasks);

    assert!(
        app.password_manager.as_ref().unwrap().rotating,
        "the panel must know a rotation is running"
    );

    // What a user does when nothing appears to have happened.
    let again = handle_key(&mut app, KeyCode::Char('r'));
    assert_eq!(again, PasswordAction::None, "a second r must not act");
    let manager = app.password_manager.as_ref().unwrap();
    assert!(
        !manager.editing(),
        "and must not reopen the prompt, which is what would collect a second rotation"
    );
    assert!(
        manager
            .notice
            .as_deref()
            .unwrap_or_default()
            .contains("already being changed"),
        "it must say why, got {:?}",
        manager.notice
    );

    // Exactly one rotation reports back, and it clears the flag.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
        .await
        .expect("the rotation must report back")
        .expect("channel open");
    app.apply(msg);
    let manager = app.password_manager.as_ref().unwrap();
    assert!(!manager.rotating, "the flag must clear when it finishes");
    assert!(
        manager
            .notice
            .as_deref()
            .unwrap_or_default()
            .contains("Master password changed"),
        "got {:?}",
        manager.notice
    );

    // And `r` works again afterwards.
    handle_key(&mut app, KeyCode::Char('r'));
    assert!(app.password_manager.as_ref().unwrap().editing());

    let _ = std::fs::remove_dir_all(dir);
}

/// Closing the panel mid-rotation must not swallow the outcome.
///
/// Whether the password that unlocks every stored credential actually changed
/// is not something to learn only if you happened to still be on the right
/// screen when the answer arrived.
#[tokio::test]
async fn the_rotation_outcome_survives_the_panel_being_closed() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, dir) = app_with_vault_async("outcome-survives").await;

    app.password_manager = None;
    app.apply(multitop::app::Msg::VaultPasswordRotated {
        epoch: app.vault_epoch,
    });

    assert!(
        multitop::ui::pane_lines(&app, 0, 20, 60, 0)
            .0
            .iter()
            .any(|l| l.contains("changed")),
        "the outcome must reach the panels, got {:?}",
        multitop::ui::pane_lines(&app, 0, 20, 60, 0).0
    );

    let _ = std::fs::remove_dir_all(dir);
}
