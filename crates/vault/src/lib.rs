//! Encrypted sudo password vault with biometric unlock
//!
//! Provides secure storage for sudo passwords with biometric unlock
//! (Touch ID on macOS, fprintd on Linux) and password fallback.

#![allow(missing_docs)]
#![deny(unsafe_code)]

pub mod api;
pub mod crypto;
pub mod format;
pub mod fprintd;
pub mod lockout;
pub mod mlock;
pub mod rollback;
pub mod secure_enclave;

// Re-export public API
pub use api::{migrate_if_needed, UnlockResult, UnlockedVault, Vault};
pub use crypto::{
    decrypt_vault, encrypt_vault, generate_salt, now_ms, sign_vault, unwrap_argon2id,
    verify_vault_signature, wrap_argon2id, Argon2Params, Ed25519PublicKey, Ed25519Signature,
    VaultKey, Wrapper, WrapperType,
};
pub use format::{atomic_write_vault, read_vault_file, VaultFile, VaultHeader};
#[cfg(target_os = "linux")]
pub use fprintd::check_fprintd;
pub use fprintd::{FingerprintResult, FingerprintVerifier};
pub use secure_enclave::{get_secure_enclave, is_available, SecureEnclave};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use zeroize::Zeroize;

/// Vault configuration
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Path to vault file
    pub vault_path: PathBuf,
    /// Argon2id parameters (auto-detected if None)
    pub argon2_params: Option<Argon2Params>,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            vault_path: dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("multitop")
                .join("vault.bin"),
            argon2_params: None,
        }
    }
}

/// In-memory password store backed by `HashMap`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultContents {
    passwords: HashMap<String, String>,
    /// Canary string embedded in plaintext for integrity verification
    canary: Option<String>,
}

impl VaultContents {
    /// Get a password
    #[must_use]
    pub fn get(&self, host: &str) -> Option<SecretString> {
        self.passwords
            .get(host)
            .map(|s| SecretString::from(s.as_str()))
    }

    /// Set a password
    pub fn set(&mut self, host: String, password: &SecretString) {
        self.passwords
            .insert(host, password.expose_secret().to_string());
    }

    /// Remove a password
    pub fn remove(&mut self, host: &str) -> bool {
        self.passwords.remove(host).is_some()
    }

    /// List all hosts
    #[must_use]
    pub fn hosts(&self) -> Vec<String> {
        self.passwords.keys().cloned().collect()
    }

    /// Set the canary string (called during initialization)
    pub fn set_canary(&mut self, canary: String) {
        self.canary = Some(canary);
    }

    /// Verify canary string matches the header's canary
    #[must_use]
    pub fn verify_canary(&self, header_canary: &str) -> bool {
        self.canary
            .as_ref()
            .is_some_and(|plaintext_canary| plaintext_canary == header_canary)
    }
}

impl Drop for VaultContents {
    fn drop(&mut self) {
        // Zeroize all passwords before deallocation
        for (_, mut v) in self.passwords.drain() {
            v.zeroize();
        }
        // Zeroize canary if present
        if let Some(mut canary) = self.canary.take() {
            canary.zeroize();
        }
    }
}

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("Vault file not found: {0}")]
    NotFound(String),

    #[error("Vault already exists: {0}")]
    AlreadyExists(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Cryptographic error: {0}")]
    Crypto(String),

    #[error("Invalid vault format: {0}")]
    InvalidFormat(String),

    #[error("Unsupported vault version: {0}")]
    UnsupportedVersion(u8),

    #[error("Corrupted vault: {0}")]
    Corrupted(String),

    #[error("Biometric authentication failed or unavailable")]
    BiometricFailed,

    #[error("Platform not supported: {0}")]
    PlatformNotSupported(String),

    #[error("fprintd error: {0}")]
    FprintdError(String),

    #[error("Secure Enclave error: {0}")]
    SecureEnclaveError(String),

    #[error("Invalid wrapper data: {0}")]
    InvalidWrapperData(String),

    #[error("Invalid wrapper type: {0}")]
    InvalidWrapperType(u8),

    #[error("Wrapper data too large: {0} bytes")]
    WrapperTooLarge(usize),

    #[error("Too many wrappers (max 8)")]
    TooManyWrappers,

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Argon2 parameter error: {0}")]
    Argon2Params(String),

    #[error("Encryption failed")]
    EncryptionFailed,

    #[error("Decryption failed")]
    DecryptionFailed,

    #[error("Invalid public key")]
    InvalidPublicKey,

    #[error("Signature verification failed")]
    SignatureVerificationFailed,

    #[error("Argon2 error: {0}")]
    Argon2Error(String),

    #[error("Wrapper error: {0}")]
    WrapperError(String),

    #[error("Other error: {0}")]
    Other(String),

    #[error("Rate limited: {0} seconds remaining")]
    RateLimited(u64),

    #[error("Rollback detected: expected counter {expected}, got {actual}")]
    RollbackDetected { expected: u32, actual: u32 },
}
