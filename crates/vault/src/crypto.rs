//! Cryptographic primitives for the vault

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm,
};
use argon2::{Argon2, Params};
use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use hkdf::Hkdf;
use rand::{RngCore, thread_rng};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Vault encryption key (32 bytes = 256 bits)
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct VaultKey([u8; 32]);

impl VaultKey {
    /// Generate a new random vault key
    pub fn new() -> Self {
        let mut key = [0u8; 32];
        thread_rng().fill_bytes(&mut key);
        Self(key)
    }

    /// Create from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Expose the raw bytes (use sparingly)
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive Ed25519 signing key from vault key via HKDF
    pub fn derive_signing_key(&self) -> SigningKey {
        let hkdf = Hkdf::<Sha256>::new(None, &self.0);
        let mut okm = [0u8; 32];
        hkdf.expand(b"multitop-vault-signing", &mut okm)
            .expect("HKDF expand failed");
        SigningKey::from_bytes(&okm)
    }

    /// Derive Ed25519 verifying key from vault key
    pub fn derive_verifying_key(&self) -> VerifyingKey {
        self.derive_signing_key().verifying_key()
    }

    /// Derive AES-256-GCM encryption sub-key via HKDF (key separation from signing key)
    pub fn encryption_key(&self) -> [u8; 32] {
        let hkdf = Hkdf::<Sha256>::new(None, &self.0);
        let mut okm = [0u8; 32];
        hkdf.expand(b"vault-aes-gcm-key", &mut okm)
            .expect("HKDF expand failed");
        okm
    }
}

impl Default for VaultKey {
    fn default() -> Self {
        Self::new()
    }
}

/// Ed25519 public key wrapper (32 bytes)
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Ed25519PublicKey(#[serde(with = "serde_bytes")] pub [u8; 32]);

impl Ed25519PublicKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Ed25519 signature wrapper (64 bytes)
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Ed25519Signature(#[serde(with = "serde_bytes")] pub [u8; 64]);

impl Ed25519Signature {
    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

/// Argon2id parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Argon2Params {
    pub t: u8,      // iterations
    pub m_kib: u32, // memory in KiB
    pub p: u8,      // parallelism
}

impl Default for Argon2Params {
    fn default() -> Self {
        Self::auto_detect()
    }
}

impl Argon2Params {
    /// Auto-detect reasonable parameters for current hardware
    pub fn auto_detect() -> Self {
        let mem_kib = get_available_memory_kib().unwrap_or(8_388_608); // 8 GiB default
        let target_mib = (mem_kib / 1024 / 4).clamp(64, 1024);
        let t = if target_mib >= 256 { 10 } else if target_mib >= 128 { 8 } else { 6 };
        Self { t, m_kib: (target_mib * 1024) as u32, p: 4 }
    }

    /// Create from config values
    pub fn from_config(t: u8, m_mib: u32, p: u8) -> Self {
        Self {
            t: t.clamp(1, 20),
            m_kib: (m_mib * 1024).clamp(32_768, 4_194_304),
            p: p.clamp(1, 8),
        }
    }

    /// Estimated time in milliseconds
    pub fn estimated_ms(&self) -> u64 {
        (self.m_kib as u64 / 1024) * self.t as u64 / self.p as u64 / 2
    }

    /// Create Argon2 instance
    pub fn to_argon2(&self) -> Result<argon2::Argon2<'static>, crate::VaultError> {
        let params = Params::new(self.m_kib, self.t as u32, self.p as u32, None)
            .map_err(|e| crate::VaultError::Argon2Params(e.to_string()))?;
        Ok(Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params))
    }
}

/// Get available system memory in KiB
fn get_available_memory_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let content = fs::read_to_string("/proc/meminfo").ok()?;
        for line in content.lines() {
            if line.starts_with("MemAvailable:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse().ok();
                }
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // Use vm_page_free_count to get actual available pages instead of total RAM
        let free_pages = Command::new("sysctl")
            .args(["-n", "vm.page_free_count"])
            .output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok());

        let page_size = Command::new("sysctl")
            .args(["-n", "hw.pagesize"])
            .output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok());

        if let (Some(pages), Some(psize)) = (free_pages, page_size) {
            Some(pages * psize / 1024)
        } else {
            // Fallback: use half of total RAM as conservative estimate
            let total = Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output().ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u64>().ok())?;
            Some(total / 1024 / 2)
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}

/// Wrapper types for different key wrapping methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum WrapperType {
    SecureEnclave = 0x01,
    Tpm2 = 0x02,
    Argon2id = 0x03,
}

impl WrapperType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(WrapperType::SecureEnclave),
            0x02 => Some(WrapperType::Tpm2),
            0x03 => Some(WrapperType::Argon2id),
            _ => None,
        }
    }
}

/// A wrapped vault key
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Wrapper {
    pub wrapper_type: WrapperType,
    pub data: Vec<u8>,
}

impl Wrapper {
    pub fn new(wrapper_type: WrapperType, data: Vec<u8>) -> Result<Self, crate::VaultError> {
        if data.len() > 65535 {
            return Err(crate::VaultError::WrapperTooLarge(data.len()));
        }
        Ok(Self { wrapper_type, data })
    }
}

