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
