//! Memory locking (mlock) for sensitive key material
//!
//! Prevents vault keys from being swapped to disk by locking
//! their memory pages. Best-effort: logs warning on failure but continues.
//!
//! This module contains unsafe code for calling mlock/munlock system calls.
//! The unsafe blocks are reviewed and contain only standard system call wrappers.

#![allow(unsafe_code)]

use libc::{mlock, munlock};

/// Lock the given memory range to prevent swapping.
/// Returns Ok(()) on success, or logs a warning and returns Ok(()) on failure
/// (best-effort: we don't want to crash if mlock is unavailable).
pub fn mlock_memory(ptr: *const u8, len: usize) -> Result<(), std::io::Error> {
    let ret = unsafe { mlock(ptr as *const libc::c_void, len) };
    if ret == 0 {
        Ok(())
    } else {
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::ENOMEM) | Some(libc::EPERM) => {
                eprintln!("vault: mlock failed ({}), continuing without memory lock", err);
                Ok(())
            }
            _ => Err(err),
        }
    }
}

/// Unlock the given memory range.
pub fn munlock_memory(ptr: *const u8, len: usize) -> Result<(), std::io::Error> {
    let ret = unsafe { munlock(ptr as *const libc::c_void, len) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// A wrapper that locks memory on creation and unlocks on drop.
pub struct LockedMemory {
    ptr: *const u8,
    len: usize,
}

// SAFETY: LockedMemory only uses the raw pointer for mlock/munlock calls
// which are thread-safe system calls. The pointer is valid for the lifetime
// of the LockedMemory instance.
unsafe impl Send for LockedMemory {}
unsafe impl Sync for LockedMemory {}

impl LockedMemory {
    /// Lock the memory for the given slice.
    pub fn new(slice: &[u8]) -> Result<Self, std::io::Error> {
        let ptr = slice.as_ptr();
        let len = slice.len();
        mlock_memory(ptr, len)?;
        Ok(Self { ptr, len })
    }
}

impl Drop for LockedMemory {
    fn drop(&mut self) {
        let _ = munlock_memory(self.ptr, self.len);
    }
}

// Provide a no-op fallback for the fallback case
impl LockedMemory {
    /// Create a no-op LockedMemory for when mlock fails
    pub fn noop() -> Self {
        Self { ptr: std::ptr::null(), len: 0 }
    }
}