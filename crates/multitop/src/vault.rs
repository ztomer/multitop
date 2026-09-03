use multitop_vault::{UnlockedVault, Vault, VaultConfig};
use secrecy::ExposeSecret;

use crate::panel::Panel;

/// Argon2id settings for a vault a test throws away: the cheapest the crypto
/// layer will accept, so a test vault costs milliseconds rather than a quarter
/// of system RAM.
const TEST_ARGON2_PASSES: u8 = 1;
const TEST_ARGON2_MEMORY_MIB: u32 = 32;
const TEST_ARGON2_LANES: u8 = 1;
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
        // rejected outright rather than being quietly weakened. Through
        // `from_config` so the clamps that guard those bounds are the ones that
        // ran, rather than a struct literal that skips them.
        argon2_params: mocked.then(|| {
            multitop_vault::crypto::Argon2Params::from_config(
                TEST_ARGON2_PASSES,
                TEST_ARGON2_MEMORY_MIB,
                TEST_ARGON2_LANES,
            )
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

/// Give a panel the password the vault holds for its host, if it has none of
/// its own.
///
/// A session password the user just typed is left alone: it is the newer of
/// the two and the one they expect to be used.
///
/// This was written once here and once inline in `App::load_known_passwords`,
/// and the two had already drifted — only the inline copy marked the password
/// as `external_password`, which is what the Upgrade view reads to say where a
/// credential came from. The copy production ran was the one no test touched.
/// One implementation now, and it is this one.
pub fn try_load_vault_password(panel: &mut Panel, unlocked: &UnlockedVault) {
    if panel.sudo_password.is_some() {
        return;
    }
    let key = password_store::account(&panel.server);
    if let Some(pass) = unlocked.get_password(&key) {
        // Through the setter: it is the one place that knows a vault password
        // also has to be marked as coming from outside this session, which is
        // what the Upgrade view reads to say where a credential came from.
        panel.set_sudo_password(pass.expose_secret().to_string(), true);
        return;
    }
    // Fallback to OS keychain when vault doesn't have it — migration for
    // hosts added before the vault existed, or vaults created empty.
    // The vault is the source of truth to *report*, but a run that needs a
    // password and finds none in the vault must still try the keychain.
    if let Ok(Some(pass)) = password_store::load(&panel.server) {
        panel.set_sudo_password(pass, true);
    }
}
