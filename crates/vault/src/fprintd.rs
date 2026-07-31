//! Linux fingerprint authentication via fprintd (STUB IMPLEMENTATION)
//!
//! This is a stub implementation. The full fprintd integration
//! will be added in a future version once the API stabilizes.

use crate::VaultError;

/// Fingerprint verification result (matches Linux enum)
#[cfg_attr(target_os = "linux", derive(Debug, Clone, PartialEq, Eq))]
#[cfg_attr(not(target_os = "linux"), derive(Debug, Clone, PartialEq, Eq))]
pub enum FingerprintResult {
    Verified,
    Failed,
    Timeout,
    NotEnrolled,
    Busy,
    Cancelled,
    Error(String),
}

/// Fingerprint verifier using fprintd (stub)
#[cfg(target_os = "linux")]
pub struct FingerprintVerifier;

#[cfg(target_os = "linux")]
impl FingerprintVerifier {
    /// Create new fingerprint verifier (stub - always fails)
    pub async fn new() -> Result<Self, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd integration not yet implemented".into()))
    }

    /// Create with custom device path and finger (stub)
    pub async fn with_device(_device_path: String, _finger: String) -> Result<Self, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd integration not yet implemented".into()))
    }

    /// Set verification timeout
    pub fn with_timeout(self, _timeout: Duration) -> Self {
        self
    }

    /// Verify fingerprint with timeout (stub - always returns unavailable)
    pub async fn verify(&self) -> Result<FingerprintResult, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd integration not yet implemented".into()))
    }

    /// Quick check if fingerprint is available
    pub async fn is_available() -> bool {
        false
    }

    /// List enrolled fingers
    pub async fn list_fingers(&self) -> Result<Vec<String>, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd integration not yet implemented".into()))
    }
}

/// Check if fprintd is available and has enrolled fingers (stub)
#[cfg(target_os = "linux")]
pub async fn check_fprintd() -> Result<Vec<String>, VaultError> {
    Err(VaultError::PlatformNotSupported("fprintd integration not yet implemented".into()))
}

/// Stub for non-Linux platforms
#[cfg(not(target_os = "linux"))]
pub struct FingerprintVerifier;

#[cfg(not(target_os = "linux"))]
impl FingerprintVerifier {
    pub async fn new() -> Result<Self, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
    }
    pub async fn verify(&self, _timeout_sec: u64) -> Result<FingerprintResult, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
    }
    pub async fn is_available() -> bool {
        false
    }
}

