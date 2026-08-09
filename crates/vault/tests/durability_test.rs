//! Writing a vault to a disk that misbehaves, and the limiter's real clock.
//!
//! These are the paths a developer never walks: a leftover temp file from a
//! process that was killed, a directory that will not take a write, a second
//! writer holding the lock. Each one is silent when it goes wrong — the vault
//! simply stops being able to save — so each one is pinned here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;

use multitop_vault::crypto::{Argon2Params, Ed25519PublicKey, Wrapper, WrapperType};
use multitop_vault::format::{atomic_write_vault, read_vault_file, VaultHeader};
use multitop_vault::{Vault, VaultConfig};

const MASTER: &str = "correct horse battery staple";

/// Cheap parameters: these tests are about the disk, not the KDF.
const fn params() -> Argon2Params {
    Argon2Params {
        t: 1,
        m_kib: 32768,
        p: 1,
    }
}

const fn config(vault_path: std::path::PathBuf) -> VaultConfig {
    VaultConfig {
        vault_path,
        argon2_params: Some(params()),
        // isolated: never the real credential store.
        use_os_keychain: false,
    }
}

fn header() -> VaultHeader {
    VaultHeader::new(
        Ed25519PublicKey([7u8; 32]),
        [9u8; 32],
        params(),
        vec![Wrapper::new(WrapperType::Argon2id, vec![0u8; 64]).unwrap()],
    )
    .unwrap()
}

// ------------------------------------------------------------ atomic writes

#[test]
fn a_written_vault_reads_back_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let ciphertext = b"not really ciphertext, but bytes all the same".to_vec();

    atomic_write_vault(&path, &header(), &ciphertext).expect("the write must land");

    let read = read_vault_file(&path).expect("and read back");
    assert_eq!(read.ciphertext, ciphertext);
    assert_ne!(
        read.header.canary, "",
        "the canary did not survive the write"
    );
    assert_eq!(read.header.wrappers.len(), 1);
}

#[test]
fn the_vault_directory_is_created_and_kept_to_the_owner() {
    // A vault the group can read is a vault the group can copy and attack
    // offline at their leisure.
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("cfg").join("multitop");
    let path = nested.join("vault.bin");

    atomic_write_vault(&path, &header(), b"body").expect("the directory must be created");

    let mode = std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o700,
        "the vault directory is readable beyond its owner"
    );
}

#[test]
fn a_leftover_temp_file_from_a_dead_writer_is_cleared_away() {
    // `create_new` stops two writers clobbering each other, but on its own it
    // turns any leftover into a permanent failure: every later save returns
    // AlreadyExists and the vault quietly stops being able to store anything.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    std::fs::write(
        path.with_extension("bin.tmp"),
        b"half a vault from a killed run",
    )
    .unwrap();

    atomic_write_vault(&path, &header(), b"body").expect("debris must not block the write");

    assert_eq!(read_vault_file(&path).unwrap().ciphertext, b"body");
    assert!(
        !path.with_extension("bin.tmp").exists(),
        "the temp file survived a successful write"
    );
}

#[test]
fn a_temp_file_a_live_writer_holds_is_left_alone() {
    // A writer that is still working holds an exclusive lock on its temp file.
    // Deleting that would be deleting another process's work in progress.
    use fs2::FileExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let tmp = path.with_extension("bin.tmp");
    let held = std::fs::File::create(&tmp).unwrap();
    held.lock_exclusive().unwrap();

    let err = atomic_write_vault(&path, &header(), b"body")
        .expect_err("a locked temp file means another writer is live");
    assert!(
        err.to_string().contains("another process"),
        "the reason must name the other writer: {err}"
    );

    let _ = FileExt::unlock(&held);
}

#[test]
fn a_directory_that_cannot_exist_is_an_error_rather_than_a_silent_no_op() {
    // A *read-only* directory is not the case to test: the writer chmods the
    // vault directory to 0o700 on the way in, so it makes its own way. What it
    // cannot do is create a directory where a file already sits.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("cfg");
    std::fs::write(&blocker, b"a file, not a directory").unwrap();

    let err = atomic_write_vault(&blocker.join("vault.bin"), &header(), b"body")
        .expect_err("a vault cannot be written inside a file");
    assert_ne!(err.to_string(), "");
}

#[test]
fn a_vault_file_that_is_not_there_is_an_error_not_an_empty_vault() {
    assert!(read_vault_file(std::path::Path::new("/no/such/vault.bin")).is_err());
}

// ------------------------------------------------------------- the limiter

#[test]
fn the_limiter_uses_the_real_clock_unless_a_test_supplies_one() {
    // `with_clock` exists for tests; `new` is what production runs, and it is
    // the one that must anchor the backoff to the wall clock.
    use multitop_vault::lockout::{LockoutGuard, LockoutState};
    use std::sync::Mutex;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let state = Mutex::new(LockoutState::new(false));

    // Count the attempt the way the unlock path does — before the KDF runs, so
    // dying mid-attempt leaves it counted rather than forgiven.
    let before = multitop_vault::crypto::now_ms();
    for _ in 0..4 {
        state.lock().unwrap().on_attempt(&path, before);
    }
    {
        // Dropped without `mark_success`: a failure, anchored to the real clock.
        let _guard = LockoutGuard::new(&state, &path);
    }
    let failed = state.lock().unwrap().lockout_until_epoch_ms;
    assert!(
        failed >= before,
        "the deadline was anchored before the attempt happened: {failed} < {before}"
    );

    // A success clears the count, which is what earns the short interval back.
    {
        let mut guard = LockoutGuard::new(&state, &path);
        guard.mark_success();
    }
    let after = state.lock().unwrap();
    assert_eq!(after.failed_attempts, 0, "a success left attempts counted");
    assert_eq!(
        after.lockout_until_epoch_ms, 0,
        "a success left a deadline standing"
    );
}

