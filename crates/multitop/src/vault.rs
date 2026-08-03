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

/// The settings every vault this process opens or creates is built with.
///
/// One place, because the two call sites disagreeing is a way to open a vault
/// that cannot be unlocked. Both knobs follow `password_store::is_mock_enabled`:
/// a test that has diverted the credential store must not have the vault reach
/// the real keychain behind its back for lockout and rollback state, and must
/// not pay Argon2id at a quarter of system RAM to make a throwaway vault.
#[must_use]
pub fn config_for(vault_path: std::path::PathBuf) -> VaultConfig {
    let mocked = password_store::is_mock_enabled();
    VaultConfig {
        vault_path,
        // 32 MiB is the floor the crypto layer accepts; anything lower is
        // rejected outright rather than being quietly weakened.
        argon2_params: mocked.then_some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Real runs use the OS keychain for lockout and rollback state.
        use_os_keychain: !mocked,
    }
}

#[must_use]
pub fn create_vault(config_path: &std::path::Path) -> Option<Vault> {
    let vault_dir = config_path.parent()?;
    let vault_path = vault_dir.join("vault.bin");
    if !vault_path.exists() {
        return None;
    }
    Some(Vault::new(config_for(vault_path)))
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

#[must_use]
pub fn host_key(panel: &Panel) -> String {
    password_store::account(&panel.server)
}
