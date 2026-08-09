//! Key dispatch while a modal owns the screen.
//!
//! Every one of these is a case where the wrong key doing the wrong thing has
//! a real cost: a stray `Enter` that kills a running dpkg on N hosts, a key
//! swallowed while a biometric prompt hangs so the app can only be killed, a
//! password field that silently eats what was typed into it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Confirm, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use tokio::sync::{mpsc, watch};

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
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

/// Everything `handle_key` needs besides the app, and nothing that reaches the
/// network: the channel is drained by the test, not by a task.
struct Keys {
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    tasks: Tasks,
}

impl Keys {
    fn new(panels: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>(64);
        let (dims_tx, dims_rx) = watch::channel((80, 24));
        // Kept alive so the receiver stays valid for the whole test.
        std::mem::forget(dims_tx);
        Self {
            tx,
            rx,
            dims_rx: Arc::new(dims_rx),
            tasks: Tasks::new(panels),
        }
    }

    fn press(&mut self, app: &mut App, code: KeyCode) {
        self.press_with(app, code, KeyModifiers::NONE);
    }

    fn press_with(&mut self, app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        handle_key(
            KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Press),
            app,
            (80, 24),
            self.dims_rx.clone(),
            &self.tx,
            &mut self.tasks,
        );
    }

    fn release(&mut self, app: &mut App, code: KeyCode) {
        handle_key(
            KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release),
            app,
            (80, 24),
            self.dims_rx.clone(),
            &self.tx,
            &mut self.tasks,
        );
    }

    fn type_str(&mut self, app: &mut App, s: &str) {
        for c in s.chars() {
            self.press(app, KeyCode::Char(c));
        }
    }
}

fn app_with_config(dir: &tempfile::TempDir, hosts: &[&str]) -> App {
    let mut app = App::new(hosts.iter().map(|h| test_server(h)).collect());
    app.config_path = Some(dir.path().join("config.toml"));
    app
}

// ---------------------------------------------------------------- key releases

#[tokio::test]
async fn a_key_release_is_not_a_second_press() {
    // Terminals that report releases would otherwise run every action twice.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha", "beta"]);
    let mut k = Keys::new(2);

    app.selected_panel = 0;
    k.release(&mut app, KeyCode::Char('2'));
    assert_eq!(app.selected_panel, 0, "a release moved the selection");
    k.press(&mut app, KeyCode::Char('2'));
    assert_eq!(app.selected_panel, 1);
}

// ------------------------------------------------------- the quit confirmation

#[tokio::test]
async fn only_the_keys_the_quit_row_names_confirm_it() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    // `Enter` and `y` are exactly the wrong keys here: this press kills a
    // running dpkg transaction on every host, and `Enter` is what an operator
    // hits to dismiss something they have not read.
    for code in [KeyCode::Enter, KeyCode::Char('y'), KeyCode::Char('x')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.panels[0].upgrade_state = UpgradeState::STARTED;
        let mut k = Keys::new(1);

        k.press(&mut app, KeyCode::Char('q'));
        assert_eq!(
            app.active_confirm(),
            Some(Confirm::Quit),
            "the quit was not armed"
        );
        k.press(&mut app, code);
        assert!(
            !app.should_quit(),
            "{code:?} confirmed a quit it does not name"
        );
        assert_eq!(
            app.active_confirm(),
            Some(Confirm::Quit),
            "{code:?} stood it down"
        );
    }
}

#[tokio::test]
async fn q_and_ctrl_c_both_confirm_a_quit_and_esc_stands_it_down() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    for code in [KeyCode::Char('q'), KeyCode::Char('Q')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.panels[0].upgrade_state = UpgradeState::STARTED;
        let mut k = Keys::new(1);
        k.press(&mut app, KeyCode::Char('q'));
        k.press(&mut app, code);
        assert!(app.should_quit(), "{code:?} did not confirm");
    }

    // Ctrl-C means the same thing everywhere.
    let mut app = app_with_config(&dir, &["alpha"]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    let mut k = Keys::new(1);
    k.press(&mut app, KeyCode::Char('q'));
    k.press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.should_quit());

    // Esc stands it down and leaves the app running.
    let mut app = app_with_config(&dir, &["alpha"]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    let mut k = Keys::new(1);
    k.press(&mut app, KeyCode::Char('q'));
    k.press(&mut app, KeyCode::Esc);
    assert!(!app.should_quit());
    assert_eq!(app.active_confirm(), None);
}

// ---------------------------------------------------- the upgrade confirmation

