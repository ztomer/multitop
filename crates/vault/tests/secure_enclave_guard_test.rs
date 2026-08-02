//! `use_os_keychain: false` must keep tests off *all* real credential storage,
//! including the Secure Enclave.
//!
//! The flag was honoured by the rollback counter and the lockout state but not
//! by the Secure Enclave path, which sits in the login keychain and is just as
//! real. Every test that initialised a vault on macOS therefore ran
//! `SecureEnclave::generate_new`, and that function begins by calling
//! `delete_existing`: running the suite deleted the developer's actual Secure
//! Enclave key and orphaned the wrapper inside their real vault. Because the
//! private key never leaves the enclave, that damage is permanent -- biometric
//! unlock stops working and cannot be recovered by re-running anything.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_vault::crypto::{Argon2Params, WrapperType};
use multitop_vault::format::VaultHeader;
use multitop_vault::{Vault, VaultConfig};

fn init_vault(dir: &std::path::Path, use_os_keychain: bool) -> VaultHeader {
    let vault_path = dir.join("vault.bin");
    let vault = Vault::new(VaultConfig {
        vault_path: vault_path.clone(),
        argon2_params: Some(Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        }),
        use_os_keychain,
    });
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize("master-pw"))
        .unwrap();
    VaultHeader::from_bytes(&std::fs::read(&vault_path).unwrap()).unwrap()
}

#[test]
fn initialising_without_keychain_permission_creates_no_secure_enclave_key() {
    let dir = std::env::temp_dir().join(format!("mt_se_guard_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let header = init_vault(&dir, false);
    assert!(
        !header.has_wrapper(WrapperType::SecureEnclave),
        "use_os_keychain=false must not produce a Secure Enclave wrapper: creating one \
         deletes and replaces the developer's real Secure Enclave key"
    );
    // The password wrapper is still there, so the vault is fully usable.
    assert!(
        header.has_wrapper(WrapperType::Argon2id),
        "the Argon2id wrapper must still be written"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
