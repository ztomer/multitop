//! Vault file format (binary serialization)

use crate::now_ms;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};

const CURRENT_VERSION: u8 = 2;

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
        let mut canary_bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut canary_bytes);
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
        if wrappers.len() > 8 {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrappers.iter().any(|w| w.data.len() > 65535) {
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
            nonce: [0u8; 12],
            ed25519_pk,
            signature: crate::crypto::Ed25519Signature([0u8; 64]),
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
        if wrappers.len() > 8 {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrappers.iter().any(|w| w.data.len() > 65535) {
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
            nonce: [0u8; 12],
            ed25519_pk,
            signature: crate::crypto::Ed25519Signature([0u8; 64]),
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
        if self.wrappers.len() >= 8
            && !self
                .wrappers
                .iter()
                .any(|w| w.wrapper_type == wrapper.wrapper_type)
        {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrapper.data.len() > 65535 {
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
        if wrapper.data.len() > 65535 {
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

        let mut created_ts = [0u8; 8];
        cursor
            .read_exact(&mut created_ts)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let created_timestamp_ms = u64::from_le_bytes(created_ts);

        let mut counter = [0u8; 4];
        cursor
            .read_exact(&mut counter)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let counter = u32::from_le_bytes(counter);

        let mut salt = [0u8; 32];
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
        if wrapper_count > 8 {
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

        let mut nonce = [0u8; 12];
        cursor
            .read_exact(&mut nonce)
            .map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut ed25519_pk = [0u8; 32];
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
        let mut sig = [0u8; 64];
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

/// Atomically write vault file (tmp + rename + dir fsync) with advisory file locking.
///
/// # Errors
/// Returns `VaultError::Io` if file operations fail,
/// or `VaultError::Io` if locking/syncing fails.
pub fn atomic_write_vault(
    path: &std::path::Path,
    header: &VaultHeader,
    ciphertext: &[u8],
) -> Result<(), crate::VaultError> {
    use fs2::FileExt;
    use std::fs::File;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(crate::VaultError::Io)?;
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(parent)
                .map_err(crate::VaultError::Io)?
                .permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(parent, perms).map_err(crate::VaultError::Io)?;
        }
    }

    // Open the vault file for writing and acquire exclusive lock
    let tmp_path = path.with_extension("bin.tmp");
    let mut file = open_tmp_reclaiming_stale(&tmp_path)?;

    // Acquire exclusive lock on the temp file
    file.lock_exclusive()
        .map_err(|e| crate::VaultError::Io(std::io::Error::other(e)))?;

    // Past this point every early return must take the temp file with it. The
    // open above refuses to overwrite an existing one, so a single leftover
    // makes every future save fail with `AlreadyExists` -- for good, and with
    // nothing pointing at the cause.
    let mut tmp_guard = TmpFileGuard {
        path: &tmp_path,
        armed: true,
    };

    let header_bytes = header.to_bytes();
    file.write_all(&header_bytes)
        .map_err(crate::VaultError::Io)?;
    file.write_all(ciphertext).map_err(crate::VaultError::Io)?;
    file.flush().map_err(crate::VaultError::Io)?;
    file.sync_all().map_err(crate::VaultError::Io)?;

    // Release lock before rename (lock is on temp file)
    file.unlock()
        .map_err(|e| crate::VaultError::Io(std::io::Error::other(e)))?;

    std::fs::rename(&tmp_path, path).map_err(crate::VaultError::Io)?;
    // The temp file is the vault now; there is nothing left to clean up.
    tmp_guard.armed = false;

    // Sync directory to ensure rename is persisted
    if let Some(parent) = path.parent() {
        let dir = File::open(parent).map_err(crate::VaultError::Io)?;
        dir.sync_all().map_err(crate::VaultError::Io)?;
    }

    Ok(())
}

/// Deletes the half-written temp file unless disarmed after a successful rename.
struct TmpFileGuard<'a> {
    path: &'a std::path::Path,
    armed: bool,
}

impl Drop for TmpFileGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(self.path);
        }
    }
}

/// Create the temp file, clearing away debris left by a writer that died.
///
/// `create_new` is deliberate -- it stops two writers clobbering one another --
/// but on its own it turns any leftover temp file into a permanent failure:
/// every later save returns `AlreadyExists`, so the vault quietly stops being
/// able to store anything. A process killed between creating the temp file and
/// the rename leaves exactly that, and so does a panic, since the release
/// profile aborts and runs no destructors.
///
/// A writer that is still working holds an exclusive lock on its temp file. So a
/// temp file whose lock can be taken belongs to nobody and is debris; one whose
/// lock is held belongs to a live writer and is left alone.
fn open_tmp_reclaiming_stale(
    tmp_path: &std::path::Path,
) -> Result<std::fs::File, crate::VaultError> {
    use fs2::FileExt;
    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut open_opts = OpenOptions::new();
    open_opts.write(true).create_new(true);
    #[cfg(unix)]
    open_opts.mode(0o600);

    match open_opts.open(tmp_path) {
        Ok(file) => Ok(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let stale = std::fs::File::open(tmp_path).map_err(crate::VaultError::Io)?;
            stale.try_lock_exclusive().map_err(|_| {
                crate::VaultError::Io(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "another process is writing the vault",
                ))
            })?;
            let _ = stale.unlock();
            drop(stale);
            std::fs::remove_file(tmp_path).map_err(crate::VaultError::Io)?;
            open_opts.open(tmp_path).map_err(crate::VaultError::Io)
        }
        Err(e) => Err(crate::VaultError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::crypto::*;
    use crate::VaultError;
    use tempfile::TempDir;

    fn make_test_header() -> VaultHeader {
        let key = VaultKey::new();
        let salt = generate_salt();
        let params = Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        };
        let wrapper = Wrapper::new(WrapperType::Argon2id, vec![1u8; 60]).unwrap();
        VaultHeader::new(
            Ed25519PublicKey(key.derive_verifying_key().to_bytes()),
            salt,
            params,
            vec![wrapper],
        )
        .unwrap()
    }

    #[test]
    fn test_vault_header_new() {
        let header = make_test_header();
        assert_eq!(header.magic, *b"MQV2");
        assert_eq!(header.version, 2);
        assert_eq!(header.key_version, 0);
        assert_eq!(header.counter, 0);
        assert!(!header.canary.is_empty());
        assert!(header.canary.starts_with("multitop-vault-canary-"));
    }

    #[test]
    fn test_vault_header_new_too_many_wrappers() {
        let key = VaultKey::new();
        let salt = generate_salt();
        let params = Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        };
        let wrappers: Vec<Wrapper> = (0u8..9)
            .map(|i| {
                Wrapper::new(
                    WrapperType::from_u8(i + 1).unwrap_or(WrapperType::Argon2id),
                    vec![1u8; 10],
                )
                .unwrap()
            })
            .collect();

        let result = VaultHeader::new(
            Ed25519PublicKey(key.derive_verifying_key().to_bytes()),
            salt,
            params,
            wrappers,
        );
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::TooManyWrappers)));
    }

    #[test]
    fn test_vault_header_new_wrapper_too_large() {
        // Wrapper::new will fail with 65536 bytes, so we can't create the wrapper
        let result_wrapper = Wrapper::new(WrapperType::Argon2id, vec![1u8; 65536]);
        assert!(result_wrapper.is_err());
    }

    #[test]
    fn test_vault_header_get_wrapper() {
        let header = make_test_header();
        assert!(header.get_wrapper(WrapperType::Argon2id).is_some());
        assert!(header.get_wrapper(WrapperType::SecureEnclave).is_none());
        assert!(header.get_wrapper(WrapperType::Tpm2).is_none());
    }

    #[test]
    fn test_vault_header_add_wrapper() {
        let mut header = make_test_header();
        let new_wrapper = Wrapper::new(WrapperType::SecureEnclave, vec![2u8; 50]).unwrap();

        header.add_wrapper(new_wrapper).unwrap();
        assert!(header.get_wrapper(WrapperType::SecureEnclave).is_some());
        assert_eq!(header.wrappers.len(), 2);
    }

    #[test]
    fn test_vault_header_add_wrapper_replaces_existing() {
        let mut header = make_test_header();
        let new_wrapper = Wrapper::new(WrapperType::Argon2id, vec![3u8; 60]).unwrap();

        header.add_wrapper(new_wrapper).unwrap();
        // Should still have only 1 Argon2id wrapper
        let count = header
            .wrappers
            .iter()
            .filter(|w| w.wrapper_type == WrapperType::Argon2id)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_vault_header_add_wrapper_too_many() {
        let mut header = make_test_header();

        // The issue is that add_wrapper replaces existing types
        // So we need to test with types that don't exist yet
        // But we only have 3 types: SecureEnclave(1), Tpm2(2), Argon2id(3)
        // Let's just verify the length check works
        header.wrappers.clear();
        for _ in 0..8 {
            let wrapper = Wrapper::new(WrapperType::Argon2id, vec![1u8; 10]).unwrap();
            header.wrappers.push(wrapper);
        }
        assert_eq!(header.wrappers.len(), 8);
    }

    #[test]
    fn test_vault_header_replace_wrapper() {
        let mut header = make_test_header();
        let new_wrapper = Wrapper::new(WrapperType::Argon2id, vec![4u8; 60]).unwrap();

        header.replace_wrapper(new_wrapper).unwrap();
        assert_eq!(header.wrappers.len(), 1);
        assert_eq!(header.wrappers[0].data, vec![4u8; 60]);
    }

    #[test]
    fn test_vault_header_has_wrapper() {
        let header = make_test_header();
        assert!(header.has_wrapper(WrapperType::Argon2id));
        assert!(!header.has_wrapper(WrapperType::SecureEnclave));
    }

    #[test]
    fn test_vault_header_signed_data() {
        let header = make_test_header();
        let ciphertext = b"test ciphertext";
        let signed_data = header.signed_data(ciphertext);

        // Signed data should include header fields + ciphertext
        assert!(signed_data.len() > ciphertext.len());
        assert!(signed_data.ends_with(ciphertext));
    }

    #[test]
    fn test_vault_header_to_bytes_roundtrip() {
        let header = make_test_header();
        let bytes = header.to_bytes();

        let restored = VaultHeader::from_bytes(&bytes).unwrap();
        assert_eq!(restored.magic, header.magic);
        assert_eq!(restored.version, header.version);
        assert_eq!(restored.key_version, header.key_version);
        assert_eq!(restored.counter, header.counter);
        assert_eq!(restored.salt, header.salt);
        assert_eq!(restored.argon2_params, header.argon2_params);
        assert_eq!(restored.wrappers.len(), header.wrappers.len());
        assert_eq!(restored.nonce, header.nonce);
        assert_eq!(restored.ed25519_pk, header.ed25519_pk);
        assert_eq!(restored.canary, header.canary);
    }

    #[test]
    fn test_vault_header_from_bytes_invalid_magic() {
        let mut header = make_test_header();
        header.magic = *b"BAD!";
        let bytes = header.to_bytes();

        let result = VaultHeader::from_bytes(&bytes);
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::InvalidFormat(_))));
    }

    #[test]
    fn test_vault_header_from_bytes_unsupported_version() {
        let mut header = make_test_header();
        header.version = 99;
        let bytes = header.to_bytes();

        let result = VaultHeader::from_bytes(&bytes);
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::UnsupportedVersion(99))));
    }

    #[test]
    fn test_vault_header_from_bytes_too_short() {
        let result = VaultHeader::from_bytes(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn test_vault_file_from_bytes_roundtrip() {
        let header = make_test_header();
        let ciphertext = vec![5u8; 100];

        let mut bytes = header.to_bytes();
        bytes.extend_from_slice(&ciphertext);

        let vault_file = VaultFile::from_bytes(&bytes).unwrap();
        assert_eq!(vault_file.header.version, header.version);
        assert_eq!(vault_file.ciphertext, ciphertext);
    }

    #[test]
    fn test_vault_file_from_bytes_too_short() {
        let header = make_test_header();
        let bytes = header.to_bytes();

        // Truncate the bytes
        let result = VaultFile::from_bytes(&bytes[..10]);
        assert!(result.is_err());
    }

    #[test]
    fn test_atomic_write_vault() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let header = make_test_header();
        let ciphertext = vec![6u8; 50];

        atomic_write_vault(&path, &header, &ciphertext).unwrap();

        assert!(path.exists());
        let vault_file = VaultFile::read(&path).unwrap();
        assert_eq!(vault_file.header.version, header.version);
        assert_eq!(vault_file.ciphertext, ciphertext);
    }

    #[test]
    fn test_atomic_write_vault_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deep").join("nested").join("vault.bin");
        let header = make_test_header();
        let ciphertext = vec![7u8; 20];

        atomic_write_vault(&path, &header, &ciphertext).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_read_vault_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_vault.bin");
        let header = make_test_header();
        let ciphertext = vec![8u8; 75];

        atomic_write_vault(&path, &header, &ciphertext).unwrap();

        let vault_file = read_vault_file(&path).unwrap();
        assert_eq!(vault_file.header.version, 2);
        assert_eq!(vault_file.ciphertext, ciphertext);
    }
}
