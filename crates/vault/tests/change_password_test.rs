//! Rotating the master password must retire the old one without risking the vault.
//!
//! The previous implementation called `secure_overwrite` on the vault file and
//! then wrote the replacement. Anything failing between those two steps -- a
//! full disk, a crash, a power cut -- left the vault filled with random bytes
//! and no new file written, so every stored password was gone with nothing to
//! restore from.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

// Integration-test crate: helper fns outside #[test] are not covered by
// clippy.toml's test exemption, so the restriction lints are expected here.

use multitop_vault::crypto::{Argon2Params, WrapperType};
use multitop_vault::format::VaultHeader;
use multitop_vault::{Vault, VaultConfig, VaultError};
use secrecy::{ExposeSecret, SecretString};

const HOST: &str = "ztomer@192.168.0.33:22";

/// A fresh scratch path for the vault; the helpers here report through
/// `Result` and the `#[test]` callers unwrap (a helper is outside clippy's
/// test exemption).
fn scratch(tag: &str) -> std::io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("mt_chpw_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("vault.bin"))
}

fn vault_at(path: &std::path::Path) -> Vault {
    Vault::new(VaultConfig {
        vault_path: path.to_path_buf(),
        argon2_params: Some(Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        }),
        use_os_keychain: false,
    })
}

fn seeded(path: &std::path::Path) -> Result<Vault, VaultError> {
    let vault = vault_at(path);
    vault.initialize("old-master")?;
    let mut unlocked = vault.unlock_with_password("old-master")?;
    unlocked.set_password(
        HOST.to_string(),
        &SecretString::from("sudo-secret".to_string()),
    )?;
    unlocked.lock();
    Ok(vault)
}

#[test]
fn the_new_password_works_and_the_old_one_stops() {
    let path = scratch("swap").expect("a scratch dir");
    let vault = seeded(&path).expect("a seeded vault");

    vault.change_password("old-master", "new-master").unwrap();

    let opened = vault
        .unlock_with_password("new-master")
        .expect("the new password must open the vault");
    assert_eq!(
        opened
            .get_password(HOST)
            .map(|p| p.expose_secret().to_string()),
        Some("sudo-secret".to_string()),
        "the stored passwords must survive the rotation"
    );
    drop(opened);

    assert!(
        vault.unlock_with_password("old-master").is_err(),
        "the retired password must no longer open the vault"
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_wrong_current_password_leaves_the_vault_untouched() {
    let path = scratch("wrong").expect("a scratch dir");
    let vault = seeded(&path).expect("a seeded vault");
    let before = std::fs::read(&path).unwrap();

    let err = vault
        .change_password("not-the-master", "new-master")
        .unwrap_err();
    assert!(
        !matches!(err, VaultError::Io(_)),
        "a wrong password must fail authentication, not I/O: {err:?}"
    );

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused rotation must not have written to the vault at all"
    );
    // And it still opens with the original password.
    assert!(vault.unlock_with_password("old-master").is_ok());

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_vault_file_is_never_left_unreadable() {
    let path = scratch("intact").expect("a scratch dir");
    let vault = seeded(&path).expect("a seeded vault");

    vault.change_password("old-master", "new-master").unwrap();

    // Parsing the header is what a shredded file would fail: the old code
    // overwrote the file with random bytes before writing the replacement.
    let bytes = std::fs::read(&path).unwrap();
    let header = VaultHeader::from_bytes(&bytes)
        .expect("the vault must remain a parseable vault after rotation");
    assert!(
        header.has_wrapper(WrapperType::Argon2id),
        "the password wrapper must be present"
    );
    assert!(
        bytes.len() > 8,
        "a vault of {} bytes is not a vault",
        bytes.len()
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn rotation_advances_the_counter_so_a_restored_old_vault_is_detectable() {
    let path = scratch("counter").expect("a scratch dir");
    let vault = seeded(&path).expect("a seeded vault");
    let before = VaultHeader::from_bytes(&std::fs::read(&path).unwrap())
        .unwrap()
        .counter;

    vault.change_password("old-master", "new-master").unwrap();

    let after = VaultHeader::from_bytes(&std::fs::read(&path).unwrap())
        .unwrap()
        .counter;
    assert!(
        after > before,
        "the counter did not advance ({before} -> {after}), so a pre-rotation \
         copy restored from a backup would not be recognised as a rollback"
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
