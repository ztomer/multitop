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

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        upgrade_cmd: None,
    }
}

/// An app with a real vault file present, so rotation is offered.
fn app_with_vault(tag: &str) -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mt_rot_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");

    let mut app = App::new(vec![server("host-a")]);
    app.config_path = Some(config);
    let vault = multitop_vault::Vault::new(multitop_vault::VaultConfig {
        vault_path: dir.join("vault.bin"),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        }),
        use_os_keychain: false,
    });
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize("old-master"))
        .unwrap();
    app.vault = Some(std::sync::Arc::new(vault));
    app.password_manager = Some(PasswordManager::new(0, false));
    (app, dir)
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
    let (mut app, dir) = app_with_vault("flow");

    assert!(matches!(
        handle_key(&mut app, KeyCode::Char('r')),
        PasswordAction::None
    ));
    let m = app.password_manager.as_ref().unwrap();
    assert!(m.editing(), "a prompt must be open");
    assert!(m.is_rotating(), "and it must be the rotation prompt");
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
    assert!(m.is_rotating(), "still rotating after the first password");
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
    let (mut app, dir) = app_with_vault("esc");

    handle_key(&mut app, KeyCode::Char('r'));
    type_in(&mut app, "old-master");
    handle_key(&mut app, KeyCode::Enter);
    // Now at the second prompt, holding the current password.
    let action = handle_key(&mut app, KeyCode::Esc);

    assert!(matches!(action, PasswordAction::None));
    let m = app.password_manager.as_ref().unwrap();
    assert!(!m.editing(), "the prompt must be closed");
    assert!(!m.is_rotating(), "and the carried password released");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_password_does_not_rotate() {
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
    let (mut app, dir) = app_with_vault("text");

    handle_key(&mut app, KeyCode::Char('s')); // SSO prompt
    type_in(&mut app, "supersecret");
    let m = app.password_manager.as_ref().unwrap();
    assert!(!m.is_rotating(), "typing r must not start a rotation");
    assert_eq!(
        m.input, "supersecret",
        "the r in 'supersecret' must land in the field"
    );

    let _ = std::fs::remove_dir_all(dir);
}
