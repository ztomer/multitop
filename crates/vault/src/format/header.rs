//! The vault header: what a reader needs before it can decrypt anything.
//!
//! Every bound in here is read back off a file another process, an older
//! build, or a damaged disk may have written, so each one fails as an error
//! rather than as a panic or an over-large allocation.

use crate::now_ms;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};

pub(super) const CURRENT_VERSION: u8 = 2;

/// How many ways a vault may be openable at once. The count is one byte on
/// disk and every wrapper is another copy of the key, so the cap is both a
/// format limit and a blast-radius one.
pub const MAX_WRAPPERS: usize = 8;
/// Bytes in the canary — the known plaintext that says a decryption used the
/// right key rather than producing plausible rubbish.
const CANARY_LEN: usize = 16;
/// Bytes in the little-endian creation timestamp.
const TIMESTAMP_LEN: usize = 8;

/// Vault file header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultHeader {
    pub magic: [u8; 4],
    pub version: u8,
    pub key_version: u8,
    pub created_timestamp_ms: u64,
    pub counter: u32,
    pub salt: [u8; 32],
    pub argon2_params: crate::crypto::Argon2Params,
    pub wrappers: Vec<crate::crypto::Wrapper>,
    pub nonce: [u8; 12],
    pub ed25519_pk: crate::crypto::Ed25519PublicKey,
    pub signature: crate::crypto::Ed25519Signature,
    pub canary: String,
}

impl VaultHeader {
    /// Generate a random canary string
    #[must_use]
    pub fn generate_canary() -> String {
        let mut canary_bytes = [0u8; CANARY_LEN];
        rand::rng().fill_bytes(&mut canary_bytes);
        format!("multitop-vault-canary-{}", hex::encode(&canary_bytes[..16]))
    }

