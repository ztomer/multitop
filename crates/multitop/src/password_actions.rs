//! Runtime effects requested by the password portion of Configuration.

use tokio::sync::mpsc::Sender;

use crate::app::{App, Msg};
use crate::config::Server;
use crate::passwords::PasswordAction;
use crate::run::Tasks;

pub fn apply(
    action: PasswordAction,
    app: &mut App,
    servers: &[Server],
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    match action {
        PasswordAction::None => {}
        PasswordAction::ApplyServers(new_servers) => {
            let result = app
                .config_path
                .as_deref()
                .ok_or_else(|| "No configuration file is active.".to_string())
                .and_then(|path| crate::config::save_servers(path, &new_servers));
            if result.is_ok() {
                // Update app.panels to match the new server list while preserving existing passwords.
                let mut new_panels = Vec::with_capacity(new_servers.len());
                for server in new_servers {
                    let mut panel = crate::app::Panel::new(server.clone());
                    if let Some(old_panel) = app.panels.iter().find(|p| p.server.host == server.host) {
                        panel.sudo_password = old_panel.sudo_password.clone();
                        panel.password_saved = old_panel.password_saved;
                    }
                    new_panels.push(panel);
                }
                app.panels = new_panels;
                if app.panels.is_empty() {
                    app.selected_panel = 0;
                } else {
                    app.selected_panel = app.selected_panel.min(app.panels.len() - 1);
                }
            }
            if let Some(manager) = app.password_manager.as_mut() {
                if !app.panels.is_empty() {
                    manager.selected = manager.selected.min(app.panels.len() - 1);
                }
                manager.notice = Some(match result {
                    Ok(()) => "Server configuration saved.".to_string(),
                    Err(error) => format!("Could not save server configuration: {error}"),
                });
            }
        }
        PasswordAction::SaveServerWithPassword {
            servers: new_servers,
            target_idx,
            password,
        } => {
            apply(PasswordAction::ApplyServers(new_servers), app, servers, tx, tasks);
            if target_idx < app.panels.len() {
                apply(
                    PasswordAction::Save {
                        panel: target_idx,
                        password,
                        resume_upgrade: false,
                    },
                    app,
                    servers,
                    tx,
                    tasks,
                );
            }
        }
        PasswordAction::Delete { panel } => {
            let result = crate::password_store::delete(&app.panels[panel].server);
            app.panels[panel].sudo_password = None;
            app.panels[panel].password_saved = false;
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some(match result {
                    Ok(()) => "Saved password removed.".to_string(),
                    Err(error) => format!("Could not remove saved password: {error}"),
                });
            }
        }
        PasswordAction::SaveSso { password } => {
            let result = crate::password_store::save_sso(&password);
            for panel in &mut app.panels {
                if panel.sudo_password.is_none() {
                    panel.sudo_password = Some(password.clone());
                    panel.password_saved = result.is_ok();
                }
            }
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some(match result {
                    Ok(()) => "Single Sign-On (SSO) master password saved.".to_string(),
                    Err(error) => format!("Could not save SSO password: {error}"),
                });
            }
        }
        PasswordAction::DeleteSso => {
            let result = crate::password_store::delete_sso();
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some(match result {
                    Ok(()) => "Single Sign-On (SSO) master password removed.".to_string(),
                    Err(error) => format!("Could not remove SSO password: {error}"),
                });
            }
        }
        PasswordAction::ToggleSparklines => {
            app.show_sparklines = !app.show_sparklines;
            if let Some(path) = &app.config_path {
                crate::config::save_show_sparklines(path, app.show_sparklines);
            }
            if let Some(manager) = app.password_manager.as_mut() {
                let status = if app.show_sparklines {
                    "Enabled"
                } else {
                    "Disabled"
                };
                manager.notice = Some(format!("Sparklines (Experimental): {status}"));
            }
        }
        PasswordAction::Save {
            panel,
            password,
            resume_upgrade,
        } => {
            app.panels[panel].sudo_password = Some(password.clone());
            let result = crate::password_store::save(&app.panels[panel].server, &password);
            app.panels[panel].password_saved = result.is_ok();
            if let Some(manager) = app.password_manager.as_mut() {
                manager.resume_upgrade = false;
                manager.notice = Some(match result {
                    Ok(()) => "Password saved securely in system credential store.".to_string(),
                    Err(error) => {
                        format!("Password kept for this session; save failed: {error}")
                    }
                });
            }
            let should_resume = resume_upgrade || app.panels[panel].mode == crate::app::Mode::Upgrade;
            if should_resume && servers.get(panel).and_then(|s| s.upgrade_cmd.as_ref()).is_some() {
                let gen = app.bump(panel);
                let palette = app.current_theme();
                app.panels[panel].mode = crate::app::Mode::Upgrade;
                app.panels[panel].view = vec![format!(
                    "{}\u{2192} Upgrade running...{}",
                    palette.meter_mid(),
                    palette.reset
                )];
                let handle = crate::tasks::spawn_upgrade(
                    panel,
                    gen,
                    servers[panel].clone(),
                    Some(password),
                    tx.clone(),
                );
                tasks.aux_is_upgrade[panel] = true;
                if let Some(old) = tasks.aux[panel].replace(handle) {
                    old.abort();
                }
            }
        }
    }
}
