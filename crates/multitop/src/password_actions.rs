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
                // Rebuild the panels through `replace_panels`, which bumps every
                // generation. Assigning `app.panels` directly left the running
                // monitor tasks matching their old indices, so after a deletion
                // the task for the removed host painted whichever host had moved
                // into its slot. It also carries credentials across the swap.
                app.replace_panels(new_servers);
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
        PasswordAction::ApplyServerEdit {
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
                // `None` means the field was emptied on purpose: take the
                // stored password back rather than keeping one the editor no
                // longer shows.
                let follow_up =
                    password.map_or(PasswordAction::Delete { panel: target_idx }, |password| {
                        PasswordAction::Save {
                            panel: target_idx,
                            password,
                            resume_upgrade: false,
                        }
                    });
                apply(follow_up, app, servers, tx, tasks);
            }
        }
        PasswordAction::Delete { panel } => {
            let result = crate::password_store::delete(&app.panels[panel].server);
            app.panels[panel].sudo_password = None;
            app.panels[panel].password_saved = false;
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some(match result {
                    Ok(()) => "Saved password removed; this host now has none.".to_string(),
                    Err(error) => format!("Could not remove saved password: {error}"),
                });
            }
        }
        PasswordAction::RotateVaultPassword { current, new } => {
            let Some(vault) = app.vault.clone() else {
                if let Some(manager) = app.password_manager.as_mut() {
                    manager.notice = Some("There is no vault to rotate.".to_string());
                }
                return;
            };
            // Argon2id runs twice here -- once to open with the old password,
            // once to wrap with the new -- so this cannot happen on the event
            // loop without freezing the UI for seconds. Same treatment as the
            // unlock path: hand it to a blocking thread, report by message, and
            // stamp it with the vault epoch so a result the user has already
            // moved on from is discarded.
            let epoch = app.bump_vault_epoch();
            let tx2 = tx.clone();
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some("Changing the master password...".to_string());
            }
            tokio::task::spawn_blocking(move || {
                let msg = match vault.change_password(&current, &new) {
                    Ok(()) => crate::app::Msg::VaultPasswordRotated { epoch },
                    Err(e) => crate::app::Msg::VaultPasswordRotationFailed {
                        epoch,
                        error: e.to_string(),
                    },
                };
                let _ = tx2.blocking_send(msg);
            });
        }
        PasswordAction::ImportSshHosts => {
            let existing: Vec<Server> = app.panels.iter().map(|p| p.server.clone()).collect();
            let outcome = crate::config::ssh_config_path()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .map(|text| {
                    crate::config::merge_ssh_hosts(
                        &existing,
                        crate::config::parse_ssh_config(&text),
                    )
                });
            match outcome {
                Some((_, 0)) => {
                    if let Some(manager) = app.password_manager.as_mut() {
                        manager.notice =
                            Some("No new hosts in ~/.ssh/config; nothing was changed.".to_string());
                    }
                }
                Some((merged, added)) => {
                    // Delegated rather than reimplemented: ApplyServers writes
                    // config.toml, rebuilds the panels through `replace_panels`
                    // so stale tasks are retired, and carries credentials across.
                    apply(
                        PasswordAction::ApplyServers(merged),
                        app,
                        servers,
                        tx,
                        tasks,
                    );
                    if let Some(manager) = app.password_manager.as_mut() {
                        let plural = if added == 1 { "host" } else { "hosts" };
                        manager.notice = Some(format!(
                            "Imported {added} {plural} from ~/.ssh/config; existing entries were left alone."
                        ));
                    }
                }
                None => {
                    if let Some(manager) = app.password_manager.as_mut() {
                        manager.notice = Some("Could not read ~/.ssh/config.".to_string());
                    }
                }
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
    // Server Settings stays open behind the prompt. It used to be closed here,
    // because `ui::draw` returned early for the configuration panel and the
    // prompt would otherwise have been invisible -- so answering it dropped the
    // user back on the stats screen, having lost the panel they were working
    // in. The renderer now draws modals over the panel, so this is just a modal
    // over the list, and Esc leaves the list where it was.
    app.begin_vault_creation();
}
