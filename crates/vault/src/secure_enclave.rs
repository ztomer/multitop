//! macOS Secure Enclave wrapper for vault key wrapping/unwrapping
//!
//! Uses ECIES (Elliptic Curve Integrated Encryption Scheme) with P-256 keys
//! stored in the Secure Enclave. Private key operations require Touch ID/Face ID authentication.

use crate::{
    crypto::{VaultKey, Wrapper, WrapperType},
    VaultError,
};

#[cfg(target_os = "macos")]
use security_framework::access_control::SecAccessControl;
#[cfg(target_os = "macos")]
use security_framework::item::{ItemClass, ItemSearchOptions, KeyClass, Reference, SearchResult};
#[cfg(target_os = "macos")]
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey, Token};
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
    /// Get or create the Secure Enclave key pair for vault wrapping.
    ///
    /// # Errors
    /// Returns `VaultError::SecureEnclaveError` if key generation or lookup fails.
    pub fn get_or_create() -> Result<Self, VaultError> {
        // Try to load existing key
        if let Ok(se) = Self::load_existing() {
            return Ok(se);
        }
        // Generate new key pair
        Self::generate_new()
    }

    /// Load the existing Secure Enclave key, never creating one.
    ///
    /// This is what the unlock path must use. `get_or_create` falls through to
    /// `generate_new`, which calls `delete_existing` -- so using it to *read* an
    /// existing wrapper means a transient lookup failure (a locked login
    /// keychain, a momentary search error) destroys the Secure Enclave private
    /// key and permanently orphans the `SecureEnclave` wrapper already sitting
    /// in the vault file. The wrapper cannot be re-derived: the private key
    /// never leaves the enclave, so once deleted the ciphertext is undecryptable
    /// forever. The user would keep being asked for their password with no
    /// indication that biometric unlock had been destroyed rather than declined.
    ///
    /// Creating a key is an enrollment action. Reading one is not.
    ///
    /// # Errors
    /// Returns `VaultError::SecureEnclaveError` if no key exists or lookup fails.
    pub fn load_only() -> Result<Self, VaultError> {
        Self::load_existing()
    }

    fn load_existing() -> Result<Self, VaultError> {
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::key())
            .key_class(KeyClass::private())
            .label(SE_KEY_LABEL)
            .load_refs(true)
            .limit(1);

        let results = search
            .search()
            .map_err(|e| VaultError::SecureEnclaveError(format!("search: {e}")))?;

        let private_key = match results.first() {
            Some(SearchResult::Ref(Reference::Key(key))) => key.clone(),
            _ => return Err(VaultError::SecureEnclaveError("no existing SE key".into())),
        };

        let public_key = private_key
            .public_key()
            .ok_or_else(|| VaultError::SecureEnclaveError("no public key".into()))?;

        Ok(Self {
            private_key,
            public_key,
        })
    }

    fn generate_new() -> Result<Self, VaultError> {
        // Create access control: require biometry (Touch ID/Face ID) for private key operations
        let access_control =
            SecAccessControl::create_with_flags(kSecAccessControlBiometryCurrentSet)
                .map_err(|e| VaultError::SecureEnclaveError(format!("access control: {e}")))?;

        // Delete any existing key with this label first
        let _ = Self::delete_existing();

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
        let public_key = private_key
            .public_key()
            .ok_or_else(|| VaultError::SecureEnclaveError("no public key after generate".into()))?;

        Ok(Self {
            private_key,
            public_key,
        })
    }

    fn delete_existing() -> Result<(), VaultError> {
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::key())
            .key_class(KeyClass::private())
            .label(SE_KEY_LABEL);
        search
            .delete()
            .map_err(|e| VaultError::SecureEnclaveError(format!("delete: {e}")))
    }

    /// Wrap a vault key using the Secure Enclave public key (ECIES)
    ///
    /// # Errors
    /// Returns `VaultError::SecureEnclaveError` if encryption fails.
    pub fn wrap_key(&self, vault_key: &VaultKey) -> Result<Wrapper, VaultError> {
        // ECIES encryption: public key encrypts, private key decrypts (with Touch ID)
        let data = vault_key.as_bytes();
        let encrypted = self
            .public_key
            .encrypt_data(Algorithm::ECIESEncryptionStandardX963SHA256AESGCM, data)
            .map_err(|e| VaultError::SecureEnclaveError(format!("wrap: {e}")))?;

        Wrapper::new(WrapperType::SecureEnclave, encrypted)
    }

    /// Unwrap a vault key using the Secure Enclave private key (triggers Touch ID)
    ///
    /// # Errors
    /// Returns `VaultError::SecureEnclaveError` if decryption fails,
    /// `VaultError::BiometricFailed` if the user cancels authentication,
    /// or `VaultError::InvalidWrapperData` if the decrypted key size is invalid.
    pub fn unwrap_key(&self, wrapper: &Wrapper) -> Result<VaultKey, VaultError> {
        let data = &wrapper.data;
        let decrypted = self
            .private_key
            .decrypt_data(Algorithm::ECIESEncryptionStandardX963SHA256AESGCM, data)
            .map_err(|e| {
                // Use CFError code for proper error classification
                let code = e.code();
                // errSecAuthFailed = -25293 (auth failed / user canceled)
                // errSecUserCanceled = -128 (user canceled)
                // errSecItemNotFound = -25300 (key invalidated)
                if code == -25293 || code == -128 {
                    VaultError::BiometricFailed
                } else if code == -25300 {
                    VaultError::SecureEnclaveError("SE key invalidated".into())
                } else {
                    VaultError::SecureEnclaveError(e.to_string())
                }
            })?;

        if decrypted.len() != crate::crypto::KEY_LEN {
            return Err(VaultError::SecureEnclaveError(
                "invalid decrypted key size".into(),
            ));
        }

        let mut key = [0u8; crate::crypto::KEY_LEN];
        key.copy_from_slice(&decrypted);
        Ok(VaultKey::from_bytes(key))
    }

    /// Check if Secure Enclave is available (has Touch ID/Face ID enrolled)
    #[must_use]
    pub fn is_available() -> bool {
        // Use LAContext to properly check biometric enrollment
        use std::process::Command;
        let output = Command::new("bioutil").args(["-rs"]).output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.contains("Touch ID") || stdout.contains("Face ID")
            }
            Err(_) => false,
        }
    }
}

