//! Tests for the vault API.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::api::*;
use crate::crypto::Argon2Params;
use crate::{VaultConfig, VaultError};
use secrecy::ExposeSecret;
use secrecy::SecretString;
use tempfile::TempDir;

// A controllable clock for lockout tests. Thread-local because the test
// harness gives each test its own thread, so tests cannot disturb one
// another's time even when run in parallel.
thread_local! {
    static TEST_CLOCK_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn test_clock() -> u64 {
    TEST_CLOCK_MS.with(std::cell::Cell::get)
}

fn set_clock(ms: u64) {
    TEST_CLOCK_MS.with(|c| c.set(ms));
}

fn advance_clock(ms: u64) {
    TEST_CLOCK_MS.with(|c| c.set(c.get() + ms));
}

fn fast_vault_config(path: std::path::PathBuf) -> VaultConfig {
    VaultConfig {
        vault_path: path,
        argon2_params: Some(Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    }
}

#[tokio::test]
async fn test_vault_init_unlock() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    let password = "test-sudo-password-123";
    vault.initialize(password).await.unwrap();
    assert!(vault.exists());

    let mut unlocked = vault.unlock_with_password(password).unwrap();

    unlocked
        .set_password("server1:22".into(), &SecretString::from("pass123"))
        .unwrap();
    assert_eq!(
        unlocked.get_password("server1:22").unwrap().expose_secret(),
        "pass123"
    );

    unlocked.lock();
    let unlocked2 = vault.unlock_with_password(password).unwrap();
    assert_eq!(
        unlocked2
            .get_password("server1:22")
            .unwrap()
            .expose_secret(),
        "pass123"
    );
}

#[tokio::test]
async fn test_vault_change_password() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    let old_pass = "old-password";
    let new_pass = "new-password-456";

    vault.initialize(old_pass).await.unwrap();
    vault.change_password(old_pass, new_pass).unwrap();

    assert!(vault.unlock_with_password(old_pass).is_err());

    let unlocked = vault.unlock_with_password(new_pass).unwrap();
    assert!(unlocked.get_password("server1:22").is_none());
}

#[tokio::test]
async fn test_rate_limiting_lockout() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());

    // Drive time explicitly. A real attempt costs an Argon2 KDF plus a
    // keychain write — together often more than the 1s first backoff tier —
    // so a wall-clock version of this test races its own setup cost and
    // flaps between "rate limited" and "window already expired".
    let vault = Vault::with_clock(config, test_clock);
    set_clock(1_000_000);

    let password = "correct-password";
    vault.initialize(password).await.unwrap();

    for _ in 0..3 {
        assert!(vault.unlock_with_password("wrong").is_err());
    }

    // The third failure earns a 1s backoff; a retry inside it is refused.
    assert!(matches!(
        vault.unlock_with_password("wrong"),
        Err(VaultError::RateLimited(_))
    ));

    // Past the backoff window, the correct password works again.
    advance_clock(2000);
    let unlocked = vault.unlock_with_password(password).unwrap();
    assert!(unlocked.get_password("test").is_none());

    // A success resets the counter, so the next failure is a plain
    // wrong-password error rather than another rate limit.
    let result = vault.unlock_with_password("wrong");
    assert!(result.is_err());
    assert!(!matches!(result, Err(VaultError::RateLimited(_))));
}

#[tokio::test]
async fn test_vault_exists_and_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    assert!(!vault.exists());
    assert_eq!(vault.path(), &path);

    vault.initialize("password").await.unwrap();
    assert!(vault.exists());
}

#[tokio::test]
async fn test_vault_initialize_already_exists() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();
    let result = vault.initialize("password").await;
    assert!(result.is_err());
    assert!(matches!(result, Err(VaultError::AlreadyExists(_))));
}

#[tokio::test]
async fn test_vault_unlock_wrong_password() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("correct-password").await.unwrap();
    let result = vault.unlock_with_password("wrong-password");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_vault_unlock_biometric_fallback() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();

    // Biometric will fail, should fall back to password prompt
    // Since we can't mock stdin, this will fail with IO error
    let result = vault.unlock_biometric().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_vault_unlock_biometric_no_fallback() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();

    // Biometric will fail, no fallback
    let result = vault.unlock_biometric().await;
    assert!(result.is_err());
    assert!(matches!(result, Err(VaultError::BiometricFailed)));
}

