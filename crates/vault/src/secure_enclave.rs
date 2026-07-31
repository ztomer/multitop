//! macOS Secure Enclave wrapper for vault key unwrapping - STUB IMPLEMENTATION
//!
//! This is a stub implementation. The full Secure Enclave integration
//! will be added in a future version once the API stabilizes.

use crate::VaultError;

/// Secure Enclave operations for macOS (stub)
#[cfg(target_os = "macos")]
pub struct SecureEnclave;

#[cfg(target_os = "macos")]
impl SecureEnclave {
    /// Get or create the Secure Enclave key for vault wrapping (stub - always returns unavailable)
    pub fn get_or_create() -> Result<Self, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave integration not yet implemented".into()))
    }

    /// Wrap a vault key using the Secure Enclave public key (ECIES) - stub
    pub fn wrap_key(&self, _vault_key: &crate::VaultKey) -> Result<crate::Wrapper, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave integration not yet implemented".into()))
    }

    /// Unwrap a vault key using the Secure Enclave private key - stub
    pub fn unwrap_key(&self, _wrapper: &crate::Wrapper) -> Result<crate::VaultKey, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave integration not yet implemented".into()))
    }

    /// Check if Secure Enclave is available
    pub fn is_available() -> bool {
        false
    }
}

/// Check if Secure Enclave is available on this system
#[cfg(target_os = "macos")]
pub fn is_available() -> bool {
    false
}

/// Create or get the Secure Enclave wrapper
#[cfg(target_os = "macos")]
pub fn get_secure_enclave() -> Result<SecureEnclave, VaultError> {
    Err(VaultError::PlatformNotSupported("Secure Enclave integration not yet implemented".into()))
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
    pub fn wrap_key(&self, _key: &crate::VaultKey) -> Result<crate::Wrapper, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave only on macOS".into()))
    }
    pub fn unwrap_key(&self, _wrapper: &crate::Wrapper) -> Result<crate::VaultKey, VaultError> {
        Err(VaultError::PlatformNotSupported("Secure Enclave only on macOS".into()))
    }
}