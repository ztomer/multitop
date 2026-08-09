//! Tests for the lazily-loaded lockout state.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::api::*;
use crate::lockout::LockoutState;
use crate::{VaultConfig, VaultError};
use std::sync::Arc;
use tempfile::TempDir;

fn fast(path: std::path::PathBuf) -> VaultConfig {
    VaultConfig {
        vault_path: path,
        argon2_params: Some(crate::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        // Tests never touch the real login keychain.
        use_os_keychain: false,
    }
}

/// A lockout already on disk must be honoured by a freshly constructed
/// `Vault`, even though the state is now loaded lazily rather than in the
/// constructor.
#[tokio::test]
async fn a_persisted_lockout_is_honoured_after_lazy_load() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let vault = Vault::new(fast(path.clone()));
    vault.initialize("pw").await.unwrap();

    // Someone was locked out earlier in a previous run of the app.
    let mut state = LockoutState::load(&path, false);
    for _ in 0..12 {
        state.on_attempt(&path, crate::crypto::now_ms());
    }

    // A brand new Vault -- the constructor reads nothing.
    let fresh = Vault::new(fast(path.clone()));
    let err = fresh.unlock_with_password("pw").unwrap_err();
    assert!(
        matches!(err, VaultError::RateLimited(_)),
        "the persisted lockout must survive lazy loading, got {err:?}"
    );
}

/// Concurrent first-use must not let anyone through unlimited.
///
/// The lazy load used to set its "loaded" flag before actually loading, so
/// a second caller arriving in that window checked the limiter against a
/// default (empty) state and was never rate limited at all.
#[tokio::test]
async fn concurrent_first_use_cannot_bypass_the_limiter() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.bin");
    let vault = Vault::new(fast(path.clone()));
    vault.initialize("pw").await.unwrap();

    let mut state = LockoutState::load(&path, false);
    for _ in 0..12 {
        state.on_attempt(&path, crate::crypto::now_ms());
    }

    // Several threads race into the very first use of one Vault.
    let shared = Arc::new(Vault::new(fast(path.clone())));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let v = Arc::clone(&shared);
        handles.push(std::thread::spawn(move || {
            matches!(
                v.unlock_with_password("pw"),
                Err(VaultError::RateLimited(_))
            )
        }));
    }
    let limited: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(
        limited.iter().all(|b| *b),
        "every racing caller must see the lockout, got {limited:?}"
    );
}
