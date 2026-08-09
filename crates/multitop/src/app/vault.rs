use crate::app::App;
use crate::app::{AppMode, VaultState};
impl App {
    /// Ask for the master password, if there is a locked vault to unlock.
    ///
    /// Returns whether the prompt went up. Bumping the epoch is the load-bearing
    /// part: it retires any attempt still in flight, so a stale answer arriving
    /// later cannot open a vault the user has moved on from.
    ///
    /// This is the password half. `begin_vault_unlock` is the door: it decides
    /// which of the two the user is asked for.
    pub fn begin_password_unlock(&mut self) -> bool {
        if self.vault.is_none() || !matches!(self.vault_state, VaultState::Locked) {
            return false;
        }
        self.bump_vault_epoch();
        self.prompt_for_master_password();
        true
    }

    /// Put the master password prompt up, wherever the unlock came from.
    ///
    /// One definition, because there are two ways in -- the user pressing `u`
    /// on a machine that cannot do biometrics, and a biometric attempt coming
    /// back refused -- and they must land in the same state. They were two
    /// copies of the same three lines.
    pub(crate) fn prompt_for_master_password(&mut self) {
        self.vault_password_input.clear();
        self.vault_state = VaultState::PasswordPrompt { error: None };
    }

    /// Start the unlock the user asked for: one touch if this vault can be
    /// opened that way on this machine, the master password if it cannot.
    ///
    /// Returns the vault and epoch when a biometric attempt should be spawned,
    /// so the caller -- which owns the terminal and the task runtime -- can
    /// start it. `None` means either there was nothing to unlock or the
    /// password prompt is now up; `show_vault_password_prompt` says which.
    ///
    /// The choice is made *before* anything is drawn. Putting up a biometric
    /// wait and discovering on failure that a password was always going to be
    /// needed is how this came to prompt twice for one unlock.
    pub fn begin_vault_unlock(&mut self) -> Option<(std::sync::Arc<multitop_vault::Vault>, u64)> {
        if !matches!(self.vault_state, VaultState::Locked) {
            return None;
        }
        let vault = self.vault.clone()?;
        if !vault.biometric_available() {
            self.begin_password_unlock();
            return None;
        }
        let epoch = self.bump_vault_epoch();
        self.vault_password_input.clear();
        self.vault_state = VaultState::Unlocking {
            awaiting_biometric: true,
        };
        Some((vault, epoch))
    }

    #[must_use]
    pub const fn show_upgrade_modal(&self) -> bool {
        matches!(self.mode, AppMode::ShowUpgradeModal)
    }

    pub fn set_show_upgrade_modal(&mut self, show: bool) {
        if show {
            self.mode = AppMode::ShowUpgradeModal;
        } else if matches!(self.mode, AppMode::ShowUpgradeModal) {
            self.mode = AppMode::Running;
        }
    }

    /// Check if vault password prompt should be shown.
    #[must_use]
    pub const fn show_vault_password_prompt(&self) -> bool {
        matches!(self.vault_state, VaultState::PasswordPrompt { .. })
    }

    pub fn set_show_vault_password_prompt(&mut self, show: bool) {
        match (show, &self.vault_state) {
            (true, VaultState::PasswordPrompt { .. }) => {}
            (true, _) => self.vault_state = VaultState::PasswordPrompt { error: None },
            (false, VaultState::PasswordPrompt { .. }) => self.vault_state = VaultState::Locked,
            (false, _) => {}
        }
    }

    #[must_use]
    pub const fn vault_verifying(&self) -> bool {
        matches!(
            self.vault_state,
            VaultState::Unlocking {
                awaiting_biometric: false
            }
        )
    }

    pub fn set_vault_unlocking(&mut self) -> u64 {
        let epoch = self.bump_vault_epoch();
        self.vault_state = VaultState::Unlocking {
            awaiting_biometric: false,
        };
        self.vault_password_input.clear();
        epoch
    }

    pub fn cancel_vault_biometric(&mut self) {
        if self.vault_awaiting_biometric() {
            self.bump_vault_epoch();
            self.vault_state = VaultState::Locked;
            self.vault_password_input.clear();
        }
    }

    pub fn bump_vault_epoch(&mut self) -> u64 {
        self.vault_epoch = self.vault_epoch.wrapping_add(1);
        self.vault_epoch
    }

    #[must_use]
    pub const fn vault_epoch_current(&self, epoch: u64) -> bool {
        self.vault_epoch == epoch
    }

    pub fn cancel_vault_verify(&mut self) {
        if self.vault_verifying() {
            self.bump_vault_epoch();
            self.vault_state = VaultState::Locked;
        }
    }