/// Create or get the Secure Enclave wrapper.
///
/// Enrollment only -- this can generate a new key pair, which deletes any
/// existing one. To read an already-wrapped key, use
/// [`get_secure_enclave_existing`] instead.
///
/// # Errors
/// Returns `VaultError::SecureEnclaveError` if key creation or lookup fails.
#[cfg(target_os = "macos")]
pub fn get_secure_enclave() -> Result<SecureEnclave, VaultError> {
    SecureEnclave::get_or_create()
}

/// Get the existing Secure Enclave wrapper without ever creating one.
///
/// # Errors
/// Returns `VaultError::SecureEnclaveError` if no key exists or lookup fails.
#[cfg(target_os = "macos")]
pub fn get_secure_enclave_existing() -> Result<SecureEnclave, VaultError> {
    SecureEnclave::load_only()
}

/// Get the existing Secure Enclave wrapper without ever creating one.
///
/// # Errors
/// Always returns `VaultError::PlatformNotSupported` off macOS.
#[cfg(not(target_os = "macos"))]
pub fn get_secure_enclave_existing() -> Result<SecureEnclave, VaultError> {
    Err(VaultError::PlatformNotSupported(
        "Secure Enclave only on macOS".into(),
    ))
}

/// Check if Secure Enclave is available on this system
#[cfg(target_os = "macos")]
#[must_use]
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
    Err(VaultError::PlatformNotSupported(
        "Secure Enclave only on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub struct SecureEnclave;

#[cfg(not(target_os = "macos"))]
impl SecureEnclave {
    pub fn wrap_key(&self, _key: &VaultKey) -> Result<Wrapper, VaultError> {
        Err(VaultError::PlatformNotSupported(
            "Secure Enclave only on macOS".into(),
        ))
    }
    pub fn unwrap_key(&self, _wrapper: &Wrapper) -> Result<VaultKey, VaultError> {
        Err(VaultError::PlatformNotSupported(
            "Secure Enclave only on macOS".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    #[cfg(not(target_os = "macos"))]
    use super::*;

    #[test]
    fn test_is_available() {
        // On non-macOS, should always return false
        #[cfg(not(target_os = "macos"))]
        assert!(!is_available());
    }

    #[test]
    fn test_get_secure_enclave_unavailable() {
        #[cfg(not(target_os = "macos"))]
        {
            let result = get_secure_enclave();
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }

    #[test]
    fn test_secure_enclave_wrap_key_unavailable() {
        #[cfg(not(target_os = "macos"))]
        {
            let se = SecureEnclave;
            let key = VaultKey::new();
            let result = se.wrap_key(&key);
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }

    #[test]
    fn test_secure_enclave_unwrap_key_unavailable() {
        #[cfg(not(target_os = "macos"))]
        {
            let se = SecureEnclave;
            let wrapper = Wrapper::new(WrapperType::SecureEnclave, vec![1u8; 64]).unwrap();
            let result = se.unwrap_key(&wrapper);
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }
}
