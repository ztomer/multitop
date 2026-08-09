//! One credential prompt per upgrade, and one only.
//!
//! The rule: pressing `u` asks the user for *one* thing. A fingerprint or a
//! master password, whichever this machine can do -- never both, and never a
//! second ask further down the run. The biometric prompt is a *replacement* for
//! the password prompt, not a step before it.
//!
//! This has regressed twice, in the same shape both times: something started a
//! second credential flow that the first one was supposed to have satisfied.
//! Both were reported by a user rather than caught here, because every test
//! looked at one prompt at a time and none counted them across the journey. So
//! this file counts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
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

/// Everything that constitutes asking the user for a credential.
///
/// Named individually rather than as one boolean so a failure says *which*
/// prompt appeared, and so a new one added later has to be added here for the
/// count to stay honest.
fn prompts_showing(app: &App) -> Vec<&'static str> {
    let mut showing = Vec::new();
    if app.vault_awaiting_biometric() {
        showing.push("biometric");
    }
    if app.show_vault_password_prompt() {
        showing.push("master password");
    }
    if app.vault_creating() {
        showing.push("create vault");
    }
    // The manager's own predicate, not a second copy of what "asking" means.
    // Today `edit` is only ever a master-password rotation; if a sudo entry is
    // ever added it goes through the same field and is counted here for free.
    if app
        .password_manager
        .as_ref()
        .is_some_and(multitop::passwords::PasswordManager::editing)
    {
        showing.push("password manager entry");
    }
    showing
}

/// Walk the journey, sampling after every step, and count how many *distinct*
/// times the user was put in front of something asking for a credential.
struct PromptCounter {
    /// Which prompt was on screen at the previous sample, if any.
    ///
    /// The identity, not a boolean. A boolean counts "a prompt appeared" and
    /// therefore misses the case that matters most here: one prompt replacing
    /// another with no gap between them, which is two things asked of the user
    /// and reads as `true` then `true`. That is exactly the shape of the defect
    /// this file exists for, and the first version of this counter could not
    /// see it -- proven by injecting a second prompt and watching it pass.
    last: Option<&'static str>,
    seen: Vec<String>,
}

impl PromptCounter {
    const fn new() -> Self {
        Self {
            last: None,
            seen: Vec::new(),
        }
    }

    fn sample(&mut self, app: &App, step: &str) {
        let showing = prompts_showing(app);
        assert!(
            showing.len() <= 1,
            "at '{step}' the user was shown {} at once: {showing:?}",
            showing.len()
        );
        let now = showing.first().copied();
        if let Some(name) = now {
            if self.last != Some(name) {
                self.seen.push(format!("{name} (at '{step}')"));
            }
        }
        self.last = now;
    }
}

/// A vault holding a password for every host, which is the configuration the
/// single-prompt promise is made about.
async fn vault_with_all_passwords(
    dir: &std::path::Path,
    servers: &[Server],
) -> Arc<multitop_vault::Vault> {
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(dir.join("vault.bin")));
    vault.initialize(MASTER).await.expect("initialise");
    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    for s in servers {
        unlocked
            .set_password(
                password_store::account(s),
                &secrecy::SecretString::from("hunter2".to_string()),
            )
            .expect("store");
    }
    unlocked.save().expect("save");
    Arc::new(vault)
}

#[tokio::test]
async fn one_upgrade_asks_for_one_credential_and_then_never_again() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![
        test_server("web-01"),
        test_server("web-02"),
        test_server("db-01"),
    ];
    let vault = vault_with_all_passwords(dir.path(), &servers).await;

    let mut app = App::new(servers.clone());
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(vault);
    app.vault_state = VaultState::Locked;

    let (tx, _rx) = mpsc::channel::<Msg>(64);
    let mut tasks = Tasks::new(3);
    let mut counter = PromptCounter::new();
    counter.sample(&app, "before anything");

    // First `u` enters the view and starts nothing.
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    counter.sample(&app, "after the first u");

    // Second `u` is the one that needs a credential.
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    counter.sample(&app, "after the second u");
    assert_eq!(
        counter.seen.len(),
        1,
        "the second `u` did not ask for exactly one credential: {:?}",
        counter.seen
    );

    // Answer it, the way the real unlock does.
    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password(MASTER)
        .expect("unlock");
    assert!(app.apply(Msg::VaultUnlocked {
        epoch: app.vault_epoch,
        unlocked: Box::new(unlocked),
    }));
    counter.sample(&app, "after answering");

    // From here the run must not ask again: the vault holds every password.
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    counter.sample(&app, "confirming the run");
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    counter.sample(&app, "after the run started");

    assert_eq!(
        counter.seen.len(),
        1,
        "the user was asked for a credential more than once: {:?}",
        counter.seen
    );
    assert!(
        app.panels.iter().all(|p| p.sudo_password.is_some()),
        "a host was left without the password the vault holds for it, so the \
         run would fail or ask again"
    );
}

#[tokio::test]
async fn the_two_prompts_are_never_on_screen_together() {
    // The defect that produced this rule: a biometric wait was set and then
    // overwritten by the password prompt on the next line, so the user was
    // asked twice for one unlock. `prompts_showing` asserts the exclusion at
    // every sample above; this pins it directly for the states themselves.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);

    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    assert_eq!(prompts_showing(&app), vec!["biometric"]);

    app.vault_state = VaultState::PasswordPrompt { error: None };
    assert_eq!(prompts_showing(&app), vec!["master password"]);

    // And a state that asks for nothing shows nothing.
    app.vault_state = VaultState::Locked;
    assert_eq!(prompts_showing(&app), Vec::<&str>::new());
}

#[tokio::test]
async fn a_vault_that_cannot_be_opened_by_touch_still_asks_only_once() {
    // The fallback is a *replacement*, not an extra step: a machine with no
    // biometric goes straight to the password and is asked once, not asked for
    // a finger it does not have and then a password.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![test_server("web-01")];
    let vault = vault_with_all_passwords(dir.path(), &servers).await;
    assert!(
        !vault.biometric_available(),
        "a test vault must not be touchable, or this proves nothing"
    );

    let mut app = App::new(servers);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(vault);
    app.vault_state = VaultState::Locked;

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    let mut counter = PromptCounter::new();

    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    counter.sample(&app, "after the first u");
    press(&mut app, KeyCode::Char('u'), &tx, &mut tasks);
    counter.sample(&app, "after the second u");

    assert_eq!(
        counter.seen,
        vec!["master password (at 'after the second u')"]
    );
}
