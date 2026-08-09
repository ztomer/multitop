//! Linux fingerprint authentication via fprintd D-Bus service
//!
//! Implements communication with fprintd (fingerprint daemon) for
//! fingerprint verification on Linux systems (Ubuntu 26.04+).

use crate::VaultError;
use std::time::Duration;
#[cfg(target_os = "linux")]
use zbus::{Connection, Proxy};

/// Deserialize a D-Bus reply body, naming the call in the error.
///
/// One helper rather than the same three lines at each call site: `call_method`
/// hands back a `Message`, and every site has to unpack it the same way. The
/// previous code annotated the binding type and let `?` do the conversion, which
/// silently stopped being possible when zbus changed the return type -- and
/// nothing noticed, because this module is compiled only on Linux and nothing
/// had built it in a very long time.
#[cfg(target_os = "linux")]
fn reply<T>(result: zbus::Result<zbus::Message>, call: &str) -> Result<T, VaultError>
where
    T: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    let message = result.map_err(|e| VaultError::FprintdError(format!("{call} failed: {e}")))?;
    // `DeserializeOwned`, so the value does not borrow from the body and the
    // body can be dropped here. Borrowing would need the body to outlive this
    // function, which is only reachable by leaking it -- on a path taken in a
    // polling loop.
    message
        .body()
        .deserialize()
        .map_err(|e| VaultError::FprintdError(format!("{call} returned unreadable data: {e}")))
}

/// How long to wait on one fingerprint-daemon call.
///
/// The daemon talks to hardware and can simply not answer; without a bound the
/// unlock hangs with nothing on screen saying why.
#[cfg(target_os = "linux")]
const FPRINTD_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long to wait between polls of an in-progress verification.
#[cfg(target_os = "linux")]
const FPRINTD_POLL_INTERVAL: Duration = Duration::from_millis(200);

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
            Self::Verified => write!(f, "verified"),
            Self::Failed => write!(f, "failed"),
            Self::Timeout => write!(f, "timeout"),
            Self::NotEnrolled => write!(f, "not enrolled"),
            Self::Busy => write!(f, "busy"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Error => write!(f, "error"),
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
    /// Create new fingerprint verifier with default device and finger.
    ///
    /// # Errors
    /// `FprintdError` if the system bus or `fprintd` cannot be reached, and
    /// `PlatformNotSupported` if the host has no fingerprint device.
    pub async fn new() -> Result<Self, VaultError> {
        let connection = Connection::system()
            .await
            .map_err(|e| VaultError::FprintdError(format!("D-Bus connection failed: {e}")))?;

        // Get the first available fingerprint device
        let device_proxy = Proxy::new(
            &connection,
            "net.reactivated.Fprint",
            "/net/reactivated/Fprint",
            "net.reactivated.Fprint",
        )
        .await
        .map_err(|e| VaultError::FprintdError(format!("Fprint proxy failed: {e}")))?;

        let devices: Vec<String> = reply(
            device_proxy.call_method("GetDevices", &()).await,
            "GetDevices",
        )?;

        if devices.is_empty() {
            return Err(VaultError::PlatformNotSupported(
                "No fingerprint devices found".into(),
            ));
        }

        let device_path = devices[0].clone();

        // Get enrolled fingers for this device
        let dev_proxy = Proxy::new(
            &connection,
            "net.reactivated.Fprint",
            device_path.as_str(),
            "net.reactivated.Fprint.Device",
        )
        .await
        .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        let fingers: Vec<String> = reply(
            dev_proxy.call_method("GetEnrolledFingers", &()).await,
            "GetEnrolledFingers",
        )?;

        if fingers.is_empty() {
            return Err(VaultError::PlatformNotSupported(
                "No enrolled fingers on device".into(),
            ));
        }

        Ok(Self {
            device_path,
            finger: fingers[0].clone(),
            timeout: FPRINTD_CALL_TIMEOUT,
            connection,
        })
    }

    /// Create with custom device path and finger.
    ///
    /// # Errors
    /// `FprintdError` if the device cannot be reached, and
    /// `PlatformNotSupported` if `finger` is not enrolled on it.
    pub async fn with_device(device_path: String, finger: String) -> Result<Self, VaultError> {
        let connection = Connection::system()
            .await
            .map_err(|e| VaultError::FprintdError(format!("D-Bus connection failed: {e}")))?;

        // Verify the finger is enrolled
        let dev_proxy = Proxy::new(
            &connection,
            "net.reactivated.Fprint",
            device_path.as_str(),
            "net.reactivated.Fprint.Device",
        )
        .await
        .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        let fingers: Vec<String> = reply(
            dev_proxy.call_method("GetEnrolledFingers", &()).await,
            "GetEnrolledFingers",
        )?;

        if !fingers.contains(&finger) {
            return Err(VaultError::PlatformNotSupported(format!(
                "Finger '{finger}' not enrolled on device"
            )));
        }

        Ok(Self {
            device_path,
            finger,
            timeout: FPRINTD_CALL_TIMEOUT,
            connection,
        })
    }

    /// Set verification timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Verify a fingerprint, giving up after `self.timeout`.
    ///
    /// # Errors
    /// `FprintdError` if the device stops answering. A finger that does not
    /// match is not an error -- it is `FingerprintResult::Failed`, because the
    /// caller has to tell "wrong finger" from "no reader" to decide what to
    /// offer the user next.
    pub async fn verify(&self) -> Result<FingerprintResult, VaultError> {
        let dev_proxy = Proxy::new(
            &self.connection,
            "net.reactivated.Fprint",
            self.device_path.as_str(),
            "net.reactivated.Fprint.Device",
        )
        .await
        .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        // Start verification
        dev_proxy
            .call_method("VerifyStart", &(self.finger.clone()))
            .await
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
            let status: String = reply(dev_proxy.call_method("GetStatus", &()).await, "GetStatus")?;

            match status.as_str() {
                "verify-match" => break FingerprintResult::Verified,
                // A finger that did not match and a scan that errored are the
                // same outcome to the caller: this attempt did not open it.
                "verify-no-match" | "verify-fail" => break FingerprintResult::Failed,
                "verify-finger-not-set" => break FingerprintResult::NotEnrolled,
                "verify-retry-scan" => {
                    // Finger moved or bad scan - continue polling for retry
                    continue;
                }
                _ => {}
            }

            // Small delay before polling again
            tokio::time::sleep(FPRINTD_POLL_INTERVAL).await;
        };

        // Always stop verification on completion (match, fail, timeout, not enrolled)
        let _ = dev_proxy.call_method("VerifyStop", &()).await;
        Ok(result)
    }

    /// Quick check if fingerprint is available
    pub async fn is_available() -> bool {
        Self::new().await.is_ok()
    }

    /// List enrolled fingers on the device this verifier was built for.
    ///
    /// # Errors
    /// `FprintdError` if the device cannot be reached or answers with
    /// something that is not a list of names.
    pub async fn list_fingers(&self) -> Result<Vec<String>, VaultError> {
        let dev_proxy = Proxy::new(
            &self.connection,
            "net.reactivated.Fprint",
            self.device_path.as_str(),
            "net.reactivated.Fprint.Device",
        )
        .await
        .map_err(|e| VaultError::FprintdError(format!("Device proxy failed: {e}")))?;

        let fingers: Vec<String> = reply(
            dev_proxy.call_method("GetEnrolledFingers", &()).await,
            "GetEnrolledFingers",
        )?;

        Ok(fingers)
    }
}

