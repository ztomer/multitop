//! The whole vault file: header plus ciphertext, and reading one off disk.

use super::VaultHeader;

/// Complete vault file (header + ciphertext)
pub struct VaultFile {
    pub header: VaultHeader,
    pub ciphertext: Vec<u8>,
}

impl VaultFile {
    /// Read vault file from disk.
    ///
    /// # Errors
    /// Returns `VaultError::Io` if the file cannot be read,
    /// or parse/format errors from `from_bytes`.
    pub fn read(path: &std::path::Path) -> Result<Self, crate::VaultError> {
        let bytes = std::fs::read(path).map_err(crate::VaultError::Io)?;
        Self::from_bytes(&bytes)
    }

    /// Deserialize vault file from bytes.
    ///
    /// # Errors
    /// Returns `VaultError` if header parsing fails or file is too short.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::VaultError> {
        let header = VaultHeader::from_bytes(bytes)?;
        let header_size = header.to_bytes().len();

        if bytes.len() < header_size {
            return Err(crate::VaultError::InvalidFormat("file too short".into()));
        }

        let ciphertext = bytes[header_size..].to_vec();

        Ok(Self { header, ciphertext })
    }
}

/// Read vault file from disk.
///
/// # Errors
/// Returns `VaultError::Io` if the file cannot be read,
/// or parse/format errors from `from_bytes`.
pub fn read_vault_file(path: &std::path::Path) -> Result<VaultFile, crate::VaultError> {
    VaultFile::read(path)
}
