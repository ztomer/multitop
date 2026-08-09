//! Two things that only happen when something is already going wrong: a pane
//! too narrow to draw a banner into, and a vault that cannot be written.
//!
//! Both are silent failures if they are got wrong — a banner that vanishes, a
//! vault that reports success and holds nothing — so both are pinned here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use ratatui::backend::TestBackend;
use secrecy::SecretString;
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

fn drawn(app: &mut App, size: (u16, u16)) -> String {
    let backend = TestBackend::new(size.0, size.1);
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

// ------------------------------------------------------------- narrow panes

#[tokio::test]
async fn a_pane_too_narrow_for_rules_draws_the_name_and_nothing_else() {
    // Below the width where a rule fits either side, the rules go and the name
    // stays. Dropping the name instead would leave a pane nobody can identify.
    let _g = isolate().await;

    for width in 5u16..=10 {
        let mut app = App::new(vec![test_server("web-01")]);
        app.panels[0].show_frame(vec!["body".to_string(); 3]);
        let text = drawn(&mut app, (width, 10));
        let row0: String = text.chars().take(width as usize).collect();

        assert!(
            !row0.contains('\u{2500}'),
            "width {width}: a rule was drawn with no room for one: {row0:?}"
        );
        assert!(
            row0.trim().chars().any(|c| c.is_ascii_alphanumeric()),
            "width {width}: nothing identifiable was drawn: {row0:?}"
        );
    }
}

#[tokio::test]
async fn the_banner_falls_back_to_the_configured_host_for_a_non_monitor_payload() {
    // Only a Monitor packet carries the host's own name. A Docker payload in
    // that slot must fall back to what was configured rather than drawing an
    // empty banner.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("configured-host")]);
    app.panels[0].last_monitor = Some(multitop_agent::proto::Payload::Docker {
        host: "ignored".into(),
        rows: vec![],
    });
    app.panels[0].show_frame(vec!["body".to_string(); 3]);

    let text = drawn(&mut app, (120, 20));
    assert!(
        text.contains("configured-host"),
        "the banner fell back to nothing:\n{text}"
    );
}

// ------------------------------------------------------- a vault that cannot

#[tokio::test]
async fn a_vault_that_cannot_be_created_says_why_and_leaves_the_prompt_up() {
    // The directory the vault would live in is a file. Creating it fails, and
    // the user has to be told rather than left watching a prompt that never
    // resolves.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("cfg");
    std::fs::write(&blocker, b"a file where a directory should be").unwrap();

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(blocker.join("config.toml"));
    assert!(app.begin_vault_creation());

    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    for c in MASTER.chars() {
        press(&mut app, KeyCode::Char(c), &tx, &mut tasks);
    }
    press(&mut app, KeyCode::Enter, &tx, &mut tasks);

    let msg = tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv())
        .await
        .expect("the create must report back")
        .expect("a message must arrive");
    let Msg::VaultCreateFailed { ref error, .. } = msg else {
        panic!("expected a failure, got {msg:?}");
    };
    assert!(!error.is_empty(), "the failure carried no reason");

    assert!(app.apply(msg));
    assert!(
        app.vault_creating(),
        "the prompt was dismissed on a failure"
    );
    assert!(app.vault_create_error().is_some(), "no reason was shown");
    assert!(
        !app.vault_create_in_flight(),
        "it still claims to be running"
    );
}

#[tokio::test]
async fn an_unlock_with_no_vault_behind_it_starts_nothing() {
    // The prompt can be up with `app.vault` unset — the vault file was there
    // when the prompt opened and is gone now. Typing a password must not panic
    // and must not spawn an unwrap against nothing.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    app.set_show_vault_password_prompt(true);
    assert!(app.vault.is_none());

    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    for c in "hunter2".chars() {
        press(&mut app, KeyCode::Char(c), &tx, &mut tasks);
    }
    press(&mut app, KeyCode::Enter, &tx, &mut tasks);

    assert!(
        rx.try_recv().is_err(),
        "an unlock was spawned with no vault"
    );
    assert!(!app.vault_verifying(), "it claims to be verifying nothing");
}