    pub fn cancel_vault_creation(&mut self) {
        if self.vault_creating() {
            self.bump_vault_epoch();
            self.vault_state = VaultState::Locked;
            self.vault_password_input.clear();
        }
    }

    #[must_use]
    pub const fn vault_creating(&self) -> bool {
        matches!(self.vault_state, VaultState::Creating { .. })
    }

    #[must_use]
    pub const fn vault_create_error(&self) -> Option<&String> {
        match &self.vault_state {
            VaultState::Creating { error, .. } => error.as_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub const fn vault_create_in_flight(&self) -> bool {
        matches!(
            self.vault_state,
            VaultState::Creating {
                in_flight: true,
                ..
            }
        )
    }

    pub fn begin_vault_create_attempt(&mut self) -> Option<String> {
        if self.vault_create_in_flight() {
            return None;
        }
        let master = std::mem::take(&mut self.vault_password_input);
        if master.is_empty() {
            self.vault_state = VaultState::Creating {
                error: Some("Master password cannot be empty".into()),
                in_flight: false,
            };
            return None;
        }
        self.vault_state = VaultState::Creating {
            error: None,
            in_flight: true,
        };
        Some(master)
    }

    pub fn fail_vault_creation(&mut self, error: String) {
        self.vault_state = VaultState::Creating {
            error: Some(error),
            in_flight: false,
        };
    }

    pub fn begin_vault_creation(&mut self) -> bool {
        if self.vault.is_some() || self.config_path.is_none() {
            return false;
        }
        self.bump_vault_epoch();
        self.vault_password_input.clear();
        self.vault_state = VaultState::Creating {
            error: None,
            in_flight: false,
        };
        true
    }

    pub(crate) fn report_rotation(&mut self, outcome: String) {
        if let Some(manager) = self.password_manager.as_mut() {
            manager.rotating = false;
            manager.notice = Some(outcome);
            return;
        }
        for p in &mut self.panels {
            p.note(outcome.clone());
        }
    }

    #[must_use]
    pub fn vault_path(&self) -> Option<std::path::PathBuf> {
        Some(self.config_path.as_ref()?.parent()?.join("vault.bin"))
    }

    pub(crate) fn seed_vault_from_panels(&mut self) {
        let known: Vec<(String, String)> = self
            .panels
            .iter()
            .filter_map(|p| {
                let pass = p.sudo_password.clone()?;
                Some((crate::password_store::account(&p.server), pass))
            })
            .collect();
        let mut failed = 0usize;
        if let VaultState::Unlocked { ref mut vault, .. } = &mut self.vault_state {
            for (key, pass) in known {
                if vault
                    .set_password(key, &secrecy::SecretString::from(pass))
                    .is_err()
                {
                    failed += 1;
                }
            }
        }
        // Say so rather than leaving the user believing the vault holds
        // passwords it never received.
        if failed > 0 {
            let note = format!(
                "\u{26a0} {failed} password(s) could not be written to the new vault; \
                 they remain in the OS credential store."
            );
            for p in &mut self.panels {
                p.note(note.clone());
            }
        }
    }

    #[must_use]
    pub const fn vault_awaiting_biometric(&self) -> bool {
        matches!(
            self.vault_state,
            VaultState::Unlocking {
                awaiting_biometric: true
            } | VaultState::Unlocked {
                awaiting_biometric: true,
                ..
            }
        )
    }

    #[must_use]
    pub fn vault_password_input(&self) -> &str {
        &self.vault_password_input
    }

    pub fn vault_password_input_mut(&mut self) -> &mut String {
        &mut self.vault_password_input
    }

    #[must_use]
    pub const fn vault_password_error(&self) -> Option<&String> {
        match &self.vault_state {
            VaultState::PasswordPrompt { error } => error.as_ref(),
            _ => None,
        }
    }

    pub fn set_vault_password_error(&mut self, err: Option<String>) {
        if let VaultState::PasswordPrompt { ref mut error } = &mut self.vault_state {
            *error = err;
        }
    }

    #[must_use]
    pub const fn vault_unlocked(&self) -> Option<&multitop_vault::UnlockedVault> {
        match &self.vault_state {
            VaultState::Unlocked { vault, .. } => Some(vault),
            _ => None,
        }
    }

    pub fn vault_unlocked_mut(&mut self) -> Option<&mut multitop_vault::UnlockedVault> {
        match &mut self.vault_state {
            VaultState::Unlocked { vault, .. } => Some(vault),
            _ => None,
        }
    }
}
