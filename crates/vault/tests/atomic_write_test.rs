//! A half-written temp file must never become permanent damage.
//!
//! `atomic_write_vault` creates `vault.bin.tmp` with `create_new`, which refuses
//! to overwrite an existing one. That is the right choice against two concurrent
//! writers, but on its own it turns any leftover temp file into a permanent
//! failure: every later save returns `AlreadyExists`, so the vault quietly stops
//! being able to store anything and nothing points at the cause. A process
//! killed between creating the temp file and the rename leaves exactly that --
//! and so does a panic, because the release profile aborts and runs no
//! destructors.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_vault::crypto::Argon2Params;
use multitop_vault::{Vault, VaultConfig};

fn vault_at(dir: &std::path::Path) -> (Vault, std::path::PathBuf) {
    let vault_path = dir.join("vault.bin");
    let vault = Vault::new(VaultConfig {
        vault_path: vault_path.clone(),
        argon2_params: Some(Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        }),
        use_os_keychain: false,
    });
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize("master-pw"))
        .unwrap();
    (vault, vault_path)
}

#[test]
fn a_stale_temp_file_does_not_block_saving_forever() {
    let dir = std::env::temp_dir().join(format!("mt_atomic_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (vault, vault_path) = vault_at(&dir);

    // Debris from a writer that was killed before it could rename.
    let tmp = vault_path.with_extension("bin.tmp");
    std::fs::write(&tmp, b"leftover from a killed process").unwrap();

    let mut unlocked = vault.unlock_with_password("master-pw").unwrap();
    unlocked
        .set_password(
            "ztomer@host:22".to_string(),
            &secrecy::SecretString::from("sudo-pw".to_string()),
        )
        .expect("a leftover temp file must not make the vault unable to save");

    // And the value really landed.
    let reopened = vault.unlock_with_password("master-pw").unwrap();
    let got = reopened.get_password("ztomer@host:22").unwrap();
    assert_eq!(secrecy::ExposeSecret::expose_secret(&got), "sudo-pw");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_successful_save_leaves_no_temp_file_behind() {
    let dir = std::env::temp_dir().join(format!("mt_atomic2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (vault, vault_path) = vault_at(&dir);

    let mut unlocked = vault.unlock_with_password("master-pw").unwrap();
    unlocked
        .set_password(
            "a@b:22".to_string(),
            &secrecy::SecretString::from("x".to_string()),
        )
        .unwrap();

    assert!(
        !vault_path.with_extension("bin.tmp").exists(),
        "the temp file must be gone once it has been renamed into place"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
