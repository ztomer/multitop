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
