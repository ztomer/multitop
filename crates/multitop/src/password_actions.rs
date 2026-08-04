//! Runtime effects requested by the password portion of Configuration.

use secrecy::SecretString;
use tokio::sync::mpsc::Sender;

use crate::app::{App, Msg};
use crate::config::Server;
use crate::passwords::PasswordAction;
use crate::run::Tasks;

/// Write the server list, rebuild the panels from it, and retire what the
/// rebuild orphaned. The one place that decides whether a server-list edit
/// landed.
///
/// Every caller that follows a list write with more work -- storing the
/// password typed in the same editor, saying how many hosts an import added --
/// has to know whether the write succeeded, because they all report through the
/// single `notice` line and a success sentence written after a failure erases
/// it. Returning the answer is what stops that being remembered.
///
/// `Ok` carries the hosts whose upgrade the rebuild interrupted.
fn write_servers(
    app: &mut App,
    new_servers: Vec<Server>,
    tasks: &mut Tasks,
) -> Result<Vec<String>, String> {
    let path = app
        .config_path
        .clone()
        .ok_or_else(|| "No configuration file is active.".to_string())?;
    crate::config::save_servers(&path, &new_servers)?;
    // Read before the swap: `replace_panels` builds fresh panels, and a fresh
    // panel does not remember that a run was under way.
    let interrupted = app.running_upgrade_hosts();
    // Rebuild through `replace_panels`, which bumps every generation. Assigning
    // `app.panels` directly left the running monitor tasks matching their old
    // indices, so after a deletion the task for the removed host painted
    // whichever host had moved into its slot. It also carries credentials
    // across the swap.
    app.replace_panels(new_servers);
    // Every generation just moved, so no message from a running upgrade will
    // ever be accepted again: the run would continue on the remote with nothing
    // able to show it, and then be killed without a word at quit. Stopping it
    // here is the same act, done where it can be said out loud. The
    // confirmation that got here warned first.
    tasks.abort_upgrades();
    Ok(interrupted)
}

/// Say what [`write_servers`] did, and keep the selection on a row that exists.
fn report_server_write(app: &mut App, result: &Result<Vec<String>, String>) {
    if let Some(manager) = app.password_manager.as_mut() {
        if !app.panels.is_empty() {
            manager.selected = manager.selected.min(app.panels.len() - 1);
        }
        manager.notice = Some(match result {
            Ok(interrupted) if interrupted.is_empty() => "Server configuration saved.".to_string(),
            Ok(interrupted) => format!(
                "Server configuration saved. The upgrade on {} was interrupted -- \
                 if it had started installing, remove ~/.cache/multitop/upgrade.lock \
                 on that host before the next run.",
                interrupted.join(", ")
            ),
            Err(error) => format!("Could not save server configuration: {error}"),
        });
    }
}

/// What the panel is currently saying, if anything.
fn current_notice(app: &App) -> Option<String> {
    app.password_manager
        .as_ref()
        .and_then(|manager| manager.notice.clone())
}

