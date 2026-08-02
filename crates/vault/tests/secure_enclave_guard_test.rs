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

/// The Linux fprintd prompt is gated on a TPM2 wrapper being present, because
/// fprintd returns only a yes or a no and holds no key material -- a verified
/// fingerprint releases nothing unless a TPM2 wrapper is there to be unwrapped.
/// Nothing in this codebase creates one, which is what keeps that prompt from
/// firing. Pinned here so that adding TPM2 *creation* without also adding TPM2
/// *unwrapping* fails a test, rather than silently reintroducing a thirty-second
/// fingerprint prompt that cannot succeed.
#[test]
fn no_vault_is_created_with_a_tpm2_wrapper() {
    let dir = std::env::temp_dir().join(format!("mt_tpm2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let header = init_vault(&dir, false);
    assert!(
        !header.has_wrapper(WrapperType::Tpm2),
        "a TPM2 wrapper now exists, so the fprintd path is live -- TPM2 unwrapping \
         must be implemented before that prompt can succeed"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
