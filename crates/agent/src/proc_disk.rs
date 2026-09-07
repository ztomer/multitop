//! Disk usage: which filesystem to ask about, and asking it.
//!
//! Split out of `proc` for length, along one of that module's real seams --
//! this is the only part of it that calls `statvfs` rather than parsing a
//! `/proc` pseudofile. The names stay reachable as `proc::*` (re-exported
//! there) because that is the path every caller and test already uses.

#![allow(
    unsafe_code,
    reason = "`statvfs(2)` is the one syscall here; see the unsafe_code note in lib.rs"
)]

use crate::proc::{read_proc_bytes, Usage};

#[must_use]
pub fn root_mount_point(mountinfo: &str) -> Option<&str> {
    mountinfo.lines().find_map(|line| {
        let mut parts = line.split_ascii_whitespace();
        let mount_point = parts.nth(4)?;
        (mount_point == "/").then_some(mount_point)
    })
}

#[must_use]
pub fn statvfs_bytes(path: &str) -> Option<(u64, u64)> {
    let c_path = std::ffi::CString::new(path).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &raw mut st) != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        // `fsblkcnt_t` is u32 on macOS and u64 on Linux, so `u64::from` widens
        // on one and is an identity on the other. The cfg is on the lint, not
        // the code, so one expression stays correct on both.
        #[cfg_attr(
            not(target_os = "macos"),
            expect(clippy::useless_conversion, reason = "u64 already")
        )]
        let (blocks, bavail) = (u64::from(st.f_blocks), u64::from(st.f_bavail));
        Some((blocks * frsize, bavail * frsize))
    }
}

/// The mount point `statvfs` should be asked about, per `/proc/self/mountinfo`.
pub fn root_mount_from(path: &str) -> Option<String> {
    let mut buf = [0u8; crate::consts::PROC_MOUNTINFO_BUF];
    let n = read_proc_bytes(path, &mut buf);
    if n == 0 {
        return None;
    }
    root_mount_point(std::str::from_utf8(&buf[..n]).ok()?).map(str::to_string)
}

#[must_use]
pub fn get_disk() -> Usage {
    let root = root_mount_from("/proc/self/mountinfo");
    let target = root.as_deref().unwrap_or("/");
    if let Some((total, free)) = statvfs_bytes(target) {
        Usage::new(total, total.saturating_sub(free))
    } else {
        Usage::default()
    }
}
