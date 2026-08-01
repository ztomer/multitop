//! Runtime effects requested by the password portion of Configuration.

use secrecy::SecretString;
use tokio::sync::mpsc::Sender;

use crate::app::{App, Msg};
use crate::config::Server;
use crate::passwords::PasswordAction;
use crate::run::Tasks;

#[allow(clippy::too_many_lines)]
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
                    if let Some(old_panel) =
                        app.panels.iter().find(|p| p.server.host == server.host)
                    {
                        panel.sudo_password.clone_from(&old_panel.sudo_password);
                        panel.password_saved = old_panel.password_saved;
                        panel.external_password = old_panel.external_password;
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
            apply(
                PasswordAction::ApplyServers(new_servers),
                app,
                servers,
                tx,
                tasks,
            );
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
            let show = !app.show_sparklines();
            app.toggle_sparklines();
            if let Some(path) = &app.config_path {
                crate::config::save_show_sparklines(path, show);
            }
            if let Some(manager) = app.password_manager.as_mut() {
                let status = if show { "Enabled" } else { "Disabled" };
                manager.notice = Some(format!("Sparklines (Experimental): {status}"));
            }
        }
        PasswordAction::Save {
            panel,
            password,
            resume_upgrade,
        } => {
            let key = crate::password_store::account(&app.panels[panel].server);
            app.panels[panel].sudo_password = Some(password.clone());
            let result = crate::password_store::save(&app.panels[panel].server, &password);
            let stored = result.is_ok();
            app.panels[panel].password_saved = stored;
            // Also save to vault if unlocked. The result is reported: dropping
            // it told the user "saved securely" whenever the keychain write
            // succeeded, even if the vault -- the thing they created to hold
            // this -- never received it.
            let vault_error = app.vault_unlocked_mut().and_then(|unlocked| {
                unlocked
                    .set_password(key, &SecretString::new(password.clone().into_boxed_str()))
                    .err()
            });
            if let Some(manager) = app.password_manager.as_mut() {
                manager.resume_upgrade = false;
                manager.notice = Some(match (&result, &vault_error) {
                    (Ok(()), None) => {
                        "Password saved securely in system credential store.".to_string()
                    }
                    (Ok(()), Some(e)) => {
                        format!("Saved to the credential store, but NOT to the vault: {e}")
                    }
                    (Err(error), _) => {
                        format!("Password kept for this session; save failed: {error}")
                    }
                });
            }
            // Saving the first password is the moment a vault becomes worth
            // having, so offer to create one here rather than expecting the
            // user to have set one up in advance.
            if stored {
                offer_vault_creation(app);
            }
            let should_resume =
                resume_upgrade || app.panels[panel].mode == crate::app::Mode::Upgrade;
            if should_resume
                && servers
                    .get(panel)
                    .and_then(|s| s.upgrade_cmd.as_ref())
                    .is_some()
            {
                let gen = app.bump(panel);
                let palette = app.current_theme();
                app.panels[panel].mode = crate::app::Mode::Upgrade;
                app.panels[panel].upgrade_state = crate::panel::UpgradeState::STARTED;
                app.panels[panel].upgrade_gen = gen;
                app.upgrade_started_at = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
                if let Some(ref path) = app.config_path {
                    let state = crate::state::AppState {
                        last_update: app.last_update,
                        upgrade_started_at: app.upgrade_started_at,
                        hosts: app.host_updates.clone(),
                    };
                    let _ = crate::state::save_state(path, &state);
                }
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

/// Move plaintext `sudo_password` values out of `config.toml` and into the OS
/// credential store, then delete them from the file.
///
/// A password in `config.toml` is plaintext on a world-readable file *and* was
/// silently ignored by the loader, so it protected nothing and did nothing.
/// This runs once: after it, the key is gone from the file and the value lives
/// where the rest of the passwords do.
pub fn port_plaintext_passwords(
    app: &mut App,
    config_path: &std::path::Path,
    entries: &[(Server, String)],
) {
    let mut moved = 0;
    let mut failed = Vec::new();

    for (server, secret) in entries {
        match crate::password_store::save(server, secret) {
            Ok(()) => {
                moved += 1;
                // Populate the matching panel so this session does not prompt
                // for something we just imported.
                for p in &mut app.panels {
                    if p.server.host == server.host && p.server.port == server.port {
                        p.sudo_password = Some(secret.clone());
                        p.password_saved = true;
                    }
                }
            }
            Err(e) => failed.push(format!("{}: {e}", server.host)),
        }
    }

    // The key is removed whether or not every value made it across. Leaving a
    // plaintext password on disk is the thing being fixed, so it does not get
    // to survive on the grounds that the keychain write failed -- that would
    // keep the secret exactly where it must not be. Anything that failed is
    // named loudly instead, and can be re-entered with `p`.
    let strip = crate::config::strip_plaintext_passwords(config_path);

    let mut note = match strip {
        Ok(n) => format!(
            "Removed {n} plaintext password(s) from config.toml; \
             {moved} moved into the credential store."
        ),
        Err(e) => format!(
            "Moved {moved} password(s) into the credential store, but could not \
             rewrite config.toml: {e}. Delete the sudo_password lines by hand \u{2014} \
             they are plaintext and unused."
        ),
    };
    if !failed.is_empty() {
        use std::fmt::Write as _;
        let _ = write!(
            note,
            "  Could not store {} of them ({}); set those again with p.",
            failed.len(),
            failed.join(", ")
        );
    }

    for p in &mut app.panels {
        p.view.push(note.clone());
    }

    // Porting is adding passwords, so it earns a vault the same way an
    // interactive save does. Otherwise the passwords land in the keychain and
    // the vault stays absent, which reads as "no vault set up" right after the
    // user believes they set their passwords up.
    if moved > 0 {
        offer_vault_creation(app);
    }
}

/// Start vault creation if there is no vault yet.
///
/// Called after a password is stored. The password is already safe in the OS
/// credential store at this point, so declining the prompt costs nothing; the
/// vault adds encryption at rest and one biometric unlock for every host
/// instead of one prompt per host.
fn offer_vault_creation(app: &mut App) {
    // Never interrupt something the user is already answering.
    if app.vault_creating()
        || app.show_vault_password_prompt()
        || app.vault_awaiting_biometric()
        || app.show_upgrade_modal()
    {
        return;
    }
    if app.begin_vault_creation() {
        app.password_manager = None;
    }
}
