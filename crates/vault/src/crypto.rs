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
        // HKDF expand with SHA-256 and 32 bytes output should never fail
        // but we use expect for safety rather than changing the API
        hkdf.expand(b"multitop-vault-signing", &mut okm)
            .expect("HKDF expand failed (should never happen with SHA-256)");
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
        // HKDF expand with SHA-256 and 32 bytes output should never fail
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Encrypt vault contents with AES-256-GCM (uses HKDF-derived encryption sub-key)
pub fn encrypt_vault(key: &VaultKey, plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12]), crate::VaultError> {
    let mut enc_key = key.encryption_key();
    let key_arr = GenericArray::clone_from_slice(&enc_key);
    let cipher = Aes256Gcm::new(&key_arr);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| crate::VaultError::EncryptionFailed)?;
    enc_key.zeroize();
    Ok((ciphertext, nonce.into()))
}

/// Decrypt vault contents with AES-256-GCM (uses HKDF-derived encryption sub-key)
pub fn decrypt_vault(key: &VaultKey, nonce: &[u8; 12], ciphertext: &[u8]) -> Result<Vec<u8>, crate::VaultError> {
    let mut enc_key = key.encryption_key();
    let key_arr = GenericArray::clone_from_slice(&enc_key);
    let cipher = Aes256Gcm::new(&key_arr);
    let nonce_arr = GenericArray::clone_from_slice(nonce);
    let plaintext = cipher
        .decrypt(&nonce_arr, ciphertext)
        .map_err(|_| crate::VaultError::DecryptionFailed)?;
    enc_key.zeroize();
    Ok(plaintext)
}

/// Sign vault data with Ed25519
pub fn sign_vault(key: &VaultKey, data: &[u8]) -> Ed25519Signature {
    let signing_key = key.derive_signing_key();
    let signature = signing_key.sign(data);
    // signing_key implements ZeroizeOnDrop, will be zeroized when dropped
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

    // Zeroize the wrapping key
    wrapping_key.zeroize();

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
    let mut plaintext = cipher
        .decrypt(&nonce_arr, ciphertext)
        .map_err(|_| crate::VaultError::DecryptionFailed)?;

    // Zeroize the wrapping key
    wrapping_key.zeroize();

    if plaintext.len() != 32 {
        plaintext.zeroize();
        return Err(crate::VaultError::InvalidWrapperData("wrong key size".into()));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext);
    plaintext.zeroize(); // Zeroize the decrypted plaintext

    Ok(VaultKey(key))
}