    /// Create a new vault header.
    ///
    /// # Errors
    /// Returns `VaultError::TooManyWrappers` if more than 8 wrappers are provided,
    /// or `VaultError::WrapperTooLarge` if any wrapper data exceeds 65,535 bytes.
    pub fn new(
        ed25519_pk: crate::crypto::Ed25519PublicKey,
        salt: [u8; 32],
        argon2_params: crate::crypto::Argon2Params,
        wrappers: Vec<crate::crypto::Wrapper>,
    ) -> Result<Self, crate::VaultError> {
        if wrappers.len() > MAX_WRAPPERS {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrappers
            .iter()
            .any(|w| w.data.len() > crate::crypto::MAX_WRAPPER_BYTES)
        {
            return Err(crate::VaultError::WrapperTooLarge(65535));
        }

        let canary = Self::generate_canary();

        Ok(Self {
            magic: *b"MQV2",
            version: CURRENT_VERSION,
            key_version: 0,
            created_timestamp_ms: now_ms(),
            counter: 0,
            salt,
            argon2_params,
            wrappers,
            nonce: [0u8; crate::crypto::NONCE_LEN],
            ed25519_pk,
            signature: crate::crypto::Ed25519Signature([0u8; crate::crypto::SIGNATURE_LEN]),
            canary,
        })
    }

    /// Create a new vault header with a pre-generated canary.
    ///
    /// # Errors
    /// Returns `VaultError::TooManyWrappers` if more than 8 wrappers are provided,
    /// or `VaultError::WrapperTooLarge` if any wrapper data exceeds 65,535 bytes.
    pub fn new_with_canary(
        ed25519_pk: crate::crypto::Ed25519PublicKey,
        salt: [u8; 32],
        argon2_params: crate::crypto::Argon2Params,
        wrappers: Vec<crate::crypto::Wrapper>,
        canary: String,
    ) -> Result<Self, crate::VaultError> {
        if wrappers.len() > MAX_WRAPPERS {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrappers
            .iter()
            .any(|w| w.data.len() > crate::crypto::MAX_WRAPPER_BYTES)
        {
            return Err(crate::VaultError::WrapperTooLarge(65535));
        }

        Ok(Self {
            magic: *b"MQV2",
            version: CURRENT_VERSION,
            key_version: 0,
            created_timestamp_ms: now_ms(),
            counter: 0,
            salt,
            argon2_params,
            wrappers,
            nonce: [0u8; crate::crypto::NONCE_LEN],
            ed25519_pk,
            signature: crate::crypto::Ed25519Signature([0u8; crate::crypto::SIGNATURE_LEN]),
            canary,
        })
    }

    #[must_use]
    pub fn get_wrapper(
        &self,
        wrapper_type: crate::crypto::WrapperType,
    ) -> Option<&crate::crypto::Wrapper> {
        self.wrappers
            .iter()
            .find(|w| w.wrapper_type == wrapper_type)
    }

    /// Add a wrapper to the header.
    ///
    /// # Errors
    /// Returns `VaultError::TooManyWrappers` if more than 8 wrappers would exist,
    /// or `VaultError::WrapperTooLarge` if the wrapper data exceeds 65,535 bytes.
    pub fn add_wrapper(
        &mut self,
        wrapper: crate::crypto::Wrapper,
    ) -> Result<(), crate::VaultError> {
        if self.wrappers.len() >= MAX_WRAPPERS
            && !self
                .wrappers
                .iter()
                .any(|w| w.wrapper_type == wrapper.wrapper_type)
        {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrapper.data.len() > crate::crypto::MAX_WRAPPER_BYTES {
            return Err(crate::VaultError::WrapperTooLarge(65535));
        }
        self.wrappers
            .retain(|w| w.wrapper_type != wrapper.wrapper_type);
        self.wrappers.push(wrapper);
        Ok(())
    }

    /// Replace an existing wrapper or add a new one.
    ///
    /// # Errors
    /// Returns `VaultError::WrapperTooLarge` if the wrapper data exceeds 65,535 bytes.
    pub fn replace_wrapper(
        &mut self,
        wrapper: crate::crypto::Wrapper,
    ) -> Result<(), crate::VaultError> {
        if wrapper.data.len() > crate::crypto::MAX_WRAPPER_BYTES {
            return Err(crate::VaultError::WrapperTooLarge(65535));
        }
        self.wrappers
            .retain(|w| w.wrapper_type != wrapper.wrapper_type);
        self.wrappers.push(wrapper);
        Ok(())
    }

    #[must_use]
    pub fn has_wrapper(&self, wrapper_type: crate::crypto::WrapperType) -> bool {
        self.get_wrapper(wrapper_type).is_some()
    }

    /// Data that gets signed (header without signature + ciphertext).
    #[must_use]
    pub fn signed_data(&self, ciphertext: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        // Writing to a Vec<u8> never fails
        self.write_header_without_sig(&mut data);
        data.extend_from_slice(ciphertext);
        data
    }

    /// Write header without signature to buffer.
    ///
    /// # Errors
    /// Returns `std::io::Error` if writing to the buffer fails (never for Vec<u8>).
    #[allow(clippy::unnecessary_wraps)]
    fn write_header_without_sig(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.magic);
        buf.push(self.version);
        buf.push(self.key_version);
        buf.extend_from_slice(&self.created_timestamp_ms.to_le_bytes());
        buf.extend_from_slice(&self.counter.to_le_bytes());
        buf.extend_from_slice(&self.salt);
        buf.push(self.argon2_params.t);
        buf.extend_from_slice(&self.argon2_params.m_kib.to_le_bytes());
        buf.push(self.argon2_params.p);
        // wrappers.len() <= 8 (enforced by add_wrapper/replace_wrapper)
        #[allow(clippy::cast_possible_truncation)]
        buf.push(self.wrappers.len() as u8);
        for w in &self.wrappers {
            buf.push(w.wrapper_type as u8);
            // w.data.len() <= 65535 (enforced by Wrapper::new)
            #[allow(clippy::cast_possible_truncation)]
            let len = w.data.len() as u16;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&w.data);
        }
        buf.extend_from_slice(&self.nonce);
        buf.extend_from_slice(&self.ed25519_pk.0);
        // Write canary (length + string)
        let canary_bytes = self.canary.as_bytes();
        // canary is fixed format "multitop-vault-canary-" + 32 hex chars = 57 chars < 65535
        #[allow(clippy::cast_possible_truncation)]
        buf.extend_from_slice(&(canary_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(canary_bytes);
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // Writing to a Vec<u8> never fails
        self.write_header_without_sig(&mut buf);
        buf.extend_from_slice(&self.signature.0);
        buf
    }

    /// Deserialize header from bytes.
    ///
    /// # Errors
    /// Returns `VaultError::ParseError` if bytes cannot be parsed,
    /// `VaultError::InvalidFormat` if magic is incorrect,
    /// `VaultError::UnsupportedVersion` if version is not supported.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::VaultError> {
        let mut cursor = Cursor::new(bytes);
        Self::from_cursor(&mut cursor)
    }

    /// Deserialize header from cursor.
    ///
    /// # Errors
    /// Returns `VaultError::ParseError` if bytes cannot be parsed,
    /// `VaultError::InvalidFormat` if magic is incorrect,
    /// `VaultError::UnsupportedVersion` if version is not supported.
    /// Deserialize header from cursor.
    ///
    /// # Errors
    /// Returns `VaultError::ParseError` if bytes cannot be parsed,
    /// `VaultError::InvalidFormat` if magic is incorrect,
    /// `VaultError::UnsupportedVersion` if version is not supported.
    #[allow(clippy::too_many_lines)]
    fn from_cursor(cursor: &mut Cursor<&[u8]>) -> Result<Self, crate::VaultError> {
        let mut magic = [0u8; 4];
        cursor
            .read_exact(&mut magic)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        if magic != *b"MQV2" {
            return Err(crate::VaultError::InvalidFormat("invalid magic".into()));
        }

        let mut version = [0u8; 1];
        cursor
            .read_exact(&mut version)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        if version[0] != CURRENT_VERSION {
            return Err(crate::VaultError::UnsupportedVersion(version[0]));
        }

        let mut key_version = [0u8; 1];
        cursor
            .read_exact(&mut key_version)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut created_ts = [0u8; TIMESTAMP_LEN];
        cursor
            .read_exact(&mut created_ts)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let created_timestamp_ms = u64::from_le_bytes(created_ts);

        let mut counter = [0u8; 4];
        cursor
            .read_exact(&mut counter)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let counter = u32::from_le_bytes(counter);

        let mut salt = [0u8; crate::crypto::KEY_LEN];
        cursor
            .read_exact(&mut salt)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut t = [0u8; 1];
        cursor
            .read_exact(&mut t)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let mut m_kib = [0u8; 4];
        cursor
            .read_exact(&mut m_kib)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let mut p = [0u8; 1];
        cursor
            .read_exact(&mut p)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let argon2_params = crate::crypto::Argon2Params {
            t: t[0],
            m_kib: u32::from_le_bytes(m_kib),
            p: p[0],
        };

        let mut wrapper_count = [0u8; 1];
        cursor
            .read_exact(&mut wrapper_count)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let wrapper_count = wrapper_count[0] as usize;

        // Validate wrapper count (max 8 wrappers allowed)
        if wrapper_count > MAX_WRAPPERS {
            return Err(crate::VaultError::ParseError(format!(
                "too many wrappers: {wrapper_count} (max 8)"
            )));
        }

        let mut wrappers = Vec::with_capacity(wrapper_count);
        for _ in 0..wrapper_count {
            let mut wt = [0u8; 1];
            cursor
                .read_exact(&mut wt)
                .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
            let _wrapper_type = crate::crypto::WrapperType::from_u8(wt[0])
                .ok_or_else(|| crate::VaultError::InvalidWrapperType(wt[0]))?;

            let mut len = [0u8; 2];
            cursor
                .read_exact(&mut len)
                .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
            let len = u16::from_le_bytes(len) as usize;

            let mut wrapper_data = vec![0u8; len];
            cursor
                .read_exact(&mut wrapper_data)
                .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

            wrappers.push(crate::crypto::Wrapper::new(
                crate::crypto::WrapperType::from_u8(wt[0])
                    .ok_or(crate::VaultError::InvalidWrapperType(wt[0]))?,
                wrapper_data,
            )?);
        }

        let mut nonce = [0u8; crate::crypto::NONCE_LEN];
        cursor
            .read_exact(&mut nonce)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut ed25519_pk = [0u8; crate::crypto::KEY_LEN];
        cursor
            .read_exact(&mut ed25519_pk)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        // Read canary string (written before signature in write_header_without_sig)
        let mut canary_len = [0u8; 2];
        cursor
            .read_exact(&mut canary_len)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let canary_len = u16::from_le_bytes(canary_len) as usize;
        let mut canary_bytes = vec![0u8; canary_len];
        cursor
            .read_exact(&mut canary_bytes)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let canary = String::from_utf8(canary_bytes)
            .map_err(|_| crate::VaultError::ParseError("invalid canary utf-8".into()))?;

        // Read signature (appended after header in to_bytes)
        let mut sig = [0u8; crate::crypto::SIGNATURE_LEN];
        cursor
            .read_exact(&mut sig)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        Ok(Self {
            magic,
            version: version[0],
            key_version: key_version[0],
            created_timestamp_ms,
            counter,
            salt,
            argon2_params,
            wrappers,
            nonce,
            ed25519_pk: crate::crypto::Ed25519PublicKey(ed25519_pk),
            signature: crate::crypto::Ed25519Signature(sig),
            canary,
        })
    }
}
