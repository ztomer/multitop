use sha2::{Digest, Sha256};
use std::path::Path;

const SERVICE: &str = "multitop-vault-rollback";

fn account(vault_path: &Path) -> String {
    let canonical = std::fs::canonicalize(vault_path).unwrap_or_else(|_| vault_path.to_path_buf());
    let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex::encode(hash)
}

/// Store the vault's counter in the system keychain for rollback detection.
///
/// # Accepted limitation
///
/// A failed write is swallowed, and the consequence is asymmetric so it is
/// worth being explicit: the stored counter can only fall BEHIND the vault, so
/// a missed write makes `check_counter` more permissive (it compares
/// `vault >= stored`), never falsely accusing a legitimate vault of being
/// rolled back. Failing the save outright would block the user from writing
/// their own vault over a transient keychain error, which is the worse trade
/// for a single-user tool. It does mean rollback protection degrades silently
/// if the keychain is persistently unwritable.
pub fn store_counter(vault_path: &Path, counter: u32, created_ts: u64, use_keychain: bool) {
    if !use_keychain {
        return;
    }
    store_counter_in(&KeychainAnchor, vault_path, counter, created_ts);
}

/// What reading the anchor produced.
///
/// `Absent` and `Unavailable` must stay distinct. An absent anchor is a first
/// unlock and should be written; an unavailable one is a transient read failure,
/// and writing then could overwrite a good anchor with a lower counter, which
/// would weaken detection rather than degrade it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorRead {
    Value(String),
    Absent,
    Unavailable,
}

/// Where the rollback anchor is kept.
///
/// The anchor only works if it lives somewhere that restoring an old vault file
/// does not also restore, which is why production uses the OS credential store.
/// This trait exists so the comparison logic can be tested at all: the
/// keychain-backed store cannot run in the suite -- it would write real items
/// and put an authorization dialog in front of whoever is at the keyboard -- so
/// before this, every branch of rollback detection was unreachable from any
/// test, in a security control.
pub trait AnchorStore {
    fn read(&self, account: &str) -> AnchorRead;
    fn write(&self, account: &str, value: &str);
}

/// The real anchor: the OS credential store.
pub struct KeychainAnchor;

impl AnchorStore for KeychainAnchor {
    fn read(&self, account: &str) -> AnchorRead {
        let Ok(entry) = keyring::Entry::new(SERVICE, account) else {
            return AnchorRead::Unavailable;
        };
        match entry.get_password() {
            Ok(v) => AnchorRead::Value(v),
            Err(keyring::Error::NoEntry) => AnchorRead::Absent,
            Err(_) => AnchorRead::Unavailable,
        }
    }

    fn write(&self, account: &str, value: &str) {
        if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
            let _ = entry.set_password(value);
        }
    }
}

/// An in-memory anchor, for tests only.
///
/// Not a security boundary and never used in production: it exists so the
/// detection logic below can be exercised without touching the real keychain.
#[derive(Default)]
pub struct MemoryAnchor(std::sync::Mutex<std::collections::HashMap<String, String>>);

impl MemoryAnchor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl AnchorStore for MemoryAnchor {
    fn read(&self, account: &str) -> AnchorRead {
        self.0.lock().map_or(AnchorRead::Unavailable, |g| {
            g.get(account)
                .map_or(AnchorRead::Absent, |v| AnchorRead::Value(v.clone()))
        })
    }

    fn write(&self, account: &str, value: &str) {
        if let Ok(mut g) = self.0.lock() {
            g.insert(account.to_string(), value.to_string());
        }
    }
}

/// An anchor that is never readable and never writable, for tests that need to
/// assert the unavailable path.
pub struct UnavailableAnchor;

impl AnchorStore for UnavailableAnchor {
    fn read(&self, _account: &str) -> AnchorRead {
        AnchorRead::Unavailable
    }
    fn write(&self, _account: &str, _value: &str) {}
}

/// Write the anchor to an arbitrary store.
pub fn store_counter_in(store: &dyn AnchorStore, vault_path: &Path, counter: u32, created_ts: u64) {
    store.write(&account(vault_path), &format!("{counter}:{created_ts}"));
}

/// Verify that the vault's counter has not regressed.
/// Returns Ok(()) if the counter is >= the stored counter, or if no stored
/// counter exists (first unlock). Returns Err on rollback detection.
///
/// # Errors
/// Returns `VaultError::RollbackDetected` if the counter or timestamp has regressed.
pub fn check_counter(
    vault_path: &Path,
    counter: u32,
    created_ts: u64,
    use_keychain: bool,
) -> Result<(), crate::VaultError> {
    // The caller decides, and the same value drives `store_counter`, so the
    // read and the write cannot disagree.
    if !use_keychain {
        return Ok(());
    }
    check_counter_in(&KeychainAnchor, vault_path, counter, created_ts)
}

/// Verify the counter against an arbitrary anchor store.
///
/// # Errors
/// Returns `VaultError::RollbackDetected` if the counter or timestamp has regressed.
pub fn check_counter_in(
    store: &dyn AnchorStore,
    vault_path: &Path,
    counter: u32,
    created_ts: u64,
) -> Result<(), crate::VaultError> {
    let acct = account(vault_path);

    let stored = match store.read(&acct) {
        AnchorRead::Value(v) => v,
        AnchorRead::Absent => {
            // First unlock: adopt the current values as the baseline.
            store.write(&acct, &format!("{counter}:{created_ts}"));
            return Ok(());
        }
        // Deliberately no write here: overwriting a good anchor with whatever
        // this vault claims, on the strength of a read that just failed, would
        // weaken detection instead of degrading it.
        AnchorRead::Unavailable => return Ok(()),
    };

    // Parsed by the shared parser rather than re-implemented. The inline copy
    // that used to live here used `unwrap_or(0)`, so a corrupt anchor such as
    // "abc:123" became counter 0 and quietly passed the comparison below --
    // while `parse_stored_counter`, which rejects exactly that, sat beside it
    // fully tested and called by nothing outside its own tests.
    let Some((stored_counter, stored_ts)) = parse_stored_counter(&stored) else {
        return Ok(()); // Unparseable anchor, skip check
    };

    // Also checks created_ts, which catches a different vault file whose
    // counter happens not to have regressed.
    if counter < stored_counter || created_ts < stored_ts {
        return Err(crate::VaultError::RollbackDetected {
            expected: stored_counter,
            actual: counter,
        });
    }

    Ok(())
}