/// Securely overwrite a file with random data + zeros before deletion.
/// Best-effort on modern SSDs with encryption; use full-disk encryption for real protection.
pub fn secure_overwrite(path: &std::path::Path) -> std::io::Result<()> {
    use std::fs::{File, OpenOptions};
    use std::io::{Seek, Write};
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let metadata = std::fs::metadata(path)?;
    let len = metadata.len() as usize;

    if len == 0 {
        return Ok(());
    }

    use rand::RngCore;

    // Open the file once for all passes to avoid TOCTOU
    #[allow(unused_mut)]
    let mut open_opts = OpenOptions::new();
    open_opts.write(true).truncate(false);
    #[cfg(unix)]
    open_opts.mode(0o600);
    
    let mut file = open_opts.open(path)?;
    let mut rng = rand::thread_rng();

    // Pass 1: random data
    let mut buf = vec![0u8; len];
    rng.fill_bytes(&mut buf);
    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(&buf)?;
    file.sync_all()?;

    // Pass 2: zeros
    buf.fill(0);
    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(&buf)?;
    file.sync_all()?;

    // Pass 3: random data
    rng.fill_bytes(&mut buf);
    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(&buf)?;
    file.sync_all()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VaultError;
    use tempfile::TempDir;

    #[test]
    fn test_vault_key_new_random() {
        let key1 = VaultKey::new();
        let key2 = VaultKey::new();
        // Two random keys should be different (probabilistic, but practically certain)
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_vault_key_from_bytes() {
        let bytes = [42u8; 32];
        let key = VaultKey::from_bytes(bytes);
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn test_vault_key_default() {
        let key = VaultKey::default();
        // Default should generate a random key
        assert!(!key.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_vault_key_derive_signing_key() {
        let key = VaultKey::from_bytes([1u8; 32]);
        let signing_key = key.derive_signing_key();
        // Deriving twice should produce the same key
        let signing_key2 = key.derive_signing_key();
        assert_eq!(signing_key.to_bytes(), signing_key2.to_bytes());
    }

    #[test]
    fn test_vault_key_derive_verifying_key() {
        let key = VaultKey::from_bytes([2u8; 32]);
        let vk = key.derive_verifying_key();
        // Deriving twice should produce the same key
        let vk2 = key.derive_verifying_key();
        assert_eq!(vk.to_bytes(), vk2.to_bytes());
    }

    #[test]
    fn test_vault_key_encryption_key() {
        let key = VaultKey::from_bytes([3u8; 32]);
        let enc_key = key.encryption_key();
        // Deriving twice should produce the same key
        let enc_key2 = key.encryption_key();
        assert_eq!(enc_key, enc_key2);
    }

    #[test]
    fn test_vault_key_signing_and_encryption_are_different() {
        let key = VaultKey::from_bytes([4u8; 32]);
        let signing_key = key.derive_signing_key().to_bytes();
        let enc_key = key.encryption_key();
        // HKDF with different labels should produce different keys
        assert_ne!(signing_key, enc_key);
    }

    #[test]
    fn test_ed25519_public_key() {
        let pk = Ed25519PublicKey([5u8; 32]);
        assert_eq!(pk.as_bytes(), &[5u8; 32]);
    }

    #[test]
    fn test_ed25519_signature() {
        let sig = Ed25519Signature([6u8; 64]);
        assert_eq!(sig.as_bytes(), &[6u8; 64]);
    }

    #[test]
    fn test_argon2_params_default() {
        let params = Argon2Params::default();
        // Default should have reasonable values
        assert!(params.t >= 1 && params.t <= 20);
        assert!(params.m_kib >= 32_768 && params.m_kib <= 4_194_304);
        assert!(params.p >= 1 && params.p <= 8);
    }

    #[test]
    fn test_argon2_params_from_config() {
        let params = Argon2Params::from_config(5, 128, 2);
        assert_eq!(params.t, 5);
        assert_eq!(params.m_kib, 128 * 1024);
        assert_eq!(params.p, 2);
    }

    #[test]
    fn test_argon2_params_from_config_clamped() {
        // Values outside bounds should be clamped
        let params = Argon2Params::from_config(0, 1, 0);
        assert_eq!(params.t, 1); // clamped from 0
        assert_eq!(params.p, 1); // clamped from 0
        
        let params = Argon2Params::from_config(255, 10000, 255);
        assert_eq!(params.t, 20); // clamped from 255
        assert_eq!(params.p, 8); // clamped from 255
    }

    #[test]
    fn test_argon2_params_estimated_ms() {
        let params = Argon2Params { t: 10, m_kib: 262_144, p: 4 }; // 256 MiB
        let ms = params.estimated_ms();
        // (262144/1024) * 10 / 4 / 2 = 256 * 10 / 4 / 2 = 320 ms
        assert_eq!(ms, 320);
    }

    #[test]
    fn test_argon2_params_to_argon2() {
        let params = Argon2Params { t: 1, m_kib: 32_768, p: 1 };
        let argon2 = params.to_argon2();
        assert!(argon2.is_ok());
    }

    #[test]
    fn test_wrapper_type_from_u8() {
        assert_eq!(WrapperType::from_u8(0x01), Some(WrapperType::SecureEnclave));
        assert_eq!(WrapperType::from_u8(0x02), Some(WrapperType::Tpm2));
        assert_eq!(WrapperType::from_u8(0x03), Some(WrapperType::Argon2id));
        assert_eq!(WrapperType::from_u8(0x00), None);
        assert_eq!(WrapperType::from_u8(0x04), None);
        assert_eq!(WrapperType::from_u8(0xFF), None);
    }

    #[test]
    fn test_wrapper_new_ok() {
        let wrapper = Wrapper::new(WrapperType::Argon2id, vec![1u8; 100]);
        assert!(wrapper.is_ok());
        let w = wrapper.unwrap();
        assert_eq!(w.wrapper_type, WrapperType::Argon2id);
        assert_eq!(w.data.len(), 100);
    }

    #[test]
    fn test_wrapper_new_too_large() {
        let wrapper = Wrapper::new(WrapperType::Argon2id, vec![1u8; 65536]);
        assert!(wrapper.is_err());
        assert!(matches!(wrapper, Err(VaultError::WrapperTooLarge(65536))));
    }

    #[test]
    fn test_generate_salt() {
        let salt1 = generate_salt();
        let salt2 = generate_salt();
        // Two random salts should be different
        assert_ne!(salt1, salt2);
    }

    #[test]
    fn test_now_ms() {
        let t1 = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = now_ms();
        assert!(t2 >= t1);
        assert!(t2 - t1 >= 10);
    }

    #[test]
    fn test_encrypt_decrypt_vault_roundtrip() {
        let key = VaultKey::new();
        let plaintext = b"hello, vault!";
        
        let (ciphertext, nonce) = encrypt_vault(&key, plaintext).unwrap();
        assert_ne!(ciphertext, plaintext);
        
        let decrypted = decrypt_vault(&key, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_vault_wrong_key_fails() {
        let key1 = VaultKey::new();
        let key2 = VaultKey::new();
        let plaintext = b"secret data";
        
        let (ciphertext, nonce) = encrypt_vault(&key1, plaintext).unwrap();
        let result = decrypt_vault(&key2, &nonce, &ciphertext);
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::DecryptionFailed)));
    }

    #[test]
    fn test_encrypt_decrypt_vault_wrong_nonce_fails() {
        let key = VaultKey::new();
        let plaintext = b"secret data";
        
        let (ciphertext, _) = encrypt_vault(&key, plaintext).unwrap();
        let wrong_nonce = [99u8; 12];
        let result = decrypt_vault(&key, &wrong_nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_sign_verify_vault_roundtrip() {
        let key = VaultKey::new();
        let data = b"important data to sign";
        
        let sig = sign_vault(&key, data);
        let pk = Ed25519PublicKey(key.derive_verifying_key().to_bytes());
        
        let result = verify_vault_signature(&pk, data, &sig);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_vault_signature_wrong_key_fails() {
        let key1 = VaultKey::new();
        let key2 = VaultKey::new();
        let data = b"important data";
        
        let sig = sign_vault(&key1, data);
        let pk2 = Ed25519PublicKey(key2.derive_verifying_key().to_bytes());
        
        let result = verify_vault_signature(&pk2, data, &sig);
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::SignatureVerificationFailed)));
    }

    #[test]
    fn test_verify_vault_signature_tampered_data_fails() {
        let key = VaultKey::new();
        let data = b"original data";
        let tampered = b"tampered data";
        
        let sig = sign_vault(&key, data);
        let pk = Ed25519PublicKey(key.derive_verifying_key().to_bytes());
        
        let result = verify_vault_signature(&pk, tampered, &sig);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_vault_signature_invalid_public_key() {
        let key = VaultKey::new();
        let data = b"test data";
        
        let sig = sign_vault(&key, data);
        // Create an invalid public key (all zeros is not a valid Ed25519 point)
        // Ed25519 will reject this as it's not on the curve
        let invalid_pk = Ed25519PublicKey([0u8; 32]);
        
        let result = verify_vault_signature(&invalid_pk, data, &sig);
        // Should fail with either InvalidPublicKey or SignatureVerificationFailed
        assert!(result.is_err());
    }

    #[test]
    fn test_wrap_unwrap_argon2id_roundtrip() {
        let key = VaultKey::new();
        let password = "strong-password-123";
        let salt = generate_salt();
        let params = Argon2Params { t: 1, m_kib: 32_768, p: 1 };
        
        let wrapped = wrap_argon2id(&key, password, &salt, &params).unwrap();
        assert!(wrapped.len() >= 12 + 32 + 16); // nonce + ciphertext + tag
        
        let unwrapped = unwrap_argon2id(&wrapped, password, &salt, &params).unwrap();
        assert_eq!(key.as_bytes(), unwrapped.as_bytes());
    }

    #[test]
    fn test_wrap_unwrap_argon2id_wrong_password_fails() {
        let key = VaultKey::new();
        let password = "correct-password";
        let wrong_password = "wrong-password";
        let salt = generate_salt();
        let params = Argon2Params { t: 1, m_kib: 32_768, p: 1 };
        
        let wrapped = wrap_argon2id(&key, password, &salt, &params).unwrap();
        let result = unwrap_argon2id(&wrapped, wrong_password, &salt, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_wrap_unwrap_argon2id_wrong_salt_fails() {
        let key = VaultKey::new();
        let password = "password";
        let salt1 = generate_salt();
        let salt2 = generate_salt();
        let params = Argon2Params { t: 1, m_kib: 32_768, p: 1 };
        
        let wrapped = wrap_argon2id(&key, password, &salt1, &params).unwrap();
        let result = unwrap_argon2id(&wrapped, password, &salt2, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_unwrap_argon2id_too_short_fails() {
        let params = Argon2Params { t: 1, m_kib: 32_768, p: 1 };
        let salt = generate_salt();
        let result = unwrap_argon2id(&[0u8; 10], "password", &salt, &params);
        assert!(result.is_err());
        assert!(matches!(result, Err(VaultError::InvalidWrapperData(_))));
    }

    #[test]
    fn test_secure_overwrite() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"sensitive data").unwrap();
        
        secure_overwrite(&path).unwrap();
        
        // File should still exist but with different content
        assert!(path.exists());
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content.len(), 14); // Same length as original
        assert_ne!(content, b"sensitive data"); // But different content
    }

    #[test]
    fn test_secure_overwrite_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.bin");
        std::fs::write(&path, b"").unwrap();
        
        secure_overwrite(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_secure_overwrite_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.bin");
        
        let result = secure_overwrite(&path);
        assert!(result.is_err());
    }
}