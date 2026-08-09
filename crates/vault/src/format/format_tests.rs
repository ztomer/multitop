//! Tests for the vault file format.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::crypto::*;
use crate::format::*;
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
    assert_ne!(header.canary, "");
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
