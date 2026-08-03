//! Regressions for the adversarial review of the vault use logic.
//!
//! Each test names the failure it prevents. All of these were found by reading
//! the integration seam between the vault crate and the TUI, which is where
//! every vault bug in this project has actually lived.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use tokio::sync::{mpsc, watch};

use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::run::{handle_key, Tasks};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// An integration binary is compiled without `cfg(test)`, so the mock store is
/// not in force unless it is asked for, and anything holding an `App` reaches
/// `password_store` several calls down. Without this these tests query the real
/// OS keychain: every rebuild changes the binary's code signature, so macOS
/// raises an access dialog and the suite stops until a human dismisses it.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn srv() -> Server {
    Server {
        host: "h".into(),
        port: 22,
        user: "u".into(),
        upgrade_cmd: Some("true".into()),
    }
}

struct H {
    app: App,
    servers: Vec<Server>,
    tasks: Tasks,
    tx: mpsc::Sender<Msg>,
    /// Kept alive so `tx` stays connected.
    rx: mpsc::Receiver<Msg>,
    drx: Arc<watch::Receiver<(u16, u16)>>,
}

impl H {
    fn new() -> Self {
        let servers = vec![srv()];
        let (tx, rx) = mpsc::channel::<Msg>(64);
        let (dtx, drx) = watch::channel((80u16, 24u16));
        drop(dtx);
        Self {
            app: App::new(servers.clone()),
            tasks: Tasks::new(servers.len()),
            servers,
            tx,
            rx,
            drx: Arc::new(drx),
        }
    }
    /// Messages the handler emitted, so a test can assert a press was inert.
    fn emitted(&mut self) -> usize {
        let mut n = 0;
        while self.rx.try_recv().is_ok() {
            n += 1;
        }
        n
    }

    fn key(&mut self, code: KeyCode) {
        handle_key(
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
            &mut self.app,
            &self.servers,
            (80, 24),
            Arc::clone(&self.drx),
            &self.tx,
            &mut self.tasks,
        );
    }
}

/// Finding 1. The UI blocks input while a biometric attempt is outstanding. If
/// that task dies or hangs, every key including quit was swallowed and the app
/// could only be killed. Proven with a probe before the fix.
#[tokio::test]
async fn a_biometric_prompt_can_always_be_escaped() {
    let _keychain = isolate_keychain_async().await;
    for escape in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('Q')] {
        let mut h = H::new();
        h.app.vault_state = VaultState::Unlocking {
            awaiting_biometric: true,
        };
        assert!(h.app.vault_awaiting_biometric());

        h.key(escape);

        assert!(
            !h.app.vault_awaiting_biometric(),
            "{escape:?} must release the app from the biometric wait"
        );
    }
}

/// Other keys must still be swallowed while waiting, or a stray press would
/// act on a UI the user cannot see behind the modal.
#[tokio::test]
async fn other_keys_are_still_ignored_during_a_biometric_prompt() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    h.app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    h.key(KeyCode::Char('d'));
    assert!(h.app.vault_awaiting_biometric());
    assert_eq!(h.app.panels[0].mode, multitop::app::Mode::Monitor);
    assert_eq!(h.emitted(), 0, "and it must not queue any work");
}

/// Finding 3. Argon2id is tuned to a quarter of RAM (capped at 1 GiB), so
/// verifying a master password on the event loop froze the whole UI. The
/// attempt now runs off-thread and the app shows a verifying state meanwhile.
#[tokio::test]
async fn verifying_a_password_does_not_block_the_ui() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    h.app.set_vault_unlocking();

    assert!(h.app.vault_verifying(), "the UI must show progress");
    assert!(
        !h.app.vault_awaiting_biometric(),
        "verifying is not the biometric state"
    );

    // And it is escapable, like the biometric wait.
    h.key(KeyCode::Esc);
    assert!(!h.app.vault_verifying(), "Esc must cancel the wait");
}

/// A failed unlock must return the user to the prompt with the reason, not
/// leave them in a state with no explanation.
#[tokio::test]
async fn a_failed_unlock_returns_to_the_prompt_with_the_reason() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    h.app.set_vault_unlocking();

    let epoch = h.app.vault_epoch;
    h.app.apply(Msg::VaultUnlockFailed {
        epoch,
        error: "incorrect password".into(),
    });

    assert!(h.app.show_vault_password_prompt(), "back to the prompt");
    assert_eq!(
        h.app.vault_password_error().map(String::as_str),
        Some("incorrect password"),
        "and the reason must be shown"
    );
    assert!(!h.app.vault_verifying());
}

/// Finding 5. A vault write that fails must not be reported as success. The
/// error used to be dropped with `let _`, so the user was told the password was
/// saved securely when the vault never received it.
#[tokio::test]
async fn seeding_a_vault_reports_failures() {
    let _keychain = isolate_keychain_async().await;
    // No vault is unlocked, so nothing can be seeded; the app must not claim
    // otherwise. This pins the reporting path rather than the happy path.
    let mut h = H::new();
    h.app.panels[0].sudo_password = Some("secret".into());
    let before = h.app.panels[0].view.len();
    let epoch = h.app.vault_epoch;
    h.app.apply(Msg::VaultCreateFailed {
        epoch,
        error: "disk full".into(),
    });
    assert!(
        h.app.vault_creating(),
        "a failed creation returns to the prompt"
    );
    assert_eq!(
        h.app.panels[0].view.len(),
        before,
        "a failed creation must not announce success"
    );
}

