//! Regression tests for the full-screen configuration and password workflow.

use crossterm::event::KeyCode;
use multitop::app::App;
use multitop::config::Server;
use multitop::passwords::{self, ConfigSection, PasswordAction};

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "root".to_string(),
        upgrade_cmd: Some("sudo apt update".to_string()),
    }
}

#[test]
fn password_entry_accepts_numeric_characters() {
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, false);
    let _ = passwords::handle_key(&mut app, KeyCode::Enter);
    for character in "pa55w0rd9".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let manager = app.password_manager.as_ref().expect("configuration open");
    assert!(manager.editing);
    assert_eq!(manager.input, "pa55w0rd9");
}

#[test]
fn server_settings_manager_opens_and_sets_resume_upgrade() {
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, true);
    let manager = app.password_manager.as_ref().expect("settings open");
    assert_eq!(manager.section, ConfigSection::Passwords);
    assert!(manager.resume_upgrade);
}

#[test]
fn server_editor_creates_a_configured_server() {
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, false);
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    let _ = passwords::handle_key(&mut app, KeyCode::Char('a'));
    for character in "new-host".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    for character in "deploy".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    let _ = passwords::handle_key(&mut app, KeyCode::Backspace);
    let _ = passwords::handle_key(&mut app, KeyCode::Backspace);
    for character in "2222".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let action = passwords::handle_key(&mut app, KeyCode::Enter);
    let PasswordAction::ApplyServers(servers) = action else {
        panic!("expected server update")
    };
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[1].host, "new-host");
    assert_eq!(servers[1].user, "deploy");
    assert_eq!(servers[1].port, 2222);
}

#[test]
fn test_parse_ssh_config_multi_alias() {
    let ssh_config = r#"
Host web db app
    HostName 192.168.1.100
    User admin
    Port 2222
"#;
    let servers = multitop::config::parse_ssh_config(ssh_config);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].host, "192.168.1.100");
    assert_eq!(servers[0].user, "admin");
    assert_eq!(servers[0].port, 2222);
}

#[test]
fn test_save_new_server_with_password() {
    let mut app = App::new(vec![server("one")]);
    passwords::open(&mut app, 0, false);
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    let _ = passwords::handle_key(&mut app, KeyCode::Char('a'));
    for character in "new-server".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    // Tab to user
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    for character in "root".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    // Tab to port
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    // Tab to upgrade_cmd
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    // Tab to password
    let _ = passwords::handle_key(&mut app, KeyCode::Tab);
    for character in "secret123".chars() {
        let _ = passwords::handle_key(&mut app, KeyCode::Char(character));
    }
    let action = passwords::handle_key(&mut app, KeyCode::Enter);
    let PasswordAction::SaveServerWithPassword {
        servers,
        target_idx,
        password,
    } = action
    else {
        panic!("expected SaveServerWithPassword action");
    };
    assert_eq!(servers.len(), 2);
    assert_eq!(target_idx, 1);
    assert_eq!(password, "secret123");
}