/// Put `before` back in front of whatever the notice now says.
///
/// One action with two things to report has one line to report them on, and
/// assigning `notice` in the second half silently erased the first. That is how
/// a config write that failed was reported as a saved password, and how a
/// warning naming an upgrade the edit had just interrupted -- the one thing
/// this panel must say out loud -- disappeared before anything drew it.
///
/// Composite actions capture the first half with [`current_notice`], let the
/// second half report however it likes, and call this. Nothing has to remember
/// which half writes last.
fn prepend_notice(app: &mut App, before: Option<String>) {
    let Some(before) = before else { return };
    if let Some(manager) = app.password_manager.as_mut() {
        manager.notice = Some(match manager.notice.take() {
            Some(after) if after != before => format!("{before} {after}"),
            _ => before,
        });
    }
}

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
            let result = write_servers(app, new_servers, tasks);
            report_server_write(app, &result);
        }
        PasswordAction::ApplyServerEdit {
            servers: new_servers,
            target_idx,
            password,
        } => {
            let result = write_servers(app, new_servers, tasks);
            report_server_write(app, &result);
            // A write that failed left the panels describing the row the editor
            // no longer shows, so the password typed in that editor belongs to
            // no host here -- storing it would put it under the identity the
            // user was editing *away* from. Stop, and leave the failure on
            // screen: the follow-up's notice used to replace it, so a config
            // write that never happened was reported as a saved password.
            if result.is_err() {
                return;
            }
            let reported = current_notice(app);
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
                prepend_notice(app, reported);
            }
        }
        PasswordAction::Delete { panel } => {
            // A password lives in TWO stores, and `Save` writes to both. This
            // wrote to one.
            //
            // The vault is the one that is read *first*: `load_known_passwords`
            // takes the vault's copy and only falls back to the credential
            // store when there is no vault at all. So emptying a host's
            // password field removed the keychain entry, left the vault entry
            // standing, said "this host now has none" -- and the password came
            // straight back on the next unlock. The one asymmetric operation
            // over a two-store credential is the whole defect.
            let key = crate::password_store::account(&app.panels[panel].server);
            let result = crate::password_store::delete(&app.panels[panel].server);
            let vault_error = app
                .vault_unlocked_mut()
                .and_then(|unlocked| unlocked.remove_password(&key).err());
            app.panels[panel].sudo_password = None;
            app.panels[panel].password_saved = false;
            app.panels[panel].external_password = false;
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some(match (&result, &vault_error) {
                    (Ok(()), None) => "Saved password removed; this host now has none.".to_string(),
                    // Named separately, because a password still in the vault is
                    // a password that will come back, and the user has to know
                    // that rather than believe it is gone.
                    (Ok(()), Some(e)) => format!(
                        "Removed from the credential store, but NOT from the vault: {e}. \
                         It will be used again until the vault entry is removed."
                    ),
                    (Err(error), _) => format!("Could not remove saved password: {error}"),
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
                // Marked before the work starts, so `r` is refused for as long
                // as it runs rather than only until the notice is read.
                manager.rotating = true;
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
        PasswordAction::CycleBannerStyle => {
            let style = app.cycle_banner_style();
            if let Some(path) = &app.config_path {
                crate::config::save_banner_style(path, style);
            }
            if let Some(manager) = app.password_manager.as_mut() {
                manager.notice = Some(format!(
                    "Banner: {}. Wide needs a font with fullwidth Latin glyphs.",
                    style.label()
                ));
            }
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
                    // Delegated rather than reimplemented: `write_servers`
                    // writes config.toml, rebuilds the panels through
                    // `replace_panels` so stale tasks are retired, and carries
                    // credentials across.
                    let result = write_servers(app, merged, tasks);
                    report_server_write(app, &result);
                    // Only on success, and in front of what the write said
                    // rather than over it. "Imported 3 hosts" used to be
                    // printed whether or not the file was written, and it also
                    // erased the warning naming an upgrade the import had just
                    // interrupted.
                    if result.is_ok() {
                        let reported = current_notice(app);
                        if let Some(manager) = app.password_manager.as_mut() {
                            let plural = if added == 1 { "host" } else { "hosts" };
                            manager.notice = Some(format!(
                                "Imported {added} {plural} from ~/.ssh/config; existing entries were left alone."
                            ));
                        }
                        prepend_notice(app, reported);
                    }
                }
                None => {
                    if let Some(manager) = app.password_manager.as_mut() {
                        manager.notice = Some("Could not read ~/.ssh/config.".to_string());
                    }
                }
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
            // A resume is for an upgrade that stopped for want of a password.
            // A host in the middle of one must not be restarted: the spawn
            // below replaces the panel's task and aborts what was there, every
            // child is `kill_on_drop`, and so saving a password would kill the
            // SSH session of a running `apt upgrade` -- interrupting a package
            // transaction on the real machine and leaving the remote lock file
            // behind. `execute_cmds` refuses to abort a running upgrade for
            // exactly this reason; this path disagreed with it.
            //
            // The condition is broad on purpose otherwise: `mode == Upgrade`
            // holds for the whole session once `u` has been pressed, which is
            // what makes "set the password, watch it resume" work at all.
            let already_running =
                app.panels[panel].upgrade_state == crate::panel::UpgradeState::STARTED;
            let should_resume = (resume_upgrade
                || app.panels[panel].mode == crate::app::Mode::Upgrade)
                && !already_running;
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
                // The same bookkeeping the confirmation modal does, through the
                // same method. This used to set only the global
                // `upgrade_started_at` and hand-roll the state write, so the
                // host's own `started_at` was never written and a resumed run
                // cut short was reported afterwards as the *previous* run's
                // outcome -- a success, or "never upgraded".
                app.mark_upgrades_started(&[panel]);
                // Into the ring, not `view`: this panel is in Upgrade mode and
                // the Upgrade pane is composed from the ring, so a line put in
                // `view` here would be one nothing draws.
                app.panels[panel]
                    .last_upgrade
                    .replace_with(std::iter::once(format!(
                        "{}\u{2192} Upgrade running...{}",
                        palette.meter_mid(),
                        palette.reset
                    )));
                let handle = crate::tasks::spawn_upgrade(
                    panel,
                    gen,
                    servers[panel].clone(),
                    Some(password),
                    tx.clone(),
                );
                tasks.set_upgrade(panel, handle);
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
            "  Could not store {} of them ({}); set those again with {}.",
            failed.len(),
            failed.join(", "),
            crate::consts::SETTINGS_KEY
        );
    }

    for p in &mut app.panels {
        p.note(note.clone());
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
