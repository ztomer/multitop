//! Vault file format (binary serialization).
//!
//! Split by what a caller is doing with it: [`header`] is the part that
//! describes the vault, [`file`] is the header plus its ciphertext, and
//! [`write`] is how one reaches the disk intact.

mod file;
mod header;
mod write;

#[cfg(test)]
#[path = "format_tests.rs"]
mod format_tests;

pub use file::{read_vault_file, VaultFile};
pub use header::VaultHeader;
pub use write::atomic_write_vault;
