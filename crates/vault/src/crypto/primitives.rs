//! The primitives themselves: encryption, signing, and password wrapping.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use rand::{rng, Rng};
use zeroize::Zeroize;

use super::{Argon2Params, Ed25519PublicKey, Ed25519Signature, VaultKey, KEY_LEN, NONCE_LEN};

/// Bytes AES-GCM appends as its authentication tag. Fixed by the cipher.
const GCM_TAG_LEN: usize = 16;

/// Generate a random salt.
#[must_use]
pub fn generate_salt() -> [u8; 32] {
    let mut salt = [0u8; KEY_LEN];
    rng().fill_bytes(&mut salt);
    salt
}

/// Get current time in milliseconds.
#[must_use]
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, duration_millis_to_u64)
}

/// Convert a Duration to milliseconds as u64.
/// Fits for billions of years — well beyond any practical concern.
#[allow(clippy::cast_precision_loss)]
#[must_use]
fn duration_millis_to_u64(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Encrypt vault contents with AES-256-GCM (uses HKDF-derived encryption sub-key).
///
/// # Errors
/// Returns `VaultError::EncryptionFailed` if encryption fails.
pub fn encrypt_vault(
    key: &VaultKey,
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; 12]), crate::VaultError> {
    let mut enc_key = key.encryption_key();
    let cipher = Aes256Gcm::new((&enc_key).into());
    let mut nonce = [0u8; NONCE_LEN];
    rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(&nonce.into(), plaintext)
        .map_err(|_| crate::VaultError::EncryptionFailed)?;
    enc_key.zeroize();
    Ok((ciphertext, nonce))
}

/// Decrypt vault contents with AES-256-GCM (uses HKDF-derived encryption sub-key).
///
/// # Errors
/// Returns `VaultError::DecryptionFailed` if decryption fails.
pub fn decrypt_vault(
    key: &VaultKey,
    nonce: &[u8; 12],
    ciphertext: &[u8],
) -> Result<Vec<u8>, crate::VaultError> {
    let mut enc_key = key.encryption_key();
    let cipher = Aes256Gcm::new((&enc_key).into());
    let plaintext = cipher
        .decrypt(nonce.into(), ciphertext)
        .map_err(|_| crate::VaultError::DecryptionFailed)?;
    enc_key.zeroize();
    Ok(plaintext)
}

/// Sign vault data with Ed25519.
#[must_use]
pub fn sign_vault(key: &VaultKey, data: &[u8]) -> Ed25519Signature {
    let signing_key = key.derive_signing_key();
    let signature = signing_key.sign(data);
    // signing_key implements ZeroizeOnDrop, will be zeroized when dropped
    Ed25519Signature(signature.to_bytes())
}

/// Verify vault signature.
///
/// # Errors
/// Returns `VaultError::InvalidPublicKey` if the public key is invalid,
/// or `VaultError::SignatureVerificationFailed` if the signature doesn't match.
pub fn verify_vault_signature(
    pk: &Ed25519PublicKey,
    data: &[u8],
    sig: &Ed25519Signature,
) -> Result<(), crate::VaultError> {
    let verifying_key =
        VerifyingKey::from_bytes(&pk.0).map_err(|_| crate::VaultError::InvalidPublicKey)?;
    let signature = Signature::from_bytes(&sig.0);
    verifying_key
        .verify(data, &signature)
        .map_err(|_| crate::VaultError::SignatureVerificationFailed)
}

/// Wrap vault key with Argon2id(password).
///
/// # Errors
/// Returns `VaultError::Argon2Params` if parameters are invalid,
/// `VaultError::Argon2Error` if hashing fails, or `VaultError::EncryptionFailed`
/// if encryption fails.
pub fn wrap_argon2id(
    key: &VaultKey,
    password: &str,
    salt: &[u8; KEY_LEN],
    params: &Argon2Params,
) -> Result<Vec<u8>, crate::VaultError> {
    let argon2 = params.to_argon2()?;
    let mut wrapping_key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut wrapping_key)
        .map_err(|e| crate::VaultError::Argon2Error(e.to_string()))?;

    // Encrypt the RAW vault key (not the derived sub-key) with wrapping_key using AES-256-GCM
    let cipher = Aes256Gcm::new((&wrapping_key).into());
    let mut nonce = [0u8; NONCE_LEN];
    rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(&nonce.into(), key.as_bytes() as &[u8])
        .map_err(|_| crate::VaultError::EncryptionFailed)?;

    // Zeroize the wrapping key
    wrapping_key.zeroize();

    // Return: nonce(12) || ciphertext(32) || tag(16) = 60 bytes
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Unwrap vault key from Argon2id wrapped form.
///
/// # Errors
/// Returns `VaultError::InvalidWrapperData` if the wrapped data is malformed,
/// `VaultError::Argon2Params` if parameters are invalid,
/// `VaultError::Argon2Error` if hashing fails, or `VaultError::DecryptionFailed`
/// if decryption fails.
pub fn unwrap_argon2id(
    wrapped: &[u8],
    password: &str,
    salt: &[u8; KEY_LEN],
    params: &Argon2Params,
) -> Result<VaultKey, crate::VaultError> {
    // Nonce, then the wrapped key, then the tag that authenticates it.
    if wrapped.len() < NONCE_LEN + KEY_LEN + GCM_TAG_LEN {
        return Err(crate::VaultError::InvalidWrapperData("too short".into()));
    }

    let argon2 = params.to_argon2()?;
    let mut wrapping_key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut wrapping_key)
        .map_err(|e| crate::VaultError::Argon2Error(e.to_string()))?;

    let cipher = Aes256Gcm::new((&wrapping_key).into());
    let mut plaintext = cipher
        .decrypt(
            (&wrapped[..NONCE_LEN])
                .try_into()
                .map_err(|_| crate::VaultError::InvalidWrapperData("bad nonce length".into()))?,
            &wrapped[NONCE_LEN..],
        )
        .map_err(|_| crate::VaultError::DecryptionFailed)?;

    // Zeroize the wrapping key
    wrapping_key.zeroize();

    if plaintext.len() != KEY_LEN {
        plaintext.zeroize();
        return Err(crate::VaultError::InvalidWrapperData(
            "wrong key size".into(),
        ));
    }

    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&plaintext);
    plaintext.zeroize(); // Zeroize the decrypted plaintext

    Ok(VaultKey(key))
}
