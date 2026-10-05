//! Tests for the vault's cryptographic primitives.

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

// ── known answers: the bytes an already-sealed vault depends on ─────────────
//
// Every other test in this file is a ROUNDTRIP, and a roundtrip cannot see the
// failure that matters when a KDF dependency moves a major version. Sealing a
// vault and opening it again proves the two halves of THIS build agree; it says
// nothing about whether this build agrees with the build that sealed the
// vault already sitting on the user's disk. If sha2 0.11, hkdf 0.13 or argon2
// 0.6 changed a single default -- the salt, the output length, the parameter
// interpretation -- every vault on every machine would become unopenable and
// this file would still be green.
//
// So these pin the OUTPUT BYTES, to values produced by the PREVIOUS
// generation (sha2 0.10.9 / hkdf 0.12.4 / argon2 0.5.3) on 2026-10-05. They
// are the only tests in the crate that would notice a KDF change that is
// supposed to be a no-op and is not.
//
// The Argon2 parameters are EXPLICIT rather than `Argon2Params::default()`,
// which is `auto_detect()` and therefore a different machine's answer on every
// host -- a vector that depends on the hardware is not a vector.

/// The password every vector below is derived from. Not a secret; a fixed
/// input is the point.
const KAT_PASSWORD: &[u8] = b"correct horse battery staple";
/// The salt every vector below is derived from.
const KAT_SALT: [u8; 32] = [0x07; 32];
/// The Argon2id tier the vector was taken at: the documented memory floor,
/// three passes, no parallelism. 32 MiB and one pass, so the test costs
/// something but not a second.
const KAT_M_KIB: u32 = 32_768;
const KAT_T: u8 = 3;
const KAT_P: u8 = 1;

/// Argon2id at the vault's own floor, byte for byte.
///
/// This is the derivation that turns a password into the key a vault's bytes
/// are sealed with. If this number moves, every existing vault is unreadable.
#[test]
fn argon2id_matches_the_vectors_existing_vaults_were_sealed_with() {
    let params = Argon2Params {
        t: KAT_T,
        m_kib: KAT_M_KIB,
        p: KAT_P,
    };
    let argon2 = params.to_argon2().unwrap();
    let mut derived = [0u8; 32];
    argon2
        .hash_password_into(KAT_PASSWORD, &KAT_SALT, &mut derived)
        .unwrap();
    assert_eq!(
        hex::encode(derived),
        "b5af6a4b543949d5eef52b724c0d80448d1a29b83e5e3de3059296a0d8322f80",
        "Argon2id output changed: every vault sealed by an earlier build is now \
         unopenable, and a roundtrip test would not have noticed"
    );
}

/// The HKDF sub-key, and the Ed25519 public key it becomes.
///
/// Two halves in one test because they fail together: `derive_verifying_key` is
/// `subkey(b"multitop-vault-signing")` fed to Ed25519, and the signature over
/// an existing vault header is checked against the result.
#[test]
fn the_signing_subkey_matches_what_existing_vaults_derive() {
    let key = VaultKey::from_bytes([0u8; 32]);

    // The signing key itself, so a failure localises to the KDF rather than to
    // Ed25519.
    let signing = key.derive_signing_key();
    assert_eq!(
        hex::encode(signing.to_bytes()),
        "87d43178f6b3ff9efae7a47d4e5131c6d23e81085391fff541c9c1f94bcf5278",
        "the HKDF-SHA256 sub-key changed, so every stored signature stops verifying"
    );

    // And the public half, which is what a header actually carries.
    assert_eq!(
        hex::encode(key.derive_verifying_key().to_bytes()),
        "78b934083975edfdd7b06b0462c417b2ae73167cc2d3dff0a368317661a79bca",
        "the derived verifying key changed, so every stored header fails its \
         signature check"
    );
}

/// The rollback anchor's account hash, so a stored keychain entry still names
/// the same account.
///
/// `rollback::account` is SHA-256 over the canonicalised vault path. It is
/// pinned here because it is the one place a `sha2` bump is observable in data
/// this crate wrote to the OS keychain rather than to a file: a changed digest
/// reads every existing anchor as belonging to a different account, which
/// silently disables rollback detection rather than failing loudly.
#[test]
fn sha256_matches_the_digest_stored_anchors_were_written_with() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        hex::encode(Sha256::digest(b"hello")),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        "SHA-256 output changed, so every stored rollback anchor names a \
         different account and rollback detection is off rather than broken"
    );
}
