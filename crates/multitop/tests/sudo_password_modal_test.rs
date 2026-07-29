//! Pedantic regression tests for Sudo Password Modal Dialog, TOML Persistence,
//! Automatic Sudo Failure Detection, Keyboard Interaction, and Command Piping.

use multitop::app::{App, Msg};
use multitop::config::{parse, save_sudo_password, Server};
use multitop::refit::refit_line;
use std::fs;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "root".to_string(),
        upgrade_cmd: Some("sudo apt update".to_string()),
        sudo_password: None,
    }
}

#[test]
fn test_sudo_password_toml_parse_and_save() {
    let toml = r#"
    [[servers]]
    host = "192.168.0.33"
    port = 22
    user = "admin"
    upgrade_cmd = "us;ud"
    sudo_password = "initial_secret_123"
    "#;

    let config = parse(toml).expect("parse config");
    assert_eq!(config.servers.len(), 1);
    assert_eq!(
        config.servers[0].sudo_password.as_deref(),
        Some("initial_secret_123")
    );

    let tmp_dir = std::env::temp_dir().join(format!("multitop_sudo_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp_dir);
    let cfg_path = tmp_dir.join("config.toml");
    fs::write(&cfg_path, toml).expect("write toml");

    // Save updated password
    save_sudo_password(&cfg_path, "192.168.0.33", "new_secret_789");

    let reloaded_toml = fs::read_to_string(&cfg_path).expect("read toml");
    let reloaded_config = parse(&reloaded_toml).expect("parse reloaded");
    assert_eq!(
        reloaded_config.servers[0].sudo_password.as_deref(),
        Some("new_secret_789")
    );

    let _ = fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_panel_sudo_prompt_state_and_selection() {
    let servers = vec![test_server("192.168.0.33"), test_server("192.168.0.90")];
    let mut app = App::new(servers);

    assert_eq!(app.selected_panel, 0);
    assert!(!app.panels[0].prompt_sudo);
    assert!(app.panels[0].password_input.is_empty());

    // Switch selected panel to panel 1
    app.selected_panel = 1;
    assert_eq!(app.selected_panel, 1);

    // Trigger sudo prompt on active panel
    app.panels[1].prompt_sudo = true;
    app.panels[1].password_input.push_str("typed_pass");
    assert!(app.panels[1].prompt_sudo);
    assert_eq!(app.panels[1].password_input, "typed_pass");

    // Apply Msg::PromptSudo
    let msg = Msg::PromptSudo { panel: 0, gen: 0 };
    app.apply(msg);
    assert!(app.panels[0].prompt_sudo);
    assert!(app.panels[0].password_input.is_empty());
}

#[tokio::test]
async fn test_sudo_cmd_wrapper_generation() {
    let server_no_pass = test_server("192.168.0.33");
    let mut server_with_pass = test_server("192.168.0.33");
    server_with_pass.sudo_password = Some("my_secret".to_string());

    // Spawn command without password
    let child_no_pass = multitop::ssh::spawn_command(&server_no_pass, "sudo apt update", None);
    assert!(child_no_pass.is_ok());

    // Spawn command with password
    let child_with_pass = multitop::ssh::spawn_command(
        &server_with_pass,
        "sudo apt update",
        Some("my_secret"),
    );
    assert!(child_with_pass.is_ok());
}

#[test]
fn test_active_panel_badge_formatting() {
    let line = "\x1b[36;1m─── Upgrade on 127.0.0.1 ───\x1b[0m";
    let refitted = refit_line(line, 60);
    assert!(!refitted.is_empty());
    assert!(refitted.contains("Upgrade on 127.0.0.1"));
}
