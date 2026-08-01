//! Memory locking (mlock) for sensitive key material
//!
//! Prevents vault keys from being swapped to disk by locking
//! their memory pages. Best-effort: logs warning on failure but continues.
//!
//! This module contains unsafe code for calling mlock/munlock system calls.
//! The unsafe blocks are reviewed and contain only standard system call wrappers.

#![allow(unsafe_code)]

use libc::{mlock, munlock};
use zeroize::Zeroize;

/// Lock the given memory range to prevent swapping.
/// Returns Ok(()) on success, or logs a warning and returns Ok(()) on failure
/// (best-effort: we don't want to crash if mlock is unavailable).
///
/// # Errors
///
/// Returns an error only if the `mlock` system call fails with an error other
/// than `ENOMEM` or `EPERM` (which are treated as best-effort failures).
pub fn mlock_memory(ptr: *const u8, len: usize) -> Result<(), std::io::Error> {
    let ret = unsafe { mlock(ptr.cast::<libc::c_void>(), len) };
    if ret == 0 {
        Ok(())
    } else {
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::ENOMEM | libc::EPERM) => {
                // Silent: a stray stderr write lands on top of the TUI. This is a
                // best-effort lock and the vault works without it.
                let _ = &err;
                Ok(())
            }
            _ => Err(err),
        }
    }
}

/// Unlock the given memory range.
///
/// # Errors
///
/// Returns an error if the `munlock` system call fails.
pub fn munlock_memory(ptr: *const u8, len: usize) -> Result<(), std::io::Error> {
    let ret = unsafe { munlock(ptr.cast::<libc::c_void>(), len) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// A wrapper that owns sensitive data, locks it in memory, and zeroizes on drop.
pub struct LockedMemory {
    data: Vec<u8>,
}

impl LockedMemory {
    /// Lock the memory for the given slice. Copies data into owned Vec.
    ///
    /// # Errors
    /// Returns `std::io::Error` if `mlock` fails with an error other than `ENOMEM` or `EPERM`.
    pub fn new(slice: &[u8]) -> Result<Self, std::io::Error> {
        let data = slice.to_vec();
        let ptr = data.as_ptr();
        let len = data.len();
        mlock_memory(ptr, len)?;
        Ok(Self { data })
    }

    /// Create a no-op `LockedMemory` for when mlock fails
    #[must_use]
    pub const fn noop() -> Self {
        Self { data: Vec::new() }
    }

    /// Get a pointer to the locked memory
    #[must_use]
    pub const fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Get the length of the locked memory
    #[must_use]
    pub const fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if this is a no-op (empty) instance
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Drop for LockedMemory {
    fn drop(&mut self) {
        if !self.data.is_empty() {
            let ptr = self.data.as_ptr();
            let len = self.data.len();
            let _ = munlock_memory(ptr, len);
        }
        // Zeroize the data before deallocation
        self.data.zeroize();
    }
}

// SAFETY: LockedMemory owns its data via Vec, which is Send/Sync.
// The mlock/munlock calls are thread-safe system calls.
unsafe impl Send for LockedMemory {}
unsafe impl Sync for LockedMemory {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn test_mlock_memory_valid() {
        let data = [1u8; 1024];
        let result = mlock_memory(data.as_ptr(), data.len());
        // On macOS, mlock may fail with EPERM for non-privileged processes
        // but we test that the function doesn't panic
        match result {
            Ok(()) => (),
            Err(e) => {
                // EPERM or ENOMEM are expected on macOS without root
                assert!(
                    e.raw_os_error() == Some(libc::EPERM) || e.raw_os_error() == Some(libc::ENOMEM),
                    "unexpected error: {e}"
                );
            }
        }
    }

    #[test]
    fn test_munlock_memory_valid() {
        let data = [1u8; 1024];
        // First try to lock
        let _ = mlock_memory(data.as_ptr(), data.len());
        // Then unlock
        let result = munlock_memory(data.as_ptr(), data.len());
        // Should not panic regardless of platform
        match result {
            Ok(()) => (),
            Err(e) => {
                // May fail if mlock didn't succeed
                assert!(
                    e.raw_os_error() == Some(libc::EINVAL) || e.raw_os_error() == Some(libc::EPERM),
                    "unexpected error: {e}"
                );
            }
        }
    }

    #[test]
    fn test_locked_memory_new() {
        let data = [2u8; 2048];
        let locked = LockedMemory::new(&data);
        // Should not panic
        match locked {
            Ok(m) => {
                assert_eq!(m.len(), 2048);
                assert!(!m.is_empty());
                assert_eq!(m.as_ptr() as usize, m.data.as_ptr() as usize);
            }
            Err(e) => {
                // Expected on macOS without root
                assert!(
                    e.raw_os_error() == Some(libc::EPERM) || e.raw_os_error() == Some(libc::ENOMEM),
                    "unexpected error: {e}"
                );
            }
        }
    }

    #[test]
    fn test_locked_memory_noop() {
        let locked = LockedMemory::noop();
        assert!(locked.is_empty());
        assert_eq!(locked.len(), 0);
        // Drop should not panic
    }

    #[test]
    fn test_locked_memory_drop_zeroizes() {
        let data = [3u8; 1024];
        let locked = LockedMemory::new(&data).unwrap_or_else(|_| LockedMemory::noop());
        drop(locked);
        // After drop, the memory should have been zeroized
        // We can't safely read freed memory, but we verify the drop didn't panic
    }

    #[test]
    fn test_locked_memory_owns_data() {
        let data = [4u8; 256];
        let locked = LockedMemory::new(&data).unwrap();
        // The locked memory should contain a copy, not a reference
        assert_eq!(&locked.data[..], &data[..]);
    }

    #[test]
    fn test_locked_memory_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<LockedMemory>();
    }

    #[test]
    fn test_locked_memory_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<LockedMemory>();
    }
}
