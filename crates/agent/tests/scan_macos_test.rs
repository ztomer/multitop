//! The macOS process scan reads the whole process table (M.REAPFLAKE sibling).
//!
//! `proc_listallpids` answers a COUNT of pids, not a byte length. The scan
//! divided that count by `size_of::<i32>()` as though it were bytes, and so
//! read the first quarter of the table: 1294 pids listed, 323 read. This test
//! found 599 of this user's 868 processes missing from the scan before the fix.

// A test crate, said where clippy reads it (see exec_run_test.rs).
#![cfg(test)]
#![cfg(target_os = "macos")]

use multitop_agent::sys::scan_macos;

/// Every process this user owns is one the scan can read (`proc_pidinfo` is
/// refused only for other users' processes), so every one that is still
/// alive after the scan must be in it.
#[test]
fn the_scan_reads_every_process_this_user_owns() {
    let mut out = Vec::new();
    let mut pids = Vec::new();
    scan_macos(&mut out, &mut pids);
    let uid = String::from_utf8(
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .expect("id runs")
            .stdout,
    )
    .expect("utf-8");
    let listed = std::process::Command::new("pgrep")
        .args(["-U", uid.trim(), "."])
        .output()
        .expect("pgrep runs");
    let own: Vec<u32> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    assert!(
        own.len() >= 5,
        "calibration: pgrep listed {} own pids",
        own.len()
    );
    let scanned: std::collections::HashSet<u32> = out.iter().map(|p| p.pid).collect();
    let missing: Vec<u32> = own
        .iter()
        .copied()
        .filter(|p| !scanned.contains(p))
        .collect();
    // A process that started after the scan, or ended before `pgrep`, is the
    // only honest miss; a handful at most, never a quarter of the table.
    assert!(
        missing.len() * 20 <= own.len(),
        "the scan missed {} of {} own processes: {:?}",
        missing.len(),
        own.len(),
        &missing[..missing.len().min(20)]
    );
}
