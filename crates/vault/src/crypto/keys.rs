//! The vault key and the Ed25519 key material derived from it.
//!
//! Every one of these zeroizes on drop: they are the secrets everything else
//! in the crate exists to protect.

use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::{rng, Rng};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{KEY_LEN, SIGNATURE_LEN};

/// Vault encryption key (32 bytes = 256 bits)
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
// `pub(super)` rather than private: `primitives` unwraps a password into raw
// key bytes and has to hand them back as a `VaultKey`. Nothing outside
// `crypto` can see the array.
pub struct VaultKey(pub(super) [u8; KEY_LEN]);

impl VaultKey {
    /// Generate a new random vault key.
    ///
    /// # Panics
    /// Panics if the system RNG fails (extremely unlikely).
    #[must_use]
    pub fn new() -> Self {
        let mut key = [0u8; KEY_LEN];
        rng().fill_bytes(&mut key);
        Self(key)
    }

    /// Create from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Expose the raw bytes (use sparingly).
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive Ed25519 signing key from vault key via HKDF.
    ///
    /// # Panics
    /// Panics if HKDF expand fails (should never happen with SHA-256 and 32-byte output).
    #[must_use]
    pub fn derive_signing_key(&self) -> SigningKey {
        let hkdf = Hkdf::<Sha256>::new(None, &self.0);
        let mut okm = [0u8; KEY_LEN];
        // HKDF expand with SHA-256 and 32 bytes output should never fail
        // but we use expect for safety rather than changing the API
        #[allow(clippy::expect_used)]
        hkdf.expand(b"multitop-vault-signing", &mut okm)
            .expect("HKDF expand failed (should never happen with SHA-256)");
        SigningKey::from_bytes(&okm)
    }

    /// Derive Ed25519 verifying key from vault key.
    ///
    /// # Panics
    /// Panics if HKDF expand fails in `derive_signing_key`.
    #[must_use]
    pub fn derive_verifying_key(&self) -> VerifyingKey {
        self.derive_signing_key().verifying_key()
    }

    /// Derive AES-256-GCM encryption sub-key via HKDF (key separation from signing key).
    ///
    /// # Panics
    /// Panics if HKDF expand fails (should never happen with SHA-256 and 32-byte output).
    #[must_use]
    pub fn encryption_key(&self) -> [u8; KEY_LEN] {
        let hkdf = Hkdf::<Sha256>::new(None, &self.0);
        let mut okm = [0u8; KEY_LEN];
        // HKDF expand with SHA-256 and 32 bytes output should never fail
        #[allow(clippy::expect_used)]
        hkdf.expand(b"vault-aes-gcm-key", &mut okm)
            .expect("HKDF expand failed (should never happen with SHA-256)");
        okm
    }
}

impl Default for VaultKey {
    fn default() -> Self {
        Self::new()
    }
}

/// Ed25519 public key wrapper (32 bytes)
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Zeroize, ZeroizeOnDrop,
)]
pub struct Ed25519PublicKey(#[serde(with = "serde_bytes")] pub [u8; KEY_LEN]);

impl Ed25519PublicKey {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Ed25519 signature wrapper (64 bytes)
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Zeroize, ZeroizeOnDrop,
)]
pub struct Ed25519Signature(#[serde(with = "serde_bytes")] pub [u8; SIGNATURE_LEN]);

impl Ed25519Signature {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}