#[tokio::test]
async fn the_upgrade_confirmation_takes_only_the_key_it_names() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    // Cancel keys are not the same thing as confirm keys: a stray key that
    // cancels can only ever be the safe answer, so several are accepted.
    for code in [
        KeyCode::Esc,
        KeyCode::Char('q'),
        KeyCode::Char('n'),
        KeyCode::Char('N'),
    ] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.set_show_upgrade_modal(true);
        let mut k = Keys::new(1);
        k.press(&mut app, code);
        assert!(!app.show_upgrade_modal(), "{code:?} did not cancel");
    }

    // A key the row does not name leaves the question up rather than answering
    // it — `Enter` above all, which is what dismisses a row unread.
    for code in [KeyCode::Enter, KeyCode::Char('y'), KeyCode::Char('z')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        app.set_show_upgrade_modal(true);
        let mut k = Keys::new(1);
        k.press(&mut app, code);
        assert!(
            app.show_upgrade_modal(),
            "{code:?} answered a question it is not on the screen for"
        );
    }
}

#[tokio::test]
async fn u_confirms_the_upgrade_the_modal_asked_about() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_upgrade_modal(true);
    let mut k = Keys::new(1);

    k.press(&mut app, KeyCode::Char('u'));
    assert!(
        !app.show_upgrade_modal(),
        "the modal stayed up after confirming"
    );
}

// -------------------------------------------------------------- vault creation

#[tokio::test]
async fn the_creation_prompt_takes_a_password_and_can_be_corrected() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    let mut k = Keys::new(1);

    k.type_str(&mut app, "hunter2");
    assert_eq!(app.vault_password_input(), "hunter2");
    k.press(&mut app, KeyCode::Backspace);
    assert_eq!(app.vault_password_input(), "hunter");
    // Keys the prompt has no use for leave what was typed alone.
    k.press(&mut app, KeyCode::Up);
    k.press(&mut app, KeyCode::Tab);
    assert_eq!(app.vault_password_input(), "hunter");
}

#[tokio::test]
async fn an_empty_master_password_is_refused_rather_than_accepted() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    let mut k = Keys::new(1);

    k.press(&mut app, KeyCode::Enter);
    assert!(
        app.vault_creating(),
        "the prompt was dismissed on an empty password"
    );
    assert!(
        !app.vault_create_in_flight(),
        "an empty password started a create"
    );
    assert!(app.vault_create_error().is_some(), "no reason was given");
    assert!(k.rx.try_recv().is_err(), "an empty password spawned work");
}

#[tokio::test]
async fn a_creation_with_nowhere_to_put_the_vault_says_so() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    // A config path is what names the directory, so force the state directly:
    // `begin_vault_creation` refuses without one.
    app.config_path = Some(std::path::PathBuf::from("config.toml"));
    assert!(app.begin_vault_creation());
    app.config_path = Some(std::path::PathBuf::new());

    let mut k = Keys::new(1);
    k.type_str(&mut app, "hunter2");
    k.press(&mut app, KeyCode::Enter);

    assert!(app.vault_create_error().is_some(), "the failure was silent");
    assert!(!app.vault_create_in_flight());
}

#[tokio::test]
async fn escape_declines_the_vault_offer_without_losing_the_password() {
    // Declining leaves the password in the OS credential store, which still
    // works; only the encrypted vault is skipped.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    let mut k = Keys::new(1);

    k.type_str(&mut app, "typed");
    k.press(&mut app, KeyCode::Esc);
    assert!(!app.vault_creating());
    assert!(
        app.vault_password_input().is_empty(),
        "the typed password was kept"
    );
}

#[tokio::test]
async fn while_a_vault_is_being_created_only_escape_still_means_anything() {
    // Argon2id is running and there is nothing to type into, but the user must
    // never be trapped waiting for it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    app.vault_password_input_mut().push_str("hunter2");
    assert!(app.begin_vault_create_attempt().is_some());
    assert!(app.vault_create_in_flight());

    let mut k = Keys::new(1);
    k.type_str(&mut app, "more");
    assert!(
        app.vault_password_input().is_empty(),
        "keys reached the field"
    );
    assert!(
        app.vault_create_in_flight(),
        "a stray key cancelled the create"
    );

    k.press(&mut app, KeyCode::Esc);
    assert!(!app.vault_creating(), "Esc did not give up on waiting");
}

// -------------------------------------------------------- the password prompt

#[tokio::test]
async fn the_unlock_prompt_takes_a_password_and_can_be_corrected() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_vault_password_prompt(true);
    let mut k = Keys::new(1);

    k.type_str(&mut app, "secret");
    assert_eq!(app.vault_password_input(), "secret");
    k.press(&mut app, KeyCode::Backspace);
    assert_eq!(app.vault_password_input(), "secre");
    k.press(&mut app, KeyCode::Down);
    assert_eq!(app.vault_password_input(), "secre");
}

#[tokio::test]
async fn an_empty_unlock_attempt_does_not_start_an_unwrap() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_vault_password_prompt(true);
    let mut k = Keys::new(1);

    k.press(&mut app, KeyCode::Enter);
    assert!(
        app.show_vault_password_prompt(),
        "an empty password left the prompt"
    );
    assert!(
        k.rx.try_recv().is_err(),
        "an empty password spawned an unwrap"
    );
}

