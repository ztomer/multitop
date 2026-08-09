//! The `/proc` readers, driven from fixture files.
//!
//! This is the path every deployed agent runs and no developer machine has:
//! the agent is built on macOS and run on Linux, so without fixtures the read
//! side of `/proc/stat`, `/proc/meminfo`, `/proc/net/dev` and
//! `/proc/self/mountinfo` is never executed until it is in production.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use multitop_agent::proc;

/// A fixture file that is removed when the test ends.
struct Fixture(PathBuf);

impl Fixture {
    fn new(tag: &str, body: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "multitop-proc-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, body).unwrap();
        Fixture(path)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const PROC_STAT: &str = "\
cpu  1000 20 300 8000 100 0 40 0 0 0
cpu0 500 10 150 4000 50 0 20 0 0 0
cpu1 500 10 150 4000 50 0 20 0 0 0
intr 12345 1 2 3
ctxt 987654
btime 1700000000
";

const MEMINFO: &str = "\
MemTotal:       16384000 kB
MemFree:         2048000 kB
MemAvailable:   10000000 kB
Buffers:          512000 kB
Cached:          4096000 kB
SwapTotal:       2048000 kB
";

const NET_DEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets
    lo: 123456     100    0    0    0     0          0         0   123456     100
  eth0: 900000    1500    0    0    0     0          0         0   450000     900
  eth1: 100000     200    0    0    0     0          0         0    50000     100
";

const MOUNTINFO: &str = "\
23 28 0:21 / /proc rw,nosuid,nodev,noexec shared:12 - proc proc rw
25 28 0:23 / /dev rw,nosuid shared:2 - devtmpfs udev rw
28 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
30 28 259:1 / /boot rw,relatime shared:14 - vfat /dev/nvme0n1p1 rw
";

// ------------------------------------------------------------------ /proc/stat

#[test]
fn proc_stat_is_read_and_parsed_into_aggregate_and_cores() {
    let f = Fixture::new("stat", PROC_STAT);
    let stat = proc::cpu_stat_from(f.path()).expect("a well-formed /proc/stat must parse");

    // Aggregate: every column summed; idle is columns 4 and 5 together.
    assert_eq!(stat.aggregate.total, (1000 + 20 + 300 + 8000 + 100) + 40);
    assert_eq!(stat.aggregate.idle, 8000 + 100);

    assert_eq!(
        stat.cores.len(),
        2,
        "intr/ctxt/btime lines must not become cores"
    );
    assert_eq!(stat.core(0).unwrap().idle, 4000 + 50);
    assert_eq!(
        stat.core(1).unwrap().total,
        (500 + 10 + 150 + 4000 + 50) + 20
    );
    assert_eq!(stat.core(9), None);
}

#[test]
fn cores_come_back_in_index_order_however_the_file_lists_them() {
    let f = Fixture::new(
        "stat-order",
        "cpu2 1 1 1 1 1\ncpu0 1 1 1 1 1\ncpu1 1 1 1 1 1\n",
    );
    let stat = proc::cpu_stat_from(f.path()).unwrap();
    assert_eq!(
        stat.cores.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn a_cpu_line_without_an_idle_column_is_unusable_and_skipped() {
    // Fewer than five columns means idle and iowait are not there, so the
    // busy percentage would be meaningless.
    let f = Fixture::new("stat-short", "cpu  1 2 3\ncpu0 1 2 3 4 5\n");
    let stat = proc::cpu_stat_from(f.path()).unwrap();
    assert_eq!(
        stat.aggregate,
        Default::default(),
        "the short line was used anyway"
    );
    assert_eq!(stat.cores.len(), 1);
}

#[test]
fn a_cpu_line_with_a_non_numeric_column_is_skipped() {
    let f = Fixture::new("stat-junk", "cpu  1 2 three 4 5\ncpu0 1 2 3 4 5\n");
    let stat = proc::cpu_stat_from(f.path()).unwrap();
    assert_eq!(stat.aggregate, Default::default());
    assert_eq!(stat.cores.len(), 1);
}

#[test]
fn an_absent_or_empty_proc_stat_sends_the_caller_elsewhere() {
    assert_eq!(proc::cpu_stat_from("/no/such/proc/stat"), None);
    let empty = Fixture::new("stat-empty", "");
    assert_eq!(proc::cpu_stat_from(empty.path()), None);
    // Present but with nothing the parser recognises is the same as absent:
    // returning an all-zero CpuStat would report a permanently idle host.
    let junk = Fixture::new("stat-nocpu", "intr 1 2 3\nctxt 4\n");
    assert_eq!(proc::cpu_stat_from(junk.path()), None);
}

// --------------------------------------------------------------- /proc/meminfo

#[test]
fn meminfo_is_read_and_used_excludes_what_the_kernel_can_reclaim() {
    let f = Fixture::new("meminfo", MEMINFO);
    let mem = proc::memory_from(f.path()).expect("a well-formed meminfo must parse");

    assert_eq!(mem.total, 16_384_000 * 1024);
    // used = total - (free + buffers + cached)
    let reclaimable = (2_048_000u64 + 512_000 + 4_096_000) * 1024;
    assert_eq!(mem.used, mem.total - reclaimable);
    assert!(mem.pct > 0.0 && mem.pct < 100.0);
}

#[test]
fn meminfo_ignores_the_fields_it_does_not_use() {
    // SwapTotal and MemAvailable must not be mistaken for any of the four
    // fields that make up the figure.
    let f = Fixture::new(
        "meminfo-only-total",
        "MemTotal: 1024 kB\nSwapTotal: 999999 kB\n",
    );
    let mem = proc::memory_from(f.path()).unwrap();
    assert_eq!(mem.total, 1024 * 1024);
    assert_eq!(
        mem.used, mem.total,
        "nothing reclaimable means all of it is used"
    );
}

#[test]
fn a_meminfo_that_reports_no_total_sends_the_caller_elsewhere() {
    assert_eq!(proc::memory_from("/no/such/meminfo"), None);
    let empty = Fixture::new("meminfo-empty", "");
    assert_eq!(proc::memory_from(empty.path()), None);
    let no_total = Fixture::new("meminfo-nototal", "MemFree: 100 kB\n");
    assert_eq!(proc::memory_from(no_total.path()), None);
    // Lines with no colon, and values that are not numbers, are skipped
    // rather than aborting the parse.
    let junk = Fixture::new(
        "meminfo-junk",
        "garbage\nMemTotal: lots kB\nMemFree: 1 kB\n",
    );
    assert_eq!(proc::memory_from(junk.path()), None);
}

// -------------------------------------------------------------- /proc/net/dev

#[test]
fn net_dev_sums_the_real_interfaces_and_ignores_loopback() {
    let f = Fixture::new("netdev", NET_DEV);
    let net = proc::net_from(f.path()).expect("a well-formed net/dev must parse");
    // eth0 + eth1, with lo left out — counting loopback would double every
    // byte the host sent to itself.
    assert_eq!(net.rx, 900_000 + 100_000);
    assert_eq!(net.tx, 450_000 + 50_000);
}

#[test]
fn a_net_dev_with_nothing_but_loopback_sends_the_caller_elsewhere() {
    let f = Fixture::new("netdev-lo", "hdr1\nhdr2\n    lo: 1 2 3 4 5 6 7 8 9 10\n");
    assert_eq!(proc::net_from(f.path()), None);
    assert_eq!(proc::net_from("/no/such/net/dev"), None);
}

#[test]
fn malformed_net_dev_rows_are_skipped_rather_than_aborting_the_sum() {
    // A row with no colon, and a row that stops before the transmit column.
    let body = "hdr1\nhdr2\nnot-an-interface-row\n  eth9: 7\n  eth0: 500 1 0 0 0 0 0 0 250 1\n";
    let f = Fixture::new("netdev-mixed", body);
    let net = proc::net_from(f.path()).unwrap();
    assert_eq!(net.rx, 500);
    assert_eq!(net.tx, 250);
}

// -------------------------------------------------------- /proc/self/mountinfo

#[test]
fn the_root_mount_is_found_among_the_others() {
    let f = Fixture::new("mountinfo", MOUNTINFO);
    assert_eq!(proc::root_mount_from(f.path()).as_deref(), Some("/"));
}

#[test]
fn a_mountinfo_without_a_root_row_names_no_mount() {
    let f = Fixture::new("mountinfo-noroot", "23 28 0:21 / /proc rw - proc proc rw\n");
    assert_eq!(proc::root_mount_from(f.path()), None);
    assert_eq!(proc::root_mount_from("/no/such/mountinfo"), None);
    // Rows too short to have a mount-point column are skipped.
    let short = Fixture::new("mountinfo-short", "23 28\n");
    assert_eq!(proc::root_mount_from(short.path()), None);
}

// ------------------------------------------------------------- non-utf8 bytes

#[test]
fn a_pseudofile_with_invalid_utf8_is_read_lossily_rather_than_failing() {
    let path = std::env::temp_dir().join(format!("multitop-proc-binary-{}", std::process::id()));
    std::fs::write(&path, [b'o', b'k', 0xff, 0xfe, b'!']).unwrap();

    // `read_proc_into` replaces what it cannot decode; a hard failure here
    // would lose a whole frame over one bad byte.
    let s = proc::read_proc(&path);
    assert!(
        s.starts_with("ok"),
        "the readable prefix must survive: {s:?}"
    );
    assert!(s.ends_with('!'));

    std::fs::remove_file(&path).unwrap();
}