/// Parse a stored counter value string
#[must_use]
pub fn parse_stored_counter(stored: &str) -> Option<(u32, u64)> {
    let parts: Vec<&str> = stored.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let counter = parts[0].parse().ok()?;
    let ts = parts[1].parse().ok()?;
    Some((counter, ts))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn test_account_deterministic() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        std::fs::write(&path, b"test").unwrap();

        let acc1 = account(&path);
        let acc2 = account(&path);
        assert_eq!(acc1, acc2);
        // Should be a 64-char hex string (SHA256)
        assert_eq!(acc1.len(), 64);
        assert!(acc1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_parse_stored_counter_valid() {
        let result = parse_stored_counter("42:1234567890");
        assert_eq!(result, Some((42, 1_234_567_890)));
    }

    #[test]
    fn test_parse_stored_counter_invalid_format() {
        assert_eq!(parse_stored_counter("invalid"), None);
        assert_eq!(parse_stored_counter("42"), None);
        assert_eq!(parse_stored_counter("42:123:456"), None);
    }

    #[test]
    fn test_parse_stored_counter_invalid_numbers() {
        assert_eq!(parse_stored_counter("abc:123"), None);
        assert_eq!(parse_stored_counter("42:xyz"), None);
    }

    #[test]
    fn test_parse_stored_counter_zero() {
        let result = parse_stored_counter("0:0");
        assert_eq!(result, Some((0, 0)));
    }

    #[test]
    fn test_check_counter_skipped_in_test() {
        // In test mode, check_counter always returns Ok
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        std::fs::write(&path, b"test").unwrap();

        // Even with a lower counter, should pass in test mode
        let result = check_counter(&path, 0, 0, false);
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Detection logic
    //
    // None of this could be tested before: `check_counter` went straight to the
    // keychain, and the suite runs with the keychain disabled, so every branch
    // below -- including the one that actually refuses a rolled-back vault --
    // was unreachable from any test in a security control.
    // -----------------------------------------------------------------------

    fn vault_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("vault.bin");
        std::fs::write(&path, b"test").unwrap();
        (dir, path)
    }

    #[test]
    fn a_regressed_counter_is_refused() {
        let (_d, path) = vault_file();
        let store = MemoryAnchor::new();
        store_counter_in(&store, &path, 10, 1_000);

        let err = check_counter_in(&store, &path, 5, 1_000).unwrap_err();
        assert!(
            matches!(
                err,
                crate::VaultError::RollbackDetected {
                    expected: 10,
                    actual: 5
                }
            ),
            "restoring an older vault must be refused, got {err:?}"
        );
    }

    #[test]
    fn a_regressed_timestamp_is_refused_even_when_the_counter_is_not() {
        let (_d, path) = vault_file();
        let store = MemoryAnchor::new();
        store_counter_in(&store, &path, 10, 9_000);

        // A different vault file whose counter happens to be high enough.
        assert!(check_counter_in(&store, &path, 11, 1_000).is_err());
    }

    #[test]
    fn the_same_or_a_newer_vault_is_accepted() {
        let (_d, path) = vault_file();
        let store = MemoryAnchor::new();
        store_counter_in(&store, &path, 10, 1_000);

        assert!(check_counter_in(&store, &path, 10, 1_000).is_ok(), "same");
        assert!(check_counter_in(&store, &path, 11, 1_001).is_ok(), "newer");
    }

    #[test]
    fn the_first_unlock_adopts_the_current_values() {
        let (_d, path) = vault_file();
        let store = MemoryAnchor::new();

        assert!(check_counter_in(&store, &path, 7, 500).is_ok());
        // Having adopted 7, going backwards is now refused.
        assert!(check_counter_in(&store, &path, 6, 500).is_err());
    }

    #[test]
    fn a_corrupt_anchor_skips_the_check_rather_than_half_doing_it() {
        let (_d, path) = vault_file();
        let store = MemoryAnchor::new();
        store.write(&account(&path), "abc:123");

        // The old inline parser turned "abc" into 0 and compared against it.
        assert!(
            check_counter_in(&store, &path, 5, 0).is_ok(),
            "an unparseable anchor must skip the check, not compare against zero"
        );
    }

    #[test]
    fn an_unavailable_anchor_skips_the_check_and_writes_nothing() {
        let (_d, path) = vault_file();
        assert!(check_counter_in(&UnavailableAnchor, &path, 1, 1).is_ok());

        // And a transient read failure must not clobber a good anchor: the
        // memory store still holds the higher counter afterwards.
        let store = MemoryAnchor::new();
        store_counter_in(&store, &path, 10, 1_000);
        let _ = check_counter_in(&UnavailableAnchor, &path, 1, 1);
        assert_eq!(
            store.read(&account(&path)),
            AnchorRead::Value("10:1000".to_string())
        );
    }

    #[test]
    fn test_rollback_error_display() {
        let err = crate::VaultError::RollbackDetected {
            expected: 10,
            actual: 5,
        };
        let msg = err.to_string();
        assert!(msg.contains("10"));
        assert!(msg.contains('5'));
    }
}
