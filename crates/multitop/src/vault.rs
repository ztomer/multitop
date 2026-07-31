use multitop_vault::{UnlockedVault, Vault, VaultConfig};
use secrecy::ExposeSecret;

use crate::panel::Panel;
use crate::password_store;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultState {
    #[default]
    None,
    Locked,
    Unlocked,
}

pub fn create_vault(config_path: &std::path::Path) -> Option<Vault> {
    let vault_dir = config_path.parent()?;
    let vault_path = vault_dir.join("vault.bin");
    if !vault_path.exists() {
        return None;
    }
    Some(Vault::new(VaultConfig {
        vault_path,
        argon2_params: None,
    }))
}

pub fn try_load_vault_password(panel: &mut Panel, unlocked: &UnlockedVault) {
    if panel.sudo_password.is_some() {
        return;
    }
    let key = password_store::account(&panel.server);
    if let Some(pass) = unlocked.get_password(&key) {
        panel.sudo_password = Some(pass.expose_secret().to_string());
    }
}

pub fn host_key(panel: &Panel) -> String {
    password_store::account(&panel.server)
}