/// Generate a random salt
pub fn generate_salt() -> [u8; 32] {
    let mut salt = [0u8; 32];
    thread_rng().fill_bytes(&mut salt);
    salt
}

/// Get current time in milliseconds
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// Encrypt vault contents with AES-256-GCM (uses HKDF-derived encryption sub-key)
pub fn encrypt_vault(key: &VaultKey, plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12]), crate::VaultError> {
    let enc_key = key.encryption_key();
    let key_arr = GenericArray::clone_from_slice(&enc_key);
    let cipher = Aes256Gcm::new(&key_arr);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| crate::VaultError::EncryptionFailed)?;
    Ok((ciphertext, nonce.into()))
}

/// Decrypt vault contents with AES-256-GCM (uses HKDF-derived encryption sub-key)
pub fn decrypt_vault(key: &VaultKey, nonce: &[u8; 12], ciphertext: &[u8]) -> Result<Vec<u8>, crate::VaultError> {
    let enc_key = key.encryption_key();
    let key_arr = GenericArray::clone_from_slice(&enc_key);
    let cipher = Aes256Gcm::new(&key_arr);
    let nonce_arr = GenericArray::clone_from_slice(nonce);
    let plaintext = cipher
        .decrypt(&nonce_arr, ciphertext)
        .map_err(|_| crate::VaultError::DecryptionFailed)?;
    Ok(plaintext)
}

/// Sign vault data with Ed25519
pub fn sign_vault(key: &VaultKey, data: &[u8]) -> Ed25519Signature {
    let signing_key = key.derive_signing_key();
    let signature = signing_key.sign(data);
    Ed25519Signature(signature.to_bytes())
}

/// Verify vault signature
pub fn verify_vault_signature(pk: &Ed25519PublicKey, data: &[u8], sig: &Ed25519Signature) -> Result<(), crate::VaultError> {
    let verifying_key = VerifyingKey::from_bytes(&pk.0)
        .map_err(|_| crate::VaultError::InvalidPublicKey)?;
    let signature = Signature::from_bytes(&sig.0);
    verifying_key.verify(data, &signature)
        .map_err(|_| crate::VaultError::SignatureVerificationFailed)
}

/// Wrap vault key with Argon2id(password)
pub fn wrap_argon2id(key: &VaultKey, password: &str, salt: &[u8; 32], params: &Argon2Params) -> Result<Vec<u8>, crate::VaultError> {
    let argon2 = params.to_argon2()?;
    let mut wrapping_key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut wrapping_key)
        .map_err(|e| crate::VaultError::Argon2Error(e.to_string()))?;

    // Encrypt the RAW vault key (not the derived sub-key) with wrapping_key using AES-256-GCM
    let key_arr = GenericArray::clone_from_slice(&wrapping_key);
    let cipher = Aes256Gcm::new(&key_arr);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, key.as_bytes() as &[u8])
        .map_err(|_| crate::VaultError::EncryptionFailed)?;

    // Return: nonce(12) || ciphertext(32) || tag(16) = 60 bytes
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Unwrap vault key from Argon2id wrapped form
pub fn unwrap_argon2id(wrapped: &[u8], password: &str, salt: &[u8; 32], params: &Argon2Params) -> Result<VaultKey, crate::VaultError> {
    if wrapped.len() < 12 + 32 + 16 {
        return Err(crate::VaultError::InvalidWrapperData("too short".into()));
    }

    let argon2 = params.to_argon2()?;
    let mut wrapping_key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut wrapping_key)
        .map_err(|e| crate::VaultError::Argon2Error(e.to_string()))?;

    let nonce_arr = GenericArray::clone_from_slice(&wrapped[0..12]);
    let ciphertext = &wrapped[12..];
    let key_arr = GenericArray::clone_from_slice(&wrapping_key);
    let cipher = Aes256Gcm::new(&key_arr);
    let plaintext = cipher
        .decrypt(&nonce_arr, ciphertext)
        .map_err(|_| crate::VaultError::DecryptionFailed)?;

    if plaintext.len() != 32 {
        return Err(crate::VaultError::InvalidWrapperData("wrong key size".into()));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext);
    Ok(VaultKey(key))
}

/// Securely overwrite a file with random data + zeros before deletion.
/// Best-effort on modern SSDs with encryption; use full-disk encryption for real protection.
pub fn secure_overwrite(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let len = metadata.len() as usize;

    if len == 0 {
        return Ok(());
    }

    use rand::RngCore;

    // Pass 1: random data
    let mut rng = rand::thread_rng();
    let mut buf = vec![0u8; len];
    rng.fill_bytes(&mut buf);
    std::fs::write(path, &buf)?;

    // Pass 2: zeros
    buf.fill(0);
    std::fs::write(path, &buf)?;

    // Pass 3: random data
    rng.fill_bytes(&mut buf);
    std::fs::write(path, &buf)?;

    // Final sync
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.sync_all()?;

    Ok(())
}