/// `Vault` must not grow a second home for an unlocked vault.
///
/// It had one: an `Option<UnlockedVault>` cache that nothing ever wrote to.
/// `get_unlocked` therefore always missed and always did a fresh biometric
/// unlock despite its name, and `Vault::lock` -- documented as "clear
/// memory" -- took from a field that was permanently `None`, so it was a
/// no-op that read as a security control. Its test asserted nothing and
/// passed; `get_unlocked`'s asserted the miss.
///
/// The unlocked vault belongs to the caller (multitop holds it in
/// `VaultState::Unlocked`). Two owners of one key, with independent
/// lifetimes, is the state this asserts cannot come back.
#[test]
fn the_vault_holds_no_second_copy_of_an_unlocked_one() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let vault = Vault::new(fast_vault_config(path));
    // Every field, spelled out: a cache added here would have to be added
    // to this list too, which is the point at which someone asks why.
    let Vault {
        config: _,
        lockout: _,
        lockout_init: _,
        lockout_loaded: _,
        clock: _,
    } = vault;
}

#[tokio::test]
async fn test_vault_delete() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();
    assert!(vault.exists());

    vault.delete().unwrap();
    assert!(!vault.exists());
}

#[tokio::test]
async fn test_vault_delete_nonexistent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.bin");
    let config = fast_vault_config(path);
    let vault = Vault::new(config);

    // Should not error
    vault.delete().unwrap();
}

#[tokio::test]
async fn test_unlocked_vault_remove_password() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();
    let mut unlocked = vault.unlock_with_password("password").unwrap();

    unlocked
        .set_password("server1:22".into(), &SecretString::from("pass1"))
        .unwrap();
    assert!(unlocked.get_password("server1:22").is_some());

    let removed = unlocked.remove_password("server1:22").unwrap();
    assert!(removed);
    assert!(unlocked.get_password("server1:22").is_none());
}

#[tokio::test]
async fn test_unlocked_vault_remove_nonexistent_password() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();
    let mut unlocked = vault.unlock_with_password("password").unwrap();

    let removed = unlocked.remove_password("nonexistent").unwrap();
    assert!(!removed);
}

#[tokio::test]
async fn test_unlocked_vault_hosts() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();
    let mut unlocked = vault.unlock_with_password("password").unwrap();

    assert_eq!(unlocked.hosts(), [] as [String; 0]);

    unlocked
        .set_password("server1:22".into(), &SecretString::from("pass1"))
        .unwrap();
    unlocked
        .set_password("server2:22".into(), &SecretString::from("pass2"))
        .unwrap();

    let mut hosts = unlocked.hosts();
    hosts.sort();
    assert_eq!(hosts, vec!["server1:22", "server2:22"]);
}

#[tokio::test]
async fn test_unlocked_vault_persists_after_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();

    {
        let mut unlocked = vault.unlock_with_password("password").unwrap();
        unlocked
            .set_password("server1:22".into(), &SecretString::from("pass1"))
            .unwrap();
    }

    // Unlock again and check password persisted
    let unlocked = vault.unlock_with_password("password").unwrap();
    assert_eq!(
        unlocked.get_password("server1:22").unwrap().expose_secret(),
        "pass1"
    );
}

#[tokio::test]
async fn test_vault_multiple_servers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Vault::new(config);

    vault.initialize("password").await.unwrap();
    let mut unlocked = vault.unlock_with_password("password").unwrap();

    // Add multiple server passwords
    for i in 0..10 {
        let host = format!("server{i}:22");
        let pass = format!("pass{i}");
        unlocked
            .set_password(host, &SecretString::from(pass.as_str()))
            .unwrap();
    }

    // Verify all passwords
    for i in 0..10 {
        let host = format!("server{i}:22");
        let pass = format!("pass{i}");
        assert_eq!(unlocked.get_password(&host).unwrap().expose_secret(), pass);
    }

    assert_eq!(unlocked.hosts().len(), 10);
}

#[tokio::test]
async fn test_vault_concurrent_access() {
    use std::sync::Arc;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_vault.bin");
    let config = fast_vault_config(path.clone());
    let vault = Arc::new(Vault::new(config));

    vault.initialize("password").await.unwrap();

    let mut handles = vec![];
    for i in 0..5 {
        let vault = vault.clone();
        handles.push(tokio::spawn(async move {
            let mut unlocked = vault.unlock_with_password("password").unwrap();
            let host = format!("server{i}:22");
            let pass = format!("pass{i}");
            unlocked
                .set_password(host, &SecretString::from(pass.as_str()))
                .unwrap();
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    // Verify all passwords were saved (last writer wins for each host)
    let unlocked = vault.unlock_with_password("password").unwrap();
    assert_eq!(unlocked.hosts().len(), 5);
}
