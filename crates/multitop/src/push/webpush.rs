//! RFC 8291 Web Push message encryption (`aes128gcm`, RFC 8188), the body a
//! `UnifiedPush` endpoint carries to the phone.
//!
//! The distributor (ntfy) and its server see only ciphertext: the key is agreed
//! between this server's one-shot P-256 key and the key the app registered
//! (`p256dh`), salted by the app's `auth` secret. The app's connector library
//! decrypts it; nothing in between can read which host went down.
//!
//! [`seal`] is the whole derivation from the ECDH secret on, pure, and pinned to
//! the RFC's Appendix A vector byte for byte. [`encrypt`] adds the two random
//! inputs (the ephemeral key and the salt) and is the only part that is not.

use ring::{aead, agreement, hkdf, rand};

/// The record size the header declares. One record holds any alert: the
/// payload is a few hundred bytes, far under it.
pub const RECORD_SIZE: u32 = 4096;
/// The salt RFC 8188 requires, in bytes.
pub const SALT_LEN: usize = 16;
/// An uncompressed P-256 point: `0x04 || X || Y`.
pub const P256_POINT_LEN: usize = 65;
/// The `auth` secret the user agent issues, in bytes (RFC 8291 section 3.2).
pub const AUTH_LEN: usize = 16;
/// The padding delimiter of the last (and only) record (RFC 8188 section 2).
const LAST_RECORD_DELIMITER: u8 = 0x02;
const IKM_LEN: usize = 32;
const CEK_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// The largest plaintext one record carries: the record less the delimiter
/// octet and the GCM tag.
pub const MAX_PLAINTEXT: usize = RECORD_SIZE as usize - 1 - aead::MAX_TAG_LEN;

/// Why a body could not be sealed. Says which input was wrong, never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// The registered public key is not an uncompressed P-256 point.
    BadPublicKey,
    /// The registered auth secret is not 16 bytes.
    BadAuth,
    /// The payload does not fit one record.
    TooLarge,
    /// The platform's RNG or the ECDH agreement failed.
    Crypto,
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadPublicKey => "the registered p256dh key is not a P-256 point",
            Self::BadAuth => "the registered auth secret is not 16 bytes",
            Self::TooLarge => "the alert does not fit one push record",
            Self::Crypto => "the push encryption failed",
        })
    }
}

/// `ring`'s HKDF output length, as the `KeyType` it asks for.
struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand<const N: usize>(prk: &hkdf::Prk, info: &[u8]) -> Result<[u8; N], SealError> {
    let mut out = [0u8; N];
    prk.expand(&[info], Len(N))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| SealError::Crypto)?;
    Ok(out)
}

/// The full `aes128gcm` body from the ECDH secret on: header, then the one
/// encrypted record.
///
/// # Errors
///
/// A key of the wrong shape, or a payload larger than one record.
pub fn seal(
    ecdh_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    auth: &[u8],
    salt: &[u8; SALT_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, SealError> {
    if ua_public.len() != P256_POINT_LEN || as_public.len() != P256_POINT_LEN {
        return Err(SealError::BadPublicKey);
    }
    if auth.len() != AUTH_LEN {
        return Err(SealError::BadAuth);
    }
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(SealError::TooLarge);
    }
    // RFC 8291 section 3.4: combine the ECDH secret with the auth secret.
    let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, auth).extract(ecdh_secret);
    let mut key_info = Vec::with_capacity(b"WebPush: info\0".len() + 2 * P256_POINT_LEN);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let ikm: [u8; IKM_LEN] = expand(&prk_key, &key_info)?;
    // RFC 8188 section 2.2/2.3: the content key and nonce from the salt.
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
    let cek: [u8; CEK_LEN] = expand(&prk, b"Content-Encoding: aes128gcm\0")?;
    let nonce: [u8; NONCE_LEN] = expand(&prk, b"Content-Encoding: nonce\0")?;

    let key = aead::UnboundKey::new(&aead::AES_128_GCM, &cek)
        .map(aead::LessSafeKey::new)
        .map_err(|_| SealError::Crypto)?;
    let mut record = Vec::with_capacity(plaintext.len() + 1 + aead::MAX_TAG_LEN);
    record.extend_from_slice(plaintext);
    record.push(LAST_RECORD_DELIMITER);
    // One record, sequence number 0: the nonce is used as derived.
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::empty(),
        &mut record,
    )
    .map_err(|_| SealError::Crypto)?;

    let mut body = Vec::with_capacity(SALT_LEN + 4 + 1 + P256_POINT_LEN + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(u8::try_from(P256_POINT_LEN).map_err(|_| SealError::Crypto)?);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// [`seal`] with a fresh ephemeral key and salt: the body for one delivery.
///
/// # Errors
///
/// A registration of the wrong shape, an oversized payload, or the RNG failing.
pub fn encrypt(ua_public: &[u8], auth: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
    let rng = rand::SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    rand::SecureRandom::fill(&rng, &mut salt).map_err(|_| SealError::Crypto)?;
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| SealError::Crypto)?;
    let as_public = private
        .compute_public_key()
        .map_err(|_| SealError::Crypto)?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public);
    let ecdh = agreement::agree_ephemeral(private, &peer, <[u8]>::to_vec)
        .map_err(|_| SealError::BadPublicKey)?;
    seal(&ecdh, ua_public, as_public.as_ref(), auth, &salt, plaintext)
}

#[cfg(test)]
#[path = "webpush_tests.rs"]
mod tests;
