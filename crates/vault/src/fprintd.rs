//! Linux fingerprint authentication via fprintd D-Bus service
//!
//! Implements communication with fprintd (fingerprint daemon) for
//! fingerprint verification on Linux systems (Ubuntu 26.04+).

use crate::VaultError;
use std::time::Duration;

#[cfg(target_os = "linux")]
use std::time::Duration;
#[cfg(target_os = "linux")]
use zbus::{Connection, Proxy, fdo::DBusProxy};

/// Fingerprint verification result
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintResult {
    Verified,
    Failed,
    Timeout,
    NotEnrolled,
    Busy,
    Cancelled,
    Error,
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintResult {
    Verified,
    Failed,
    Timeout,
    NotEnrolled,
    Busy,
    Cancelled,
    Error,
}

impl std::fmt::Display for FingerprintResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FingerprintResult::Verified => write!(f, "verified"),
            FingerprintResult::Failed => write!(f, "failed"),
            FingerprintResult::Timeout => write!(f, "timeout"),
            FingerprintResult::NotEnrolled => write!(f, "not enrolled"),
            FingerprintResult::Busy => write!(f, "busy"),
            FingerprintResult::Cancelled => write!(f, "cancelled"),
            FingerprintResult::Error => write!(f, "error"),
        }
    }
}

/// Fingerprint verifier using fprintd D-Bus service
#[cfg(target_os = "linux")]
pub struct FingerprintVerifier {
    device_path: String,
    finger: String,
    timeout: Duration,
    connection: Connection,
}

#[cfg(target_os = "linux")]
impl FingerprintVerifier {
    /// Create new fingerprint verifier with default device and finger
    pub async fn new() -> Result<Self, VaultError> {
        let connection = Connection::system().await
            .map_err(|e| VaultError::FprintdError(format!("D-Bus connection failed: {e}")))?;

        // Get the first available fingerprint device
        let device_proxy = Proxy::new(&connection, "net.reactivated.Fprint", "/net/reactivated/Fprint", "net.reactivated.Fprint").await
            .map_err(|e| VaultError::FprintdError(format!("Fprint proxy failed: {e}")))?;

        let devices: Vec<String> = device_proxy.call_method("GetDevices", &()).await
            .map_err(|e| VaultError::FprintdError(format!("GetDevices failed: {e}")))?;

        if devices.is_empty() {
            return Err(VaultError::PlatformNotSupported("No fingerprint devices found".into()));
        }

        let device_path = devices[0].clone();

        // Get enrolled fingers for this device
        let dev_proxy = Proxy::new(&connection, "net.reactivated.Fprint", &device_path, "net.reactivated.Fprint.Device").await
            .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        let fingers: Vec<String> = dev_proxy.call_method("GetEnrolledFingers", &()).await
            .map_err(|e| VaultError::FprintdError(format!("GetEnrolledFingers failed: {e}")))?;

        if fingers.is_empty() {
            return Err(VaultError::PlatformNotSupported("No enrolled fingers on device".into()));
        }

        Ok(Self {
            device_path,
            finger: fingers[0].clone(),
            timeout: Duration::from_secs(30),
            connection,
        })
    }

    /// Create with custom device path and finger
    pub async fn with_device(device_path: String, finger: String) -> Result<Self, VaultError> {
        let connection = Connection::system().await
            .map_err(|e| VaultError::FprintdError(format!("D-Bus connection failed: {e}")))?;

        // Verify the finger is enrolled
        let dev_proxy = Proxy::new(&connection, "net.reactivated.Fprint", &device_path, "net.reactivated.Fprint.Device").await
            .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        let fingers: Vec<String> = dev_proxy.call_method("GetEnrolledFingers", &()).await
            .map_err(|e| VaultError::FprintdError(format!("GetEnrolledFingers failed: {e}")))?;

        if !fingers.contains(&finger) {
            return Err(VaultError::PlatformNotSupported(format!("Finger '{}' not enrolled on device", finger)));
        }

        Ok(Self {
            device_path,
            finger,
            timeout: Duration::from_secs(30),
            connection,
        })
    }

    /// Set verification timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Verify fingerprint with timeout
    pub async fn verify(&self) -> Result<FingerprintResult, VaultError> {
        let dev_proxy = Proxy::new(&self.connection, "net.reactivated.Fprint", &self.device_path, "net.reactivated.Fprint.Device").await
            .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        // Start verification
        let _: () = dev_proxy.call_method("VerifyStart", &(self.finger.clone())).await
            .map_err(|e| VaultError::FprintdError(format!("VerifyStart failed: {e}")))?;

        // Poll for result
        let start = std::time::Instant::now();
        let result = loop {
            if start.elapsed() >= self.timeout {
                // Cancel verification on timeout
                let _ = dev_proxy.call_method("VerifyStop", &()).await;
                break FingerprintResult::Timeout;
            }

            // Check status
            let status: String = dev_proxy.call_method("GetStatus", &()).await
                .map_err(|e| VaultError::FprintdError(format!("GetStatus failed: {e}")))?;

            match status.as_str() {
                "verify-match" => break FingerprintResult::Verified,
                "verify-no-match" => break FingerprintResult::Failed,
                "verify-fail" => break FingerprintResult::Failed,
                "verify-finger-not-set" => break FingerprintResult::NotEnrolled,
                "verify-retry-scan" => {
                    // Finger moved or bad scan - continue polling for retry
                    continue;
                }
                _ => {}
            }

            // Small delay before polling again
            tokio::time::sleep(Duration::from_millis(200)).await;
        };

        // Always stop verification on completion (match, fail, timeout, not enrolled)
        let _ = dev_proxy.call_method("VerifyStop", &()).await;
        Ok(result)
    }

