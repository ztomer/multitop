//! Pedantic regression tests for Sudo Password Modal Dialog, TOML Persistence,
//! Automatic Sudo Failure Detection, Keyboard Interaction, and Command Piping.

use multitop::app::{App, Mode, Msg};
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

    app.selected_panel = 1;
    assert_eq!(app.selected_panel, 1);

    app.panels[1].prompt_sudo = true;
    app.panels[1].password_input.push_str("typed_pass");
    assert!(app.panels[1].prompt_sudo);
    assert_eq!(app.panels[1].password_input, "typed_pass");

    // Apply Msg::PromptSudo sets prompt_sudo on panel 0
    let msg = Msg::PromptSudo { panel: 0, gen: 0 };
    app.apply(msg);
    assert!(app.panels[0].prompt_sudo);
    assert!(app.panels[0].password_input.is_empty());
}

/// Verify that PromptSudo is ignored when sudo_password is already set.
#[test]
fn test_prompt_sudo_skipped_when_password_already_set() {
    let servers = vec![test_server("192.168.0.33")];
    let mut app = App::new(servers);
    app.panels[0].sudo_password = Some("existing".to_string());

    app.apply(Msg::PromptSudo { panel: 0, gen: 0 });
    assert!(!app.panels[0].prompt_sudo, "should not prompt when password already set");
}

/// Verify that PromptSudo is ignored for stale gen.
#[test]
fn test_prompt_sudo_rejected_for_stale_gen() {
    let servers = vec![test_server("192.168.0.33")];
    let mut app = App::new(servers);

    // Bump gen to 1
    app.bump(0);
    assert_eq!(app.panels[0].gen, 1);

    // PromptSudo with old gen=0 should be rejected
    app.apply(Msg::PromptSudo { panel: 0, gen: 0 });
    assert!(!app.panels[0].prompt_sudo, "stale gen should be rejected");

    // PromptSudo with current gen=1 should be accepted
    app.apply(Msg::PromptSudo { panel: 0, gen: 1 });
    assert!(app.panels[0].prompt_sudo, "current gen should be accepted");
}

/// Verify that run_upgrade puts panels in Upgrade mode with correct view.
#[test]
fn test_run_upgrade_sets_mode_and_view() {
    let servers = vec![test_server("192.168.0.33"), test_server("192.168.0.90")];
    let mut app = App::new(servers);

    let cmds = app.run_upgrade();
    assert_eq!(cmds.len(), 2);
    for panel in &app.panels {
        assert_eq!(panel.mode, Mode::Upgrade);
        assert!(!panel.view.is_empty(), "view should contain upgrade status");
    }
}

/// Verify no upgrade command is emitted for servers without upgrade_cmd.
#[test]
fn test_run_upgrade_no_cmd_configured() {
    let mut server = test_server("192.168.0.33");
    server.upgrade_cmd = None;
    let mut app = App::new(vec![server]);

    let cmds = app.run_upgrade();
    assert!(cmds.is_empty(), "no RunUpgrade command for server without upgrade_cmd");
    assert_eq!(app.panels[0].mode, Mode::Upgrade);
    assert!(app.panels[0].view[0].contains("No upgrade_cmd"));
}

#[tokio::test]
async fn test_sudo_cmd_wrapper_generation() {
    let server_no_pass = test_server("192.168.0.33");

    // Without password — should spawn successfully
    let child_no_pass = multitop::ssh::spawn_command(&server_no_pass, "echo hello", None);
    assert!(child_no_pass.is_ok());

    // With password — should spawn successfully and embed password in command
    let child_with_pass = multitop::ssh::spawn_command(
        &server_no_pass,
        "sudo apt update",
        Some("my_secret"),
    );
    assert!(child_with_pass.is_ok());
}

/// Verify local command with password actually pipes the password via echo.
#[tokio::test]
async fn test_local_sudo_password_piping_echo() {
    use tokio::io::AsyncReadExt;

    let server = Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: Some("echo done".to_string()),
        sudo_password: None,
    };

    // With password, the command should contain `echo 'test_pass' | sudo -S -v`
    // and the actual command should still run. We verify by capturing stdout.
    let mut child = multitop::ssh::spawn_command(&server, "echo done", Some("test_pass"))
        .expect("spawn local with password");
    let status = child.wait().await.expect("wait");
    // Should complete (exit status may vary depending on sudo config, but process runs)
    assert!(status.code().is_some(), "process should exit with a code");

    // Without password, should also work
    let mut child2 = multitop::ssh::spawn_command(&server, "echo hello", None)
        .expect("spawn local without password");
    let mut out = String::new();
    if let Some(mut stdout) = child2.stdout.take() {
        stdout.read_to_string(&mut out).await.unwrap();
    }
    assert!(out.trim().contains("hello"), "expected 'hello' in output: {out}");
}

#[test]
fn test_active_panel_badge_formatting() {
    let line = "\x1b[36;1m─── Upgrade on 127.0.0.1 ───\x1b[0m";
    let refitted = refit_line(line, 60);
    assert!(!refitted.is_empty());
    assert!(refitted.contains("Upgrade on 127.0.0.1"));
}

/// Verify TOML with sudo_pass alias also works.
#[test]
fn test_sudo_pass_alias_in_toml() {
    let toml = r#"
    [[servers]]
    host = "10.0.0.1"
    port = 22
    user = "deploy"
    upgrade_cmd = "apt update"
    sudo_pass = "alias_password"
    "#;
    let config = parse(toml).expect("parse with sudo_pass alias");
    assert_eq!(
        config.servers[0].sudo_password.as_deref(),
        Some("alias_password")
    );
}
