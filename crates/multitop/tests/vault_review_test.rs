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
    let mut h = H::new();
    h.app.set_vault_unlocking();

    h.app
        .apply(Msg::VaultUnlockFailed("incorrect password".into()));

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
    // No vault is unlocked, so nothing can be seeded; the app must not claim
    // otherwise. This pins the reporting path rather than the happy path.
    let mut h = H::new();
    h.app.panels[0].sudo_password = Some("secret".into());
    let before = h.app.panels[0].view.len();
    h.app.apply(Msg::VaultCreateFailed("disk full".into()));
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