#[tokio::test]
async fn a_new_vault_that_cannot_hold_the_session_passwords_says_so() {
    // Seeding writes each panel's password into the fresh vault. If those
    // writes fail the user must be told, or they believe the vault holds
    // passwords it never received.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path.clone()));
    vault.initialize(MASTER).await.expect("initialise");
    let unlocked = vault.unlock_with_password(MASTER).expect("unlock");

    let mut app = App::new(vec![test_server("alpha"), test_server("beta")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.panels[0].sudo_password = Some("alpha-secret".into());
    app.panels[1].sudo_password = Some("beta-secret".into());

    // Take the vault's directory away, so every save the seeding attempts
    // fails. The handle is already open; only the file it writes to is gone.
    std::fs::remove_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path(), b"now a file, not a directory").unwrap();

    app.apply(Msg::VaultCreated {
        epoch: app.vault_epoch,
        unlocked: Box::new(unlocked),
    });

    let notes = app.panels[0].notes.join("\n");
    assert!(
        notes.contains("could not be written"),
        "the failed seeding was silent:\n{notes}"
    );
    assert!(
        notes.contains("credential store"),
        "the note must say where the passwords still are:\n{notes}"
    );
}

// ------------------------------------------------------------------ filter

#[tokio::test]
async fn keys_the_filter_has_no_use_for_leave_the_query_alone() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha"), test_server("beta")]);
    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(2);

    press(&mut app, KeyCode::Char('/'), &tx, &mut tasks);
    for c in "alp".chars() {
        press(&mut app, KeyCode::Char(c), &tx, &mut tasks);
    }
    assert_eq!(app.filter_query, "alp");

    // Navigation and function keys are not query text and must not be eaten by
    // the single-letter bindings underneath either.
    for code in [KeyCode::Up, KeyCode::F(2), KeyCode::Insert, KeyCode::Tab] {
        press(&mut app, code, &tx, &mut tasks);
    }
    assert_eq!(app.filter_query, "alp", "a stray key changed the query");
    assert!(app.is_filtering(), "a stray key closed the filter");
}

// ------------------------------------------------------------- the notices

#[tokio::test]
async fn a_leading_notice_is_kept_whether_or_not_a_second_one_follows() {
    // Two halves of one action each report; whichever writes last must not
    // erase the other, and an identical pair must not be printed twice.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path));
    vault.initialize(MASTER).await.expect("initialise");

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(vault));
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
    multitop::passwords::open(&mut app, 0, false);

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);

    // A save reports once. An import over the same screen reports again, and
    // the first report has to survive into the combined notice.
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Save {
            panel: 0,
            password: "hunter2".into(),
            resume_upgrade: false,
        },
        &mut app,
        &tx,
        &mut tasks,
    );
    let after_save = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(!after_save.is_empty(), "the save said nothing");

    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::ImportSshHosts,
        &mut app,
        &tx,
        &mut tasks,
    );
    let after_import = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(!after_import.is_empty(), "the import said nothing");
    assert_ne!(
        after_import, after_save,
        "the import reported the save's notice"
    );
}

// -------------------------------------------------------------- vault reads

#[tokio::test]
async fn a_password_the_vault_holds_is_marked_as_coming_from_outside() {
    // The Upgrade view reads `external_password` to say where a credential came
    // from. A vault password reported as a session one tells the user it will
    // be gone next launch.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    let vault = multitop_vault::Vault::new(multitop::vault::config_for(vault_path));
    vault.initialize(MASTER).await.expect("initialise");

    let server = test_server("alpha");
    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    unlocked
        .set_password(
            password_store::account(&server),
            &SecretString::from("from-the-vault".to_string()),
        )
        .expect("store");
    unlocked.save().expect("save");

    let mut panel = multitop::panel::Panel::new(server);
    multitop::vault::try_load_vault_password(&mut panel, &unlocked);
    assert_eq!(panel.sudo_password.as_deref(), Some("from-the-vault"));
    assert!(
        panel.external_password,
        "a vault password was reported as a session one"
    );
}