/// Check if fprintd is available and has enrolled fingers.
///
/// # Errors
/// Whatever `FingerprintVerifier::new` and `list_fingers` return: there is no
/// device, or it cannot be reached.
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
    /// Create a new fingerprint verifier.
    ///
    /// # Errors
    /// Always returns `VaultError::PlatformNotSupported` on non-Linux platforms.
    pub fn new() -> impl std::future::Future<Output = Result<Self, VaultError>> {
        std::future::ready(Err(VaultError::PlatformNotSupported(
            "fprintd only on Linux".into(),
        )))
    }

    /// Create a verifier for a specific device and finger.
    ///
    /// # Errors
    /// Always returns `VaultError::PlatformNotSupported` on non-Linux platforms.
    pub fn with_device(
        _device_path: String,
        _finger: String,
    ) -> impl std::future::Future<Output = Result<Self, VaultError>> {
        std::future::ready(Err(VaultError::PlatformNotSupported(
            "fprintd only on Linux".into(),
        )))
    }

    /// Verify a fingerprint.
    ///
    /// # Errors
    /// Always returns `VaultError::PlatformNotSupported` on non-Linux platforms.
    pub fn verify(
        &self,
    ) -> impl std::future::Future<Output = Result<FingerprintResult, VaultError>> {
        std::future::ready(Err(VaultError::PlatformNotSupported(
            "fprintd only on Linux".into(),
        )))
    }

    #[must_use]
    pub const fn with_timeout(self, _timeout: Duration) -> Self {
        self
    }

    pub fn is_available() -> impl std::future::Future<Output = bool> {
        std::future::ready(false)
    }

    /// List enrolled fingers.
    ///
    /// # Errors
    /// Always returns `VaultError::PlatformNotSupported` on non-Linux platforms.
    pub fn list_fingers(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<String>, VaultError>> {
        std::future::ready(Err(VaultError::PlatformNotSupported(
            "fprintd only on Linux".into(),
        )))
    }
}

/// Check if fprintd is available and list enrolled fingers.
///
/// # Errors
/// Always returns `VaultError::PlatformNotSupported` on non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn check_fprintd() -> impl std::future::Future<Output = Result<Vec<String>, VaultError>> {
    std::future::ready(Err(VaultError::PlatformNotSupported(
        "fprintd only on Linux".into(),
    )))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
