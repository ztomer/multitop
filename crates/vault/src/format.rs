//! Vault file format (binary serialization)

use crate::now_ms;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

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

        // Generate a random canary for integrity checking
        let mut canary_bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut canary_bytes);
        let canary = format!("multitop-vault-canary-{}", hex::encode(&canary_bytes[..16]));

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

    pub fn get_wrapper(&self, wrapper_type: crate::crypto::WrapperType) -> Option<&crate::crypto::Wrapper> {
        self.wrappers.iter().find(|w| w.wrapper_type == wrapper_type)
    }

    pub fn add_wrapper(&mut self, wrapper: crate::crypto::Wrapper) -> Result<(), crate::VaultError> {
        if self.wrappers.len() >= 8 && !self.wrappers.iter().any(|w| w.wrapper_type == wrapper.wrapper_type) {
            return Err(crate::VaultError::TooManyWrappers);
        }
        if wrapper.data.len() > 65535 {
            return Err(crate::VaultError::WrapperTooLarge(65535));
        }
        self.wrappers.retain(|w| w.wrapper_type != wrapper.wrapper_type);
        self.wrappers.push(wrapper);
        Ok(())
    }

    pub fn replace_wrapper(&mut self, wrapper: crate::crypto::Wrapper) -> Result<(), crate::VaultError> {
        if wrapper.data.len() > 65535 {
            return Err(crate::VaultError::WrapperTooLarge(65535));
        }
        self.wrappers.retain(|w| w.wrapper_type != wrapper.wrapper_type);
        self.wrappers.push(wrapper);
        Ok(())
    }

    pub fn has_wrapper(&self, wrapper_type: crate::crypto::WrapperType) -> bool {
        self.get_wrapper(wrapper_type).is_some()
    }

    /// Data that gets signed (header without signature + ciphertext)
    pub fn signed_data(&self, ciphertext: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        self.write_header_without_sig(&mut data).expect("write to vec");
        data.extend_from_slice(ciphertext);
        data
    }

    fn write_header_without_sig(&self, buf: &mut Vec<u8>) -> std::io::Result<()> {
        buf.extend_from_slice(&self.magic);
        buf.push(self.version);
        buf.push(self.key_version);
        buf.extend_from_slice(&self.created_timestamp_ms.to_le_bytes());
        buf.extend_from_slice(&self.counter.to_le_bytes());
        buf.extend_from_slice(&self.salt);
        buf.push(self.argon2_params.t);
        buf.extend_from_slice(&self.argon2_params.m_kib.to_le_bytes());
        buf.push(self.argon2_params.p);
        buf.push(self.wrappers.len() as u8);
        for w in &self.wrappers {
            buf.push(w.wrapper_type as u8);
            let len = w.data.len() as u16;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&w.data);
        }
        buf.extend_from_slice(&self.nonce);
        buf.extend_from_slice(&self.ed25519_pk.0);
        // Write canary (length + string)
        let canary_bytes = self.canary.as_bytes();
        buf.extend_from_slice(&(canary_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(canary_bytes);
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_header_without_sig(&mut buf).expect("write to vec");
        buf.extend_from_slice(&self.signature.0);
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::VaultError> {
        let mut cursor = Cursor::new(bytes);
        Self::from_cursor(&mut cursor)
    }

    fn from_cursor(cursor: &mut Cursor<&[u8]>) -> Result<Self, crate::VaultError> {
        let mut magic = [0u8; 4];
        cursor.read_exact(&mut magic).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        if magic != *b"MQV2" {
            return Err(crate::VaultError::InvalidFormat("invalid magic".into()));
        }

        let mut version = [0u8; 1];
        cursor.read_exact(&mut version).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        if version[0] != CURRENT_VERSION {
            return Err(crate::VaultError::UnsupportedVersion(version[0]));
        }

        let mut key_version = [0u8; 1];
        cursor.read_exact(&mut key_version).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut created_ts = [0u8; 8];
        cursor.read_exact(&mut created_ts).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let created_timestamp_ms = u64::from_le_bytes(created_ts);

        let mut counter = [0u8; 4];
        cursor.read_exact(&mut counter).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let counter = u32::from_le_bytes(counter);

        let mut salt = [0u8; 32];
        cursor.read_exact(&mut salt).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut t = [0u8; 1];
        cursor.read_exact(&mut t).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let mut m_kib = [0u8; 4];
        cursor.read_exact(&mut m_kib).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let mut p = [0u8; 1];
        cursor.read_exact(&mut p).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let argon2_params = crate::crypto::Argon2Params {
            t: t[0],
            m_kib: u32::from_le_bytes(m_kib),
            p: p[0],
        };

        let mut wrapper_count = [0u8; 1];
        cursor.read_exact(&mut wrapper_count).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let wrapper_count = wrapper_count[0] as usize;

        let mut wrappers = Vec::with_capacity(wrapper_count);
        for _ in 0..wrapper_count {
            let mut wt = [0u8; 1];
            cursor.read_exact(&mut wt).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
            let _wrapper_type = crate::crypto::WrapperType::from_u8(wt[0])
                .ok_or_else(|| crate::VaultError::InvalidWrapperType(wt[0]))?;

            let mut len = [0u8; 2];
            cursor.read_exact(&mut len).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
            let len = u16::from_le_bytes(len) as usize;

            let mut wrapper_data = vec![0u8; len];
            cursor.read_exact(&mut wrapper_data).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

            wrappers.push(crate::crypto::Wrapper::new(
                crate::crypto::WrapperType::from_u8(wt[0]).ok_or(crate::VaultError::InvalidWrapperType(wt[0]))?,
                wrapper_data,
            )?);
        }

        let mut nonce = [0u8; 12];
        cursor.read_exact(&mut nonce).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        let mut ed25519_pk = [0u8; 32];
        cursor.read_exact(&mut ed25519_pk).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

        // Read canary string (written before signature in write_header_without_sig)
        let mut canary_len = [0u8; 2];
        cursor.read_exact(&mut canary_len).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let canary_len = u16::from_le_bytes(canary_len) as usize;
        let mut canary_bytes = vec![0u8; canary_len];
        cursor.read_exact(&mut canary_bytes).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;
        let canary = String::from_utf8(canary_bytes).map_err(|_| crate::VaultError::ParseError("invalid canary utf-8".into()))?;

        // Read signature (appended after header in to_bytes)
        let mut sig = [0u8; 64];
        cursor.read_exact(&mut sig).map_err(|e| crate::VaultError::ParseError(e.to_string()))?;

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
    pub fn read(path: &std::path::Path) -> Result<Self, crate::VaultError> {
        let bytes = std::fs::read(path).map_err(crate::VaultError::Io)?;
        Self::from_bytes(&bytes)
    }

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

/// Read vault file from disk
pub fn read_vault_file(path: &std::path::Path) -> Result<VaultFile, crate::VaultError> {
    VaultFile::read(path)
}

/// Atomically write vault file (tmp + rename + dir fsync) with advisory file locking
pub fn atomic_write_vault(path: &std::path::Path, header: &VaultHeader, ciphertext: &[u8]) -> Result<(), crate::VaultError> {
    use std::fs::{File, OpenOptions};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use fs2::FileExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(crate::VaultError::Io)?;
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(parent).map_err(crate::VaultError::Io)?.permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(parent, perms).map_err(crate::VaultError::Io)?;
        }
    }

    // Open the vault file for writing and acquire exclusive lock
    let tmp_path = path.with_extension("bin.tmp");
    #[allow(unused_mut)]
    let mut open_opts = OpenOptions::new();
    open_opts.write(true).create_new(true);
    #[cfg(unix)]
    open_opts.mode(0o600);
    let mut file = open_opts.open(&tmp_path).map_err(crate::VaultError::Io)?;

    // Acquire exclusive lock on the temp file
    file.lock_exclusive().map_err(|e| crate::VaultError::Io(std::io::Error::other(e)))?;

    let header_bytes = header.to_bytes();
    file.write_all(&header_bytes).map_err(crate::VaultError::Io)?;
    file.write_all(ciphertext).map_err(crate::VaultError::Io)?;
    file.flush().map_err(crate::VaultError::Io)?;
    file.sync_all().map_err(crate::VaultError::Io)?;

    // Release lock before rename (lock is on temp file)
    file.unlock().map_err(|e| crate::VaultError::Io(std::io::Error::other(e)))?;

    std::fs::rename(&tmp_path, path).map_err(crate::VaultError::Io)?;

    // Sync directory to ensure rename is persisted
    if let Some(parent) = path.parent() {
        let dir = File::open(parent).map_err(crate::VaultError::Io)?;
        dir.sync_all().map_err(crate::VaultError::Io)?;
    }

    Ok(())
}