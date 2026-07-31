//! macOS Secure Enclave wrapper for vault key wrapping/unwrapping
//!
//! Uses ECIES (Elliptic Curve Integrated Encryption Scheme) with P-256 keys
//! stored in the Secure Enclave. Private key operations require Touch ID/Face ID authentication.

use crate::{VaultError, crypto::{WrapperType, Wrapper, VaultKey}};

#[cfg(target_os = "macos")]
use security_framework::key::{SecKey, KeyType, Token, GenerateKeyOptions, Algorithm};
#[cfg(target_os = "macos")]
use security_framework::access_control::{SecAccessControl};
#[cfg(target_os = "macos")]
use security_framework::item::{ItemSearchOptions, ItemClass, KeyClass, SearchResult, Reference};
#[cfg(target_os = "macos")]
use security_framework_sys::access_control::kSecAccessControlBiometryCurrentSet;

const SE_KEY_LABEL: &str = "com.ztomer.multitop.vault.se-key";

/// Secure Enclave operations for macOS
#[cfg(target_os = "macos")]
pub struct SecureEnclave {
    private_key: SecKey,
    public_key: SecKey,
}

#[cfg(target_os = "macos")]
impl SecureEnclave {
    /// Get or create the Secure Enclave key pair for vault wrapping.
    /// The private key is protected by biometric authentication (Touch ID/Face ID).
    pub fn get_or_create() -> Result<Self, VaultError> {
        // Try to load existing key
        if let Ok(se) = Self::load_existing() {
            return Ok(se);
        }
        // Generate new key pair
        Self::generate_new()
    }

    fn load_existing() -> Result<Self, VaultError> {
        let mut search = ItemSearchOptions::new();
        search.class(ItemClass::key())
            .key_class(KeyClass::private())
            .label(SE_KEY_LABEL)
            .load_refs(true)
            .limit(1);

        let results = search.search()
            .map_err(|e| VaultError::SecureEnclaveError(format!("search: {e}")))?;

        let private_key = match results.first() {
            Some(SearchResult::Ref(Reference::Key(key))) => key.clone(),
            _ => return Err(VaultError::SecureEnclaveError("no existing SE key".into())),
        };

        let public_key = private_key.public_key()
            .ok_or_else(|| VaultError::SecureEnclaveError("no public key".into()))?;

        Ok(Self { private_key, public_key })
    }

fn generate_new() -> Result<Self, VaultError> {
        // Create access control: require biometry (Touch ID/Face ID) for private key operations
        let access_control = SecAccessControl::create_with_flags(
            kSecAccessControlBiometryCurrentSet,
        ).map_err(|e| VaultError::SecureEnclaveError(format!("access control: {e}")))?;

        // Configure key generation for Secure Enclave
        let mut options = GenerateKeyOptions::default();
        options.set_key_type(KeyType::ec());
        options.set_size_in_bits(256);
        options.set_token(Token::SecureEnclave);
        options.set_label(SE_KEY_LABEL);
        options.set_access_control(access_control);

        // Generate the key pair - returns the private key
        let private_key = SecKey::new(&options)
            .map_err(|e| VaultError::SecureEnclaveError(format!("generate: {e}")))?;

        // Get the public key
        let public_key = private_key.public_key()
            .ok_or_else(|| VaultError::SecureEnclaveError("no public key after generate".into()))?;

        Ok(Self { private_key, public_key })
    }

    /// Wrap a vault key using the Secure Enclave public key (ECIES)
    pub fn wrap_key(&self, vault_key: &VaultKey) -> Result<Wrapper, VaultError> {
        // ECIES encryption: public key encrypts, private key decrypts (with Touch ID)
        let data = vault_key.as_bytes();
        let encrypted = self.public_key.encrypt_data(
            Algorithm::ECIESEncryptionStandardX963SHA256AESGCM,
            data,
        ).map_err(|e| VaultError::SecureEnclaveError(format!("wrap: {e}")))?;

        Wrapper::new(WrapperType::SecureEnclave, encrypted)
    }

    /// Unwrap a vault key using the Secure Enclave private key (triggers Touch ID)
    pub fn unwrap_key(&self, wrapper: &Wrapper) -> Result<VaultKey, VaultError> {
        let data = &wrapper.data;
        let decrypted = self.private_key.decrypt_data(
            Algorithm::ECIESEncryptionStandardX963SHA256AESGCM,
            data,
        ).map_err(|e| {
            let err_str = format!("{e}");
            if err_str.contains("authentication failed") || err_str.contains("user cancel") {
                VaultError::BiometricFailed
            } else if err_str.contains("invalid key") || err_str.contains("not found") {
                VaultError::SecureEnclaveError("SE key invalidated".into())
            } else {
                VaultError::SecureEnclaveError(err_str)
            }
        })?;

        if decrypted.len() != 32 {
            return Err(VaultError::SecureEnclaveError("invalid decrypted key size".into()));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&decrypted);
        Ok(VaultKey::from_bytes(key))
    }

    /// Check if Secure Enclave is available (has Touch ID/Face ID enrolled)
    pub fn is_available() -> bool {
        // Try to create an access control with biometry - will fail if no biometry enrolled
        SecAccessControl::create_with_flags(kSecAccessControlBiometryCurrentSet).is_ok()
    }
}

/// Create or get the Secure Enclave wrapper
#[cfg(target_os = "macos")]
pub fn get_secure_enclave() -> Result<SecureEnclave, VaultError> {
    SecureEnclave::get_or_create()
}

/// Check if Secure Enclave is available on this system
#[cfg(target_os = "macos")]
pub fn is_available() -> bool {
    SecureEnclave::is_available()
}

/// Platform stub for non-macOS
#[cfg(not(target_os = "macos"))]
pub fn is_available() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn get_secure_enclave() -> Result<SecureEnclave, VaultError> {
    Err(VaultError::PlatformNotSupported("Secure Enclave only on macOS".into()))
}

#[cfg(not(target_os = "macos"))]
pub struct SecureEnclave;

#[cfg(not(target_os = "macos"))]
impl SecureEnclave {
    pub fn wrap_key(&self, _key: &VaultKey) -> Result<Wrapper, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave only on macOS".into()))
    }
    pub fn unwrap_key(&self, _wrapper: &Wrapper) -> Result<VaultKey, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave only on macOS".into()))
    }
}