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

/// `proc_taskinfo.pti_resident_size` is BYTES. The scan stores it as pages and
/// the caller (`proc.rs`) multiplies by `sysconf(_SC_PAGESIZE)`, so the scan
/// must divide by that same page size: Apple silicon pages are 16 KiB, and a
/// 4096 constant made every RSS 4x too large (M.PAGESIZE). Compared against
/// `ps -o rss=` (KiB) for one live, idle child.
#[test]
fn rss_matches_ps_for_a_live_process() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("sleep spawns");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let pid = child.id();
    let mut out = Vec::new();
    let mut pids = Vec::new();
    scan_macos(&mut out, &mut pids);
    let ps = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    child.kill().expect("kill");
    child.wait().expect("wait");
    let ps_bytes: u64 = String::from_utf8_lossy(&ps.stdout)
        .trim()
        .parse::<u64>()
        .expect("ps rss is a number")
        * 1024;
    // `getconf` asks the same OS value as `sysconf(_SC_PAGESIZE)`, with no `unsafe`.
    let page: u64 = String::from_utf8_lossy(
        &std::process::Command::new("getconf")
            .arg("PAGESIZE")
            .output()
            .expect("getconf runs")
            .stdout,
    )
    .trim()
    .parse()
    .expect("page size is a number");
    let ours = out
        .iter()
        .find(|p| p.pid == pid)
        .expect("the scan lists the child")
        .rss_pages
        * page;
    eprintln!("page={page} ours={ours} ps={ps_bytes}");
    assert!(
        ours.abs_diff(ps_bytes) * 20 <= ps_bytes,
        "scan RSS {ours} B vs ps {ps_bytes} B differ by more than 5% (page {page})"
    );
}
