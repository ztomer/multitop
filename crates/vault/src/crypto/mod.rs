//! Cryptographic primitives for the vault.
//!
//! Split by what each piece is: [`keys`] is the secret material, [`params`] is
//! how hard a password is to turn into a key, [`wrapper`] is the shape a
//! wrapped key takes on disk, and [`primitives`] is the encryption, signing
//! and wrapping themselves.

/// Bytes in a vault key, and in everything derived alongside it.
///
/// The vault key, each HKDF sub-key, the Ed25519 public key and the salt are
/// all this long by construction — AES-256 and Ed25519 both take 32. Written
/// out four times it would be four places to disagree.
pub const KEY_LEN: usize = 32;
/// Bytes in an AES-GCM nonce. Fixed by the cipher, not chosen.
pub const NONCE_LEN: usize = 12;
/// Bytes in an Ed25519 signature. Fixed by the scheme.
pub const SIGNATURE_LEN: usize = 64;

mod keys;
mod params;
mod primitives;
mod wrapper;

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod crypto_tests;

pub use keys::{Ed25519PublicKey, Ed25519Signature, VaultKey};
pub use params::{Argon2Params, MAX_M_KIB, MAX_P, MAX_T, MIN_M_KIB, MIN_P, MIN_T};
pub use primitives::{
    decrypt_vault, encrypt_vault, generate_salt, now_ms, sign_vault, unwrap_argon2id,
    verify_vault_signature, wrap_argon2id,
};
pub use wrapper::{Wrapper, WrapperType, MAX_WRAPPER_BYTES};
