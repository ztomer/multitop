//! Which process groups make up a session, for ending all of them.
//!
//! Split from `pty` because the question is the process table's, not the
//! terminal's: the terminal knows one group, its foreground one, and that is
//! not the group the shell is in (M.REAPFLAKE, see `pty::terminate`).

/// Every process group with a member in session `sid`, the session's own first.
///
/// A session outlives its leader for as long as a member does, and its id is
/// not reused while it does, so this is right both before the leader is reaped
/// and after.
#[must_use]
pub fn groups(sid: libc::pid_t) -> Vec<libc::pid_t> {
    let mut groups = vec![sid];
    for pid in all_pids() {
        // SAFETY: `getsid` and `getpgid` only read; a pid that has gone since
        // the listing answers -1, which matches nothing.
        if pid > 0 && unsafe { libc::getsid(pid) } == sid {
            let g = unsafe { libc::getpgid(pid) };
            if g > 0 && !groups.contains(&g) {
                groups.push(g);
            }
        }
    }
    groups
}

/// Every pid in the process table, from `/proc`.
#[cfg(not(target_os = "macos"))]
fn all_pids() -> Vec<libc::pid_t> {
    std::fs::read_dir("/proc").map_or_else(
        |_| Vec::new(),
        |dir| {
            dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
                .collect()
        },
    )
}

/// Every pid in the process table, from the kernel's list.
///
/// `proc_listallpids` answers a COUNT of pids, not a byte length, both when
/// asked for the size and when filling the buffer.
#[cfg(target_os = "macos")]
fn all_pids() -> Vec<libc::pid_t> {
    // SAFETY: a null buffer of length 0 asks only for the count.
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Ok(n) = usize::try_from(n) else {
        return Vec::new();
    };
    let mut pids: Vec<libc::pid_t> = vec![0; n + 64];
    let bytes = i32::try_from(pids.len() * std::mem::size_of::<libc::pid_t>()).unwrap_or(i32::MAX);
    // SAFETY: `pids` is a live buffer of exactly `bytes` bytes.
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(usize::try_from(got).unwrap_or(0).min(pids.len()));
    pids
}