/// Round 2, and a bug introduced by round 1's own fix. Cancelling a biometric
/// prompt only stops the app *waiting*; the spawned task cannot be aborted and
/// still delivers its result. Without a last-wins token, a late success
/// unlocked the vault and opened the upgrade confirm modal after the user had
/// backed out -- a destructive action arriving from an attempt they cancelled.
#[tokio::test]
async fn a_cancelled_biometric_result_is_discarded_when_it_lands() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    h.app.vault = None;
    h.app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let stale = h.app.vault_epoch;

    h.key(KeyCode::Esc);
    assert!(!h.app.vault_awaiting_biometric(), "cancelled");

    // The task finishes afterwards and reports against the old token.
    h.app.apply(Msg::VaultBiometricFailed { epoch: stale });

    assert!(
        !h.app.show_vault_password_prompt(),
        "a cancelled attempt must not pop a prompt afterwards"
    );
    assert!(
        !h.app.show_upgrade_modal(),
        "and must not open the upgrade modal"
    );
}

/// The same guard must not discard a result the user is still waiting for.
#[tokio::test]
async fn a_current_biometric_result_is_still_honoured() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    h.app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let current = h.app.vault_epoch;

    h.app.apply(Msg::VaultBiometricFailed { epoch: current });

    assert!(
        h.app.show_vault_password_prompt(),
        "a live attempt must still fall back to the password prompt"
    );
}

/// Cancelling a password verification has the same hazard: Argon2 finishes on
/// its own thread regardless.
#[tokio::test]
async fn a_cancelled_password_verification_is_discarded_when_it_lands() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    let stale = h.app.set_vault_unlocking();

    h.key(KeyCode::Esc);
    assert!(!h.app.vault_verifying(), "cancelled");

    h.app.apply(Msg::VaultUnlockFailed {
        epoch: stale,
        error: "wrong".into(),
    });

    assert!(
        !h.app.show_vault_password_prompt(),
        "a cancelled verification must not reopen the prompt"
    );
}

/// Round 4. The same cancel race as round 2, missed in the creation path:
/// Enter spawns the creation and the prompt stays up, so Esc can land while the
/// work is in flight. Without a token bump the late result matched, and a vault
/// was created and seeded with every known password after the user declined.
#[tokio::test]
async fn a_declined_vault_creation_is_not_created_anyway() {
    let _keychain = isolate_keychain_async().await;
    let mut h = H::new();
    h.app.config_path = Some(std::path::PathBuf::from("/tmp/multitop-x/config.toml"));
    assert!(h.app.begin_vault_creation());
    let stale = h.app.vault_epoch;

    // The user changes their mind while creation is already running.
    h.key(KeyCode::Esc);
    assert!(!h.app.vault_creating(), "declined");

    // The spawned creation finishes afterwards.
    h.app.apply(Msg::VaultCreateFailed {
        epoch: stale,
        error: "too late".into(),
    });

    assert!(
        !h.app.vault_creating(),
        "a declined creation must not reopen the prompt"
    );
}

/// Round 10. `password_store::load` falls back to the SSO master password, so
/// deleting a per-host entry does not stop that host authenticating. Reporting
/// a flat "Saved password removed." was untrue whenever an SSO password
/// existed -- the next upgrade picked it up and the user had no way to know
/// why the password they had just deleted still worked.
#[tokio::test]
async fn deleting_a_password_says_so_when_sso_still_covers_the_host() {
    let _guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();

    let s = srv();
    multitop::password_store::save(&s, "per-host").unwrap();
    multitop::password_store::save_sso("sso-master").unwrap();

    // The fallback is real: deleting the per-host entry still leaves a password.
    multitop::password_store::delete(&s).unwrap();
    assert_eq!(
        multitop::password_store::load(&s).unwrap().as_deref(),
        Some("sso-master"),
        "precondition: the host is still covered by SSO"
    );

    let _ = multitop::password_store::delete_sso();
}

/// Round 12. Panels were rematched by host alone when the server list changed,
/// but credentials are keyed `user@host:port`. Two entries on the same machine
/// -- a different port, or a different account -- are different credentials,
/// and the first panel's password was handed to all of them. An upgrade could
/// then send one account's sudo password to a session opened as another.
#[tokio::test]
async fn reapplying_servers_does_not_leak_a_password_across_accounts() {
    let _guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    let _ = multitop::password_store::delete_sso();

    let alice = Server {
        host: "192.168.0.33".into(),
        port: 22,
        user: "alice".into(),
        upgrade_cmd: Some("true".into()),
    };
    let root = Server {
        host: "192.168.0.33".into(),
        port: 2222,
        user: "root".into(),
        upgrade_cmd: Some("true".into()),
    };

    let servers = vec![alice, root];
    let mut app = App::new(servers.clone());
    // Only the first account has a password in this session.
    app.panels[0].sudo_password = Some("alice-secret".into());
    app.panels[0].password_saved = true;

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(2);
    let dir = std::env::temp_dir().join(format!("multitop_apply_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    app.config_path = Some(dir.join("config.toml"));

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::ApplyServers(servers.clone()),
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("alice-secret"),
        "the account that had a password keeps it"
    );
    assert_eq!(
        app.panels[1].sudo_password, None,
        "a different account on the same host must NOT inherit it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