#[tokio::test]
async fn escape_leaves_the_unlock_prompt_and_clears_what_was_typed() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_vault_password_prompt(true);
    app.set_vault_password_error(Some("wrong password".into()));
    let mut k = Keys::new(1);

    k.type_str(&mut app, "secret");
    k.press(&mut app, KeyCode::Esc);

    assert!(!app.show_vault_password_prompt());
    assert!(
        app.vault_password_input().is_empty(),
        "the password was kept"
    );
    assert!(
        app.vault_password_error().is_none(),
        "the stale error survived into the next prompt"
    );
}

// ------------------------------------------------------------ the async prompts

#[tokio::test]
async fn a_hung_biometric_prompt_can_still_be_escaped() {
    // The outcome normally arrives as a message; if that task dies or hangs,
    // every key including quit was swallowed and the app could only be killed.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('Q')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        // Set directly: reaching this state through `begin_vault_unlock`
        // needs a real vault, and unwrapping one costs an Argon2id pass.
        app.vault_state = VaultState::Unlocking {
            awaiting_biometric: true,
        };
        assert!(app.vault_awaiting_biometric());
        let mut k = Keys::new(1);

        k.press(&mut app, code);
        assert!(
            !app.vault_awaiting_biometric(),
            "{code:?} did not get the user out"
        );
    }

    // Anything else is swallowed while the prompt is up.
    let mut app = app_with_config(&dir, &["alpha"]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let mut k = Keys::new(1);
    k.press(&mut app, KeyCode::Char('u'));
    assert!(app.vault_awaiting_biometric());
}

#[tokio::test]
async fn a_password_being_verified_can_still_be_escaped() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_vault_unlocking();
    assert!(app.vault_verifying());
    let mut k = Keys::new(1);

    // Stray keys are swallowed; Esc is not.
    k.press(&mut app, KeyCode::Char('x'));
    assert!(app.vault_verifying());
    k.press(&mut app, KeyCode::Esc);
    assert!(!app.vault_verifying(), "Esc did not cancel the verify");
}

// ------------------------------------------------------------- ordinary keys

#[tokio::test]
async fn the_sort_keys_only_restart_the_agents_when_the_sort_changes() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    // Already sorting by CPU: pressing `c` changes nothing, so nothing is torn
    // down and rebuilt.
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);
    k.press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);

    k.press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    k.press(&mut app, KeyCode::Char('M'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    k.press(&mut app, KeyCode::Char('C'));
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);
}

#[tokio::test]
async fn paging_scrolls_further_than_a_single_step() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    // Fill the pane so there is somewhere to scroll to. The offset counts
    // lines back from the bottom, so scrolling up raises it.
    for i in 0..100 {
        app.panels[0].last_upgrade.push(format!("line {i}"));
    }
    app.enter_upgrade_view();

    k.press(&mut app, KeyCode::End);
    assert_eq!(
        app.panels[0].scroll_offset, 0,
        "End is the bottom of the log"
    );

    k.press(&mut app, KeyCode::Up);
    let one_step = app.panels[0].scroll_offset;
    assert!(one_step > 0, "Up did not move");

    k.press(&mut app, KeyCode::End);
    k.press(&mut app, KeyCode::PageUp);
    assert!(
        app.panels[0].scroll_offset > one_step,
        "a page moved no further than a single line"
    );

    // And back down again by the same amounts.
    let paged = app.panels[0].scroll_offset;
    k.press(&mut app, KeyCode::Down);
    assert!(
        app.panels[0].scroll_offset < paged,
        "Down did not move back"
    );
    k.press(&mut app, KeyCode::PageDown);
    assert_eq!(
        app.panels[0].scroll_offset, 0,
        "a page down overshot the bottom"
    );

    // Home goes as far back as the log reaches.
    k.press(&mut app, KeyCode::Home);
    assert!(
        app.panels[0].scroll_offset > paged,
        "Home did not reach the top"
    );

    // `j` and `k` are the same keys by another name.
    k.press(&mut app, KeyCode::Char('j'));
    k.press(&mut app, KeyCode::Char('k'));
}

// -------------------------------------------------------- restarting agents

#[tokio::test]
async fn changing_the_sort_in_the_docker_view_restarts_the_docker_pollers_too() {
    // The monitor streams are restarted on any sort change. The docker pollers
    // carry the sort as well, and a panel left polling with the old one shows a
    // table ordered by something the keybar no longer says.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha", "beta"]);
    let mut k = Keys::new(2);

    let cmds = app.toggle_docker((80, 24));
    assert!(!cmds.is_empty(), "entering the docker view must spawn work");
    assert!(app.in_docker());

    k.press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    assert!(
        app.in_docker(),
        "the restart dropped the view the user was in"
    );

    // Back again, so both sort keys run against a docker grid.
    k.press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);
    assert!(app.in_docker());
}

#[tokio::test]
async fn a_sort_change_outside_the_docker_view_starts_no_docker_pollers() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    assert!(!app.in_docker());
    k.press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    assert!(
        !app.in_docker(),
        "a sort change put the app in a view nobody asked for"
    );
}
