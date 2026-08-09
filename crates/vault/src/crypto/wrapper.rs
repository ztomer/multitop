//! How a vault key is wrapped, and by what.
//!
//! A wrapper is the vault key encrypted to one authenticator — a password, a
//! Secure Enclave key, a TPM. A vault carries one per way it can be opened.

/// Wrapper types for different key wrapping methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum WrapperType {
    SecureEnclave = 0x01,
    Tpm2 = 0x02,
    Argon2id = 0x03,
}

impl WrapperType {
    #[must_use]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::SecureEnclave),
            0x02 => Some(Self::Tpm2),
            0x03 => Some(Self::Argon2id),
            _ => None,
        }
    }
}

/// The most a wrapper can hold. The on-disk length is a `u16`, so anything
/// longer could not be read back — the bound is the format's, not a policy.
pub const MAX_WRAPPER_BYTES: usize = u16::MAX as usize;

/// A wrapped vault key
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Wrapper {
    pub wrapper_type: WrapperType,
    pub data: Vec<u8>,
}

impl Wrapper {
    /// Create a new wrapper.
    ///
    /// # Errors
    /// Returns `VaultError::WrapperTooLarge` if `data` exceeds 65,535 bytes.
    pub fn new(wrapper_type: WrapperType, data: Vec<u8>) -> Result<Self, crate::VaultError> {
        if data.len() > MAX_WRAPPER_BYTES {
            return Err(crate::VaultError::WrapperTooLarge(data.len()));
        }
        Ok(Self { wrapper_type, data })
    }
}