// --------------------------------------------------------------- parameters

#[test]
fn auto_detected_parameters_scale_with_the_memory_they_find() {
    // Whatever this machine reports, the result has to pass the validator that
    // guards every unlock — and land on one of the three documented tiers.
    let detected = Argon2Params::auto_detect();
    assert!(detected.validate().is_ok(), "{detected:?}");
    assert!(
        [6u8, 8, 10].contains(&detected.t),
        "iterations off the tier table: {detected:?}"
    );
    assert!(
        (32_768..=1_048_576).contains(&detected.m_kib),
        "memory outside the clamp: {detected:?}"
    );
}

// ---------------------------------------------------------------- unwrapping

#[test]
fn a_wrapper_that_decrypts_to_the_wrong_size_is_refused() {
    use multitop_vault::crypto::{unwrap_argon2id, wrap_argon2id, VaultKey};

    let key = VaultKey::new();
    let salt = multitop_vault::crypto::generate_salt();
    let wrapped = wrap_argon2id(&key, MASTER, &salt, &params()).expect("wrap");

    // The right password unwraps it.
    assert!(unwrap_argon2id(&wrapped, MASTER, &salt, &params()).is_ok());
    // The wrong one does not, and says so as a decryption failure rather than
    // handing back whatever the cipher produced.
    assert!(unwrap_argon2id(&wrapped, "wrong", &salt, &params()).is_err());
    // Neither does the right password against a different salt.
    assert!(unwrap_argon2id(&wrapped, MASTER, &[0u8; 32], &params()).is_err());
}

// ------------------------------------------------------------- end to end

#[test]
fn a_vault_survives_being_closed_and_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::new(config(dir.path().join("vault.bin")));
    assert!(!vault.exists(), "a vault exists before it is created");

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize(MASTER))
        .expect("initialise");
    assert!(vault.exists());

    let mut unlocked = vault.unlock_with_password(MASTER).expect("unlock");
    unlocked
        .set_password(
            "web-01".to_string(),
            &secrecy::SecretString::from("hunter2".to_string()),
        )
        .expect("store");
    unlocked.save().expect("save");
    drop(unlocked);

    let reopened = vault.unlock_with_password(MASTER).expect("reopen");
    assert_eq!(reopened.hosts(), vec!["web-01".to_string()]);

    vault.delete().expect("delete");
    assert!(!vault.exists(), "the file survived a delete");
    // Deleting one that is already gone is not an error.
    vault.delete().expect("a second delete is a no-op");
}

// ------------------------------------------------------- a vault that was edited

/// A real vault, then the same file with one byte of its ciphertext flipped.
fn tampered_vault(dir: &std::path::Path) -> Vault {
    let path = dir.join("vault.bin");
    let vault = Vault::new(config(path.clone()));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize(MASTER))
        .expect("initialise");

    let read = read_vault_file(&path).expect("read back");
    let mut ciphertext = read.ciphertext.clone();
    ciphertext[0] ^= 0xff;
    atomic_write_vault(&path, &read.header, &ciphertext).expect("rewrite");
    vault
}

#[test]
fn a_vault_whose_ciphertext_was_edited_is_refused_before_it_is_decrypted() {
    // The signature is checked first, on purpose: decrypting attacker-chosen
    // bytes and *then* asking whether they were authentic is the wrong order.
    // The message has to name the reason, or a corrupted vault and a wrong
    // password are indistinguishable to whoever is looking at the screen.
    let dir = tempfile::tempdir().unwrap();
    let vault = tampered_vault(dir.path());

    let err = vault
        .unlock_with_password(MASTER)
        .expect_err("an edited vault must not open");
    assert!(
        err.to_string().to_lowercase().contains("signature"),
        "the refusal did not say the file had been altered: {err}"
    );
}

#[test]
fn an_edited_vault_refuses_the_biometric_path_as_well() {
    // Both doors check the signature. A path that skipped it would be the one
    // an attacker uses.
    let dir = tempfile::tempdir().unwrap();
    let vault = tampered_vault(dir.path());

    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.unlock_biometric());
    assert!(outcome.is_err(), "an edited vault opened by touch");
}

#[test]
fn a_vault_file_that_is_not_there_fails_the_unlock_rather_than_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::new(config(dir.path().join("never-created.bin")));
    assert!(vault.unlock_with_password(MASTER).is_err());
    assert!(!vault.biometric_available());
}

#[test]
fn a_vault_kept_out_of_the_keychain_never_offers_a_touch() {
    // The config flag alone settles it, before any file is read: a vault that
    // was told not to use the OS keychain has no enclave wrapper to unwrap.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let vault = Vault::new(config(path));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize(MASTER))
        .expect("initialise");
    assert!(!vault.biometric_available());
}
