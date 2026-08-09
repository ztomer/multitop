//! Tests for the vault's cryptographic primitives.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::crypto::*;
use crate::VaultError;

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
fn test_argon2_params_to_argon2() {
    let params = Argon2Params {
        t: 1,
        m_kib: 32_768,
        p: 1,
    };
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
    assert!(matches!(
        result,
        Err(VaultError::SignatureVerificationFailed)
    ));
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
    let params = Argon2Params {
        t: 1,
        m_kib: 32_768,
        p: 1,
    };

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
    let params = Argon2Params {
        t: 1,
        m_kib: 32_768,
        p: 1,
    };

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
    let params = Argon2Params {
        t: 1,
        m_kib: 32_768,
        p: 1,
    };

    let wrapped = wrap_argon2id(&key, password, &salt1, &params).unwrap();
    let result = unwrap_argon2id(&wrapped, password, &salt2, &params);
    assert!(result.is_err());
}

#[test]
fn test_unwrap_argon2id_too_short_fails() {
    let params = Argon2Params {
        t: 1,
        m_kib: 32_768,
        p: 1,
    };
    let salt = generate_salt();
    let result = unwrap_argon2id(&[0u8; 10], "password", &salt, &params);
    assert!(result.is_err());
    assert!(matches!(result, Err(VaultError::InvalidWrapperData(_))));
}
