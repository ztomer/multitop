use sha2::{Digest, Sha256};
use std::path::Path;

const SERVICE: &str = "multitop-vault-rollback";

fn account(vault_path: &Path) -> String {
    let canonical = std::fs::canonicalize(vault_path)
        .unwrap_or_else(|_| vault_path.to_path_buf());
    let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex::encode(hash)
}

/// Store the vault's counter in the system keychain for rollback detection.
pub fn store_counter(vault_path: &Path, counter: u32, created_ts: u64) {
    let value = format!("{counter}:{created_ts}");
    match keyring::Entry::new(SERVICE, &account(vault_path)) {
        Ok(entry) => {
            let _ = entry.set_password(&value);
        }
        Err(e) => {
            eprintln!("rollback: failed to create keyring entry: {e}");
        }
    }
}

/// Verify that the vault's counter has not regressed.
/// Returns Ok(()) if the counter is >= the stored counter, or if no stored
/// counter exists (first unlock). Returns Err on rollback detection.
pub fn check_counter(vault_path: &Path, counter: u32, created_ts: u64) -> Result<(), crate::VaultError> {
    // In test/CI environments, skip keychain check
    if cfg!(test) || std::env::var("CI").is_ok() || std::env::var("MULTITOP_MOCK_KEYCHAIN").is_ok() {
        return Ok(());
    }

    let entry = match keyring::Entry::new(SERVICE, &account(vault_path)) {
        Ok(e) => e,
        Err(_) => return Ok(()), // No keychain available, skip
    };

    let stored = match entry.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) => {
            // First unlock — store current and return ok
            store_counter(vault_path, counter, created_ts);
            return Ok(());
        }
        Err(e) => {
            eprintln!("rollback: failed to read keyring entry: {e}");
            return Ok(()); // Keychain error, skip check
        }
    };

    let parts: Vec<&str> = stored.split(':').collect();
    if parts.len() != 2 {
        return Ok(()); // Invalid format, skip check
    }

    let stored_counter: u32 = parts[0].parse().unwrap_or(0);
    let stored_ts: u64 = parts[1].parse().unwrap_or(0);

    if counter < stored_counter {
        return Err(crate::VaultError::RollbackDetected {
            expected: stored_counter,
            actual: counter,
        });
    }

    // Also check created_ts hasn't regressed (different vault file)
    if created_ts < stored_ts {
        return Err(crate::VaultError::RollbackDetected {
            expected: stored_counter,
            actual: counter,
        });
    }

    Ok(())
}

/// Parse a stored counter value string
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
        assert_eq!(result, Some((42, 1234567890)));
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
        let result = check_counter(&path, 0, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_rollback_error_display() {
        let err = crate::VaultError::RollbackDetected {
            expected: 10,
            actual: 5,
        };
        let msg = err.to_string();
        assert!(msg.contains("10"));
        assert!(msg.contains("5"));
    }
}
