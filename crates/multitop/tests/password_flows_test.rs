//! Saving, removing and importing credentials, and what the user is told.
//!
//! The reporting is the point. Telling someone "saved securely" when only half
//! the write landed, or "removed" when a copy is still in the vault and will
//! come back, is worse than an error — they stop looking.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::password_actions::apply;
use multitop::password_store;
use multitop::passwords::{open, PasswordAction};
use multitop::run::Tasks;
use tokio::sync::mpsc;

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

struct Harness {
    app: App,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    tasks: Tasks,
}

impl Harness {
    fn new(hosts: &[&str]) -> Self {
        let app = App::new(hosts.iter().map(|h| test_server(h)).collect());
        let (tx, rx) = mpsc::channel::<Msg>(64);
        let tasks = Tasks::new(hosts.len());
        Self { app, tx, rx, tasks }
    }

    /// Open Server Settings, which is what puts the notice surface on screen.
    fn with_settings_open(mut self) -> Self {
        open(&mut self.app, 0, false);
        self
    }

    fn run(&mut self, action: PasswordAction) {
        apply(action, &mut self.app, &self.tx, &mut self.tasks);
    }

    fn notice(&self) -> String {
        self.app
            .password_manager
            .as_ref()
            .and_then(|m| m.notice.clone())
            .unwrap_or_default()
    }
}

// -------------------------------------------------------------------- saving

#[tokio::test]
async fn a_saved_password_is_reported_as_saved() {
    let _g = isolate().await;
    let mut h = Harness::new(&["alpha"]).with_settings_open();

    h.run(PasswordAction::Save {
        panel: 0,
        password: "hunter2".into(),
        resume_upgrade: false,
    });

    assert!(h.notice().contains("saved securely"), "{}", h.notice());
    assert!(h.app.panels[0].password_saved);
    assert_eq!(h.app.panels[0].sudo_password.as_deref(), Some("hunter2"));
    // The value really is in the store, not merely reported as such.
    assert_eq!(
        password_store::load(&test_server("alpha"))
            .unwrap()
            .as_deref(),
        Some("hunter2")
    );
}

#[tokio::test]
async fn saving_a_password_offers_the_vault_that_would_hold_it() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(&["alpha"]).with_settings_open();
    h.app.config_path = Some(dir.path().join("config.toml"));

    h.run(PasswordAction::Save {
        panel: 0,
        password: "hunter2".into(),
        resume_upgrade: false,
    });
    assert!(
        h.app.vault_creating(),
        "the password landed in the keychain and the vault was never offered"
    );
}

#[tokio::test]
async fn the_vault_offer_never_interrupts_a_question_already_on_screen() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(&["alpha"]).with_settings_open();
    h.app.config_path = Some(dir.path().join("config.toml"));
    h.app.set_show_upgrade_modal(true);

    h.run(PasswordAction::Save {
        panel: 0,
        password: "hunter2".into(),
        resume_upgrade: false,
    });
    assert!(
        h.app.show_upgrade_modal(),
        "the vault offer took over a modal the user was answering"
    );
    assert!(!h.app.vault_creating());
}

// ------------------------------------------------------------------ removing

#[tokio::test]
async fn removing_a_password_says_the_host_now_has_none() {
    let _g = isolate().await;
    let mut h = Harness::new(&["alpha"]).with_settings_open();
    h.run(PasswordAction::Save {
        panel: 0,
        password: "hunter2".into(),
        resume_upgrade: false,
    });

    h.run(PasswordAction::Delete { panel: 0 });
    assert!(h.notice().contains("now has none"), "{}", h.notice());
    assert!(!h.app.panels[0].password_saved);
    assert_eq!(password_store::load(&test_server("alpha")).unwrap(), None);
}

// -------------------------------------------------------------- ssh import

#[tokio::test]
async fn importing_from_a_missing_ssh_config_says_it_could_not_be_read() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(&["alpha"]).with_settings_open();
    // A config path whose sibling ~/.ssh/config does not exist.
    h.app.config_path = Some(dir.path().join("config.toml"));

    h.run(PasswordAction::ImportSshHosts);
    let notice = h.notice();
    assert!(
        !notice.is_empty(),
        "an import that did nothing said nothing"
    );
}

