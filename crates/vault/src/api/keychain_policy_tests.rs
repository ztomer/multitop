//! Tests for the keychain-use policy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::api::*;
use crate::VaultConfig;

fn cfg(use_os_keychain: bool) -> VaultConfig {
    VaultConfig {
        vault_path: std::path::PathBuf::from("/tmp/multitop-policy/vault.bin"),
        argon2_params: None,
        use_os_keychain,
    }
}

/// The limiter must carry the vault's keychain policy from the moment the
/// vault exists, not from the first time someone happens to load it.
///
/// `LockoutState::default()` cannot know the answer -- `use_keychain` is
/// `#[serde(skip)]`, so it defaults to false. Building a production vault
/// on that made it claim, briefly, that it must not persist to the
/// keychain. Nothing exploited the window today because
/// `unlock_with_password` loads first, but the value was wrong and a new
/// caller would have inherited it.
#[test]
fn a_vault_carries_its_keychain_policy_from_construction() {
    let real = Vault::new(cfg(true));
    assert!(
        real.lockout.lock().unwrap().use_keychain,
        "a production vault must intend to persist the limiter to the keychain \
         before anything is loaded"
    );

    let isolated = Vault::new(cfg(false));
    assert!(
        !isolated.lockout.lock().unwrap().use_keychain,
        "and a vault told not to must never intend to"
    );
}

/// `VaultConfig::default()` is the one place a caller can avoid stating the
/// policy. It answers `true` on purpose: a test that slips through gets the
/// real keychain and is noticed immediately, whereas `false` would let
/// production quietly stop persisting the limiter.
#[test]
fn the_config_default_fails_loudly_rather_than_silently() {
    // keychain-safe: reads a field off a default config. Nothing is
    // constructed from it and nothing is opened.
    assert!(
        VaultConfig::default().use_os_keychain,
        "the default must be the fail-loud direction"
    );
}