    /// Quick check if fingerprint is available
    pub async fn is_available() -> bool {
        Self::new().await.is_ok()
    }

    /// List enrolled fingers on default device
    pub async fn list_fingers(&self) -> Result<Vec<String>, VaultError> {
        let dev_proxy = Proxy::new(&self.connection, "net.reactivated.Fprint", &self.device_path, "net.reactivated.Fprint.Device").await
            .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        let fingers: Vec<String> = dev_proxy.call_method("GetEnrolledFingers", &()).await
            .map_err(|e| VaultError::FprintdError(format!("GetEnrolledFingers failed: {e}")))?;

        Ok(fingers)
    }
}

/// Check if fprintd is available and has enrolled fingers
#[cfg(target_os = "linux")]
pub async fn check_fprintd() -> Result<Vec<String>, VaultError> {
    let verifier = FingerprintVerifier::new().await?;
    verifier.list_fingers().await
}

/// Stub for non-Linux platforms
#[cfg(not(target_os = "linux"))]
pub struct FingerprintVerifier;

#[cfg(not(target_os = "linux"))]
impl FingerprintVerifier {
    pub async fn new() -> Result<Self, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
    }
    pub async fn with_device(_device_path: String, _finger: String) -> Result<Self, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
    }
    pub async fn verify(&self) -> Result<FingerprintResult, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
    }
    pub fn with_timeout(self, _timeout: Duration) -> Self {
        self
    }
    pub async fn is_available() -> bool {
        false
    }
    pub async fn list_fingers(&self) -> Result<Vec<String>, VaultError> {
        Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn check_fprintd() -> Result<Vec<String>, VaultError> {
    Err(VaultError::PlatformNotSupported("fprintd only on Linux".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint_result_display() {
        assert_eq!(FingerprintResult::Verified.to_string(), "verified");
        assert_eq!(FingerprintResult::Failed.to_string(), "failed");
        assert_eq!(FingerprintResult::Timeout.to_string(), "timeout");
        assert_eq!(FingerprintResult::NotEnrolled.to_string(), "not enrolled");
        assert_eq!(FingerprintResult::Busy.to_string(), "busy");
        assert_eq!(FingerprintResult::Cancelled.to_string(), "cancelled");
        assert_eq!(FingerprintResult::Error.to_string(), "error");
    }

    #[test]
    fn test_fingerprint_result_debug() {
        // Verify Debug is implemented
        let _ = format!("{:?}", FingerprintResult::Verified);
    }

    #[test]
    fn test_fingerprint_result_clone() {
        let r = FingerprintResult::Verified;
        let r2 = r;
        assert_eq!(r, r2);
    }

    #[test]
    fn test_fingerprint_result_partial_eq() {
        assert_eq!(FingerprintResult::Verified, FingerprintResult::Verified);
        assert_ne!(FingerprintResult::Verified, FingerprintResult::Failed);
    }

    #[tokio::test]
    async fn test_fingerprint_verifier_new_unavailable() {
        #[cfg(not(target_os = "linux"))]
        {
            let result = FingerprintVerifier::new().await;
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }

    #[tokio::test]
    async fn test_fingerprint_verifier_verify_unavailable() {
        #[cfg(not(target_os = "linux"))]
        {
            let verifier = FingerprintVerifier;
            let result = verifier.verify().await;
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }

    #[tokio::test]
    async fn test_fingerprint_verifier_is_available() {
        #[cfg(not(target_os = "linux"))]
        {
            let result = FingerprintVerifier::is_available().await;
            assert!(!result);
        }
    }

    #[tokio::test]
    async fn test_fingerprint_verifier_list_fingers_unavailable() {
        #[cfg(not(target_os = "linux"))]
        {
            let verifier = FingerprintVerifier;
            let result = verifier.list_fingers().await;
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }

    #[tokio::test]
    async fn test_check_fprintd_unavailable() {
        #[cfg(not(target_os = "linux"))]
        {
            let result = check_fprintd().await;
            assert!(result.is_err());
            assert!(matches!(result, Err(VaultError::PlatformNotSupported(_))));
        }
    }
}