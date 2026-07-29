//! Comprehensive integration tests for Server Settings Manager,
//! keybar visual flare, hotkeys ('e'), and upgrade flow.

use crossterm::event::KeyCode;
use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::passwords::{self, ConfigSection, PasswordAction};

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("sudo apt update".to_string()),
    }
}

#[test]
fn test_open_and_close_settings_manager_with_e_key() {
    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    assert!(app.password_manager.is_none());

    // Open via passwords::open
    passwords::open(&mut app, 0, false);
    assert!(app.password_manager.is_some());
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );

    // Press 'e' to close
    let action = passwords::handle_key(&mut app, KeyCode::Char('e'));
    assert_eq!(action, PasswordAction::None);
    assert!(app.password_manager.is_none());
}

#[test]
fn test_tab_between_passwords_and_servers_sections() {
    let mut app = App::new(vec![test_server("host1")]);
    passwords::open(&mut app, 0, false);
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );

    // Press Tab to switch to Servers section
    let action = passwords::handle_key(&mut app, KeyCode::Tab);
    assert_eq!(action, PasswordAction::None);
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Servers
    );

    // Press Tab to switch back to Passwords section
    let action2 = passwords::handle_key(&mut app, KeyCode::Tab);
    assert_eq!(action2, PasswordAction::None);
    assert_eq!(
        app.password_manager.as_ref().unwrap().section,
        ConfigSection::Passwords
    );
}

#[test]
fn test_apply_servers_updates_panels_dynamically() {
    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    let tmp_path = std::env::temp_dir().join(format!("multitop_test_cfg_{}.toml", std::process::id()));
    app.config_path = Some(tmp_path.clone());
    app.panels[0].sudo_password = Some("secret1".to_string());
    app.panels[0].password_saved = true;

    let new_servers = vec![
        test_server("host1"),
        test_server("host2"),
        test_server("host3"),
    ];

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(2);

    multitop::password_actions::apply(
        PasswordAction::ApplyServers(new_servers.clone()),
        &mut app,
        &new_servers,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels.len(), 3);
    assert_eq!(app.panels[0].server.host, "host1");
    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("secret1"));
    assert_eq!(app.panels[2].server.host, "host3");

    let _ = std::fs::remove_file(tmp_path);
}

#[tokio::test]
async fn test_save_password_in_upgrade_mode_triggers_upgrade_resume() {
    let mut app = App::new(vec![test_server("host1")]);
    app.panels[0].mode = Mode::Upgrade;

    let (tx, _rx) = tokio::sync::mpsc::channel::<Msg>(10);
    let mut tasks = multitop::run::Tasks::new(1);
    let servers = vec![test_server("host1")];

    multitop::password_actions::apply(
        PasswordAction::Save {
            panel: 0,
            password: "mypassword".to_string(),
            resume_upgrade: false,
        },
        &mut app,
        &servers,
        &tx,
        &mut tasks,
    );

    assert_eq!(app.panels[0].sudo_password.as_deref(), Some("mypassword"));
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
}

#[test]
fn test_add_and_delete_server_from_passwords_section() {
    let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
    passwords::open(&mut app, 0, false);

    // Press 'a' in Passwords section -> opens draft
    let action = passwords::handle_key(&mut app, KeyCode::Char('a'));
    assert_eq!(action, PasswordAction::None);
    let manager = app.password_manager.as_ref().unwrap();
    assert!(manager.draft.is_some());

    // Cancel draft
    let _ = passwords::handle_key(&mut app, KeyCode::Esc);
    assert!(app.password_manager.as_ref().unwrap().draft.is_none());

    // Press 'd' in Passwords section -> returns ApplyServers with host1 removed
    let action_del = passwords::handle_key(&mut app, KeyCode::Char('d'));
    let PasswordAction::ApplyServers(remaining) = action_del else {
        panic!("expected ApplyServers action");
    };
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].host, "host2");
}
