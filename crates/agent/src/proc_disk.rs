//! Disk usage: the root filesystem, asked through `statvfs`.
//!
//! Split out of `proc` for length, along one of that module's real seams --
//! this is the only part of it that calls `statvfs` rather than parsing a
//! `/proc` pseudofile. It once read all of `/proc/self/mountinfo` every tick
//! to find the row mounted at `/` and then asked about `/` -- the answer was
//! the question (90 KB a tick on .33, roadmap M2), so it asks about `/`. The names stay reachable as `proc::*` (re-exported
//! there) because that is the path every caller and test already uses.

use crate::proc::Usage;

#[must_use]
pub fn statvfs_bytes(path: &str) -> Option<(u64, u64)> {
    let c_path = std::ffi::CString::new(path).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &raw mut st) != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        // `fsblkcnt_t` is u32 on macOS and u64 on Linux: widened on one,
        // taken as-is on the other.
        #[cfg(target_os = "macos")]
        let (blocks, bavail) = (u64::from(st.f_blocks), u64::from(st.f_bavail));
        #[cfg(not(target_os = "macos"))]
        let (blocks, bavail) = (st.f_blocks, st.f_bavail);
        Some((blocks * frsize, bavail * frsize))
    }
}

#[must_use]
pub fn get_disk() -> Usage {
    if let Some((total, free)) = statvfs_bytes("/") {
        Usage::new(total, total.saturating_sub(free))
    } else {
        Usage::default()
    }
}
