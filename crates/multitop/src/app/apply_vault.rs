//! The vault half of `App::apply`: what the UI does when the vault task
//! answers. Split from `apply.rs` for the 500-line cap; the dispatch stays
//! there, these are the arms it reaches.

use crate::app::App;
use crate::app::{AppMode, VaultState};
use secrecy::ExposeSecret;
use std::sync::Arc;

impl App {
    pub(super) fn on_vault_created(
        &mut self,
        epoch: u64,
        unlocked: Box<multitop_vault::UnlockedVault>,
    ) -> bool {
        if !self.vault_epoch_current(epoch) {
            return false;
        }
        if let Some(ref path) = self.config_path {
            self.vault = crate::vault::create_vault(path).map(Arc::new);
        }
        self.vault_state = VaultState::Unlocked {
            vault: unlocked,
            awaiting_biometric: false,
        };
        self.seed_vault_from_panels();
        self.vault_password_input.clear();
        let note = "Vault created. Sudo passwords are now stored encrypted; \
             unlock with Touch ID.";
        // Said where the user is looking. Creating a vault starts in
        // Server Settings, whose panel covers the whole screen, so a
        // line appended to the panels behind it is a line nobody reads.
        if let Some(manager) = self.password_manager.as_mut() {
            manager.notice = Some(note.to_string());
        }
        for p in &mut self.panels {
            p.note(note.to_string());
        }
        true
    }

    pub(super) fn on_vault_unlock_failed(&mut self, epoch: u64, error: String) -> bool {
        if !self.vault_epoch_current(epoch) {
            return false;
        }
        // Back to the prompt with the reason, rather than silently
        // dropping the user somewhere with no explanation.
        self.vault_state = VaultState::PasswordPrompt { error: Some(error) };
        true
    }

    pub(super) fn on_vault_create_failed(&mut self, epoch: u64, error: String) -> bool {
        // Also refuses to reopen the prompt over a vault that exists:
        // the failing attempt may be a duplicate of one that already
        // succeeded, and reporting it would take a working vault back
        // off the user.
        if !self.vault_epoch_current(epoch) || self.vault.is_some() {
            return false;
        }
        self.fail_vault_creation(error);
        true
    }

    pub(super) fn on_vault_unlocked(
        &mut self,
        epoch: u64,
        unlocked: Box<multitop_vault::UnlockedVault>,
    ) -> bool {
        if !self.vault_epoch_current(epoch) {
            return false;
        }
        self.vault_state = VaultState::Unlocked {
            vault: unlocked,
            awaiting_biometric: false,
        };
        // Reload vault passwords into panels now that the vault is
        // open — otherwise the upgrade view still says "will prompt"
        // for hosts whose passwords are already in the vault.
        // Collect first to avoid borrowing `self` mutably and immutably
        // at once.
        let to_load: Vec<(String, String)> =
            if let VaultState::Unlocked { vault, .. } = &self.vault_state {
                vault
                    .hosts()
                    .into_iter()
                    .filter_map(|host| {
                        let pass = vault.get_password(&host)?;
                        Some((host, pass.expose_secret().to_string()))
                    })
                    .collect()
            } else {
                Vec::new()
            };
        for p in &mut self.panels {
            let key = crate::password_store::account(&p.server);
            if p.sudo_password.is_some() {
                continue;
            }
            if let Some((_, pass)) = to_load.iter().find(|(k, _)| k == &key) {
                p.set_sudo_password(pass.clone(), true);
            } else if let Ok(Some(pass)) = crate::password_store::load(&p.server) {
                p.set_sudo_password(pass, true);
            }
        }
        self.mode = AppMode::ShowUpgradeModal;
        true
    }

    pub(super) fn on_vault_password_rotated(&mut self, epoch: u64) -> bool {
        if !self.vault_epoch_current(epoch) {
            return false;
        }
        // The vault key is unchanged by a rotation, so an unlocked
        // handle stays valid and any Secure Enclave wrapper still
        // decrypts. Only the password that unwraps it has moved.
        self.report_rotation("Master password changed.".to_string());
        true
    }

    pub(super) fn on_vault_password_rotation_failed(&mut self, epoch: u64, error: &str) -> bool {
        if !self.vault_epoch_current(epoch) {
            return false;
        }
        // Said plainly, because the common cause is a mistyped current
        // password and the useful fact is that nothing changed.
        self.report_rotation(format!("Master password NOT changed: {error}"));
        true
    }

    pub(super) fn on_vault_biometric_failed(&mut self, epoch: u64) -> bool {
        if !self.vault_epoch_current(epoch) {
            return false;
        }
        // Biometrics refused or cancelled: fall back to the master
        // password. `Unlocking { awaiting_biometric: false }` would be a
        // dead end -- no prompt, no modal, nothing for the user to do.
        // Through the same function the direct route uses, so the two
        // ways into this prompt cannot land in different states.
        self.prompt_for_master_password();
        true
    }
}