// ------------------------------------------------------------ vault rotation

#[tokio::test]
async fn rotating_with_no_vault_says_there_is_nothing_to_rotate() {
    let _g = isolate().await;
    let mut h = Harness::new(&["alpha"]).with_settings_open();

    h.run(PasswordAction::RotateVaultPassword {
        current: "old".into(),
        new: "new".into(),
    });
    assert!(h.notice().contains("no vault"), "{}", h.notice());
    assert!(
        h.rx.try_recv().is_err(),
        "work was spawned with no vault to do it on"
    );
}

// ---------------------------------------------------------------- banner key

#[tokio::test]
async fn cycling_the_banner_style_comes_back_round() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    // A hand-maintained file: the comment must survive the rewrite.
    std::fs::write(
        &config,
        "# my servers\nbanner_style = \"wide\"\n\n[[servers]]\nhost = \"alpha\"\n",
    )
    .unwrap();

    let mut h = Harness::new(&["alpha"]).with_settings_open();
    h.app.config_path = Some(config.clone());

    let first = h.app.banner_style;
    h.run(PasswordAction::CycleBannerStyle);
    assert_ne!(h.app.banner_style, first, "the style did not move");

    let after = std::fs::read_to_string(&config).unwrap();
    assert!(
        after.contains("# my servers"),
        "rewriting the config dropped the user's comments:\n{after}"
    );
    assert!(after.contains("host = \"alpha\""), "{after}");
}

#[tokio::test]
async fn a_config_that_cannot_be_parsed_is_left_alone_rather_than_rewritten() {
    // The mock store is process-global, so every test in this file holds the
    // same guard for its whole body — including the ones that only touch the
    // filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Overwriting a file that failed to parse would destroy whatever the user
    // was in the middle of editing.
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let damaged = "this is not = = toml\n";
    std::fs::write(&config, damaged).unwrap();

    multitop::config::save_theme(&config, "kare");
    multitop::config::save_banner_style(&config, multitop::layout::BannerStyle::parse("plain"));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), damaged);

    // A file that is not there at all is also left alone rather than created.
    let absent = dir.path().join("nope.toml");
    multitop::config::save_theme(&absent, "kare");
    multitop::config::save_banner_style(&absent, multitop::layout::BannerStyle::parse("plain"));
    assert!(!absent.exists());
}

#[tokio::test]
async fn saving_a_theme_keeps_the_rest_of_the_file_as_written() {
    // The mock store is process-global, so every test in this file holds the
    // same guard for its whole body — including the ones that only touch the
    // filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "# keep me\ntheme = \"old\"\n\n[[servers]]\n# and me\nhost = \"alpha\"\n",
    )
    .unwrap();

    multitop::config::save_theme(&config, "kare");
    let after = std::fs::read_to_string(&config).unwrap();
    assert!(after.contains("kare"), "{after}");
    assert!(after.contains("# keep me"), "a comment was lost:\n{after}");
    assert!(after.contains("# and me"), "a comment was lost:\n{after}");
}

#[tokio::test]
async fn stripping_a_plaintext_password_touches_the_file_only_when_there_is_one() {
    // The mock store is process-global, so every test in this file holds the
    // same guard for its whole body — including the ones that only touch the
    // filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");

    // Nothing to strip: the file must not be rewritten at all.
    let clean = "# untouched\n[[servers]]\nhost = \"alpha\"\n";
    std::fs::write(&config, clean).unwrap();
    assert_eq!(
        multitop::config::strip_plaintext_passwords(&config).unwrap(),
        0
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), clean);

    // One to strip: gone, and the comment stays.
    std::fs::write(
        &config,
        "# untouched\n[[servers]]\nhost = \"alpha\"\nsudo_password = \"hunter2\"\n",
    )
    .unwrap();
    assert_eq!(
        multitop::config::strip_plaintext_passwords(&config).unwrap(),
        1
    );
    let after = std::fs::read_to_string(&config).unwrap();
    assert!(!after.contains("hunter2"), "{after}");
    assert!(after.contains("# untouched"), "{after}");
}
