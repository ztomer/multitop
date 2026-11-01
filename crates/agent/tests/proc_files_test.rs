//! The `/proc` readers, driven from fixture files.
//!
//! This is the path every deployed agent runs and no developer machine has:
//! the agent is built on macOS and run on Linux, so without fixtures the read
//! side of `/proc/stat`, `/proc/meminfo`, and `/proc/net/dev`
//! is never executed until it is in production.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

// Integration-test crate: helper fns outside #[test] are not covered by
// clippy.toml's test exemption, so the restriction lints are expected here.
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use multitop_agent::proc;

/// A fixture file that is removed when the test ends.
struct Fixture(PathBuf);

impl Fixture {
    /// Write the fixture, or say why the filesystem refused.
    fn new(tag: &str, body: &str) -> std::io::Result<Self> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "multitop-proc-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, body)?;
        Ok(Self(path))
    }
    /// The path as text; a temp dir on this machine is UTF-8, and one that
    /// is not is reported rather than guessed at.
    fn path(&self) -> Result<&str, String> {
        self.0
            .to_str()
            .ok_or_else(|| format!("fixture path is not UTF-8: {}", self.0.display()))
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

// ------------------------------------------------------------------ /proc/stat

#[test]
fn proc_stat_is_read_and_parsed_into_aggregate_and_cores() {
    let f = Fixture::new("stat", PROC_STAT).expect("fixture written");
    let stat = proc::cpu_stat_from(f.path().expect("utf-8 path"))
        .expect("a well-formed /proc/stat must parse");

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
    )
    .expect("fixture written");
    let stat = proc::cpu_stat_from(f.path().expect("utf-8 path")).unwrap();
    assert_eq!(
        stat.cores.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn a_cpu_line_without_an_idle_column_is_unusable_and_skipped() {
    // Fewer than five columns means idle and iowait are not there, so the
    // busy percentage would be meaningless.
    let f = Fixture::new("stat-short", "cpu  1 2 3\ncpu0 1 2 3 4 5\n").expect("fixture written");
    let stat = proc::cpu_stat_from(f.path().expect("utf-8 path")).unwrap();
    assert_eq!(
        stat.aggregate,
        proc::CpuTimes::default(),
        "the short line was used anyway"
    );
    assert_eq!(stat.cores.len(), 1);
}

#[test]
fn a_cpu_line_with_a_non_numeric_column_is_skipped() {
    let f =
        Fixture::new("stat-junk", "cpu  1 2 three 4 5\ncpu0 1 2 3 4 5\n").expect("fixture written");
    let stat = proc::cpu_stat_from(f.path().expect("utf-8 path")).unwrap();
    assert_eq!(stat.aggregate, proc::CpuTimes::default());
    assert_eq!(stat.cores.len(), 1);
}

#[test]
fn an_absent_or_empty_proc_stat_sends_the_caller_elsewhere() {
    assert_eq!(proc::cpu_stat_from("/no/such/proc/stat"), None);
    let empty = Fixture::new("stat-empty", "").expect("fixture written");
    assert_eq!(proc::cpu_stat_from(empty.path().expect("utf-8 path")), None);
    // Present but with nothing the parser recognises is the same as absent:
    // returning an all-zero CpuStat would report a permanently idle host.
    let junk = Fixture::new("stat-nocpu", "intr 1 2 3\nctxt 4\n").expect("fixture written");
    assert_eq!(proc::cpu_stat_from(junk.path().expect("utf-8 path")), None);
}

// --------------------------------------------------------------- /proc/meminfo

#[test]
fn meminfo_is_read_and_used_excludes_what_the_kernel_can_reclaim() {
    let f = Fixture::new("meminfo", MEMINFO).expect("fixture written");
    let mem =
        proc::memory_from(f.path().expect("utf-8 path")).expect("a well-formed meminfo must parse");

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
    )
    .expect("fixture written");
    let mem = proc::memory_from(f.path().expect("utf-8 path")).unwrap();
    assert_eq!(mem.total, 1024 * 1024);
    assert_eq!(
        mem.used, mem.total,
        "nothing reclaimable means all of it is used"
    );
}

#[test]
fn a_meminfo_that_reports_no_total_sends_the_caller_elsewhere() {
    assert_eq!(proc::memory_from("/no/such/meminfo"), None);
    let empty = Fixture::new("meminfo-empty", "").expect("fixture written");
    assert_eq!(proc::memory_from(empty.path().expect("utf-8 path")), None);
    let no_total = Fixture::new("meminfo-nototal", "MemFree: 100 kB\n").expect("fixture written");
    assert_eq!(
        proc::memory_from(no_total.path().expect("utf-8 path")),
        None
    );
    // Lines with no colon, and values that are not numbers, are skipped
    // rather than aborting the parse.
    let junk = Fixture::new(
        "meminfo-junk",
        "garbage\nMemTotal: lots kB\nMemFree: 1 kB\n",
    )
    .expect("fixture written");
    assert_eq!(proc::memory_from(junk.path().expect("utf-8 path")), None);
}

// -------------------------------------------------------------- /proc/net/dev

#[test]
fn net_dev_sums_the_real_interfaces_and_ignores_loopback() {
    let f = Fixture::new("netdev", NET_DEV).expect("fixture written");
    let net =
        proc::net_from(f.path().expect("utf-8 path")).expect("a well-formed net/dev must parse");
    // eth0 + eth1, with lo left out — counting loopback would double every
    // byte the host sent to itself.
    assert_eq!(net.rx, 900_000 + 100_000);
    assert_eq!(net.tx, 450_000 + 50_000);
}

#[test]
fn a_net_dev_with_nothing_but_loopback_sends_the_caller_elsewhere() {
    let f = Fixture::new("netdev-lo", "hdr1\nhdr2\n    lo: 1 2 3 4 5 6 7 8 9 10\n")
        .expect("fixture written");
    assert_eq!(proc::net_from(f.path().expect("utf-8 path")), None);
    assert_eq!(proc::net_from("/no/such/net/dev"), None);
}

#[test]
fn malformed_net_dev_rows_are_skipped_rather_than_aborting_the_sum() {
    // A row with no colon, and a row that stops before the transmit column.
    let body = "hdr1\nhdr2\nnot-an-interface-row\n  eth9: 7\n  eth0: 500 1 0 0 0 0 0 0 250 1\n";
    let f = Fixture::new("netdev-mixed", body).expect("fixture written");
    let net = proc::net_from(f.path().expect("utf-8 path")).unwrap();
    assert_eq!(net.rx, 500);
    assert_eq!(net.tx, 250);
}

// ------------------------------------------- files the host makes any size
//
// The size of a pseudofile is the host's to decide: `/proc/net/dev` was 5,991
// bytes on .33 (2026-11-01, roadmap M2), and the reader that held it in one
// 2 KiB `read` counted only the interfaces that fitted. So each file is read
// here at a size no fixed buffer of the old kind holds, with the line that
// matters LAST.

/// `n` interfaces of 1 rx / 2 tx bytes each, plus loopback.
fn net_dev_of(n: usize) -> String {
    let mut s = String::from(
        "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes\n",
    );
    s.push_str("    lo: 9 0 0 0 0 0 0 0 9 0 0 0 0 0 0 0\n");
    for i in 0..n {
        writeln!(s, "  veth{i:05}: 1 0 0 0 0 0 0 0 2 0 0 0 0 0 0 0").expect("String write");
    }
    s
}

#[test]
fn every_interface_is_counted_however_long_net_dev_is() {
    let f = Fixture::new("netdev-big", &net_dev_of(400)).expect("fixture written");
    let net = proc::net_from(f.path().expect("utf-8 path")).expect("parses");
    assert_eq!((net.rx, net.tx), (400, 800));
}

#[test]
fn every_core_is_read_however_long_proc_stat_is() {
    let mut body = String::from("cpu  1 1 1 1 1\n");
    for i in 0..512 {
        writeln!(body, "cpu{i} 100000 200000 300000 400000 500000 0 0 0 0 0")
            .expect("String write");
    }
    let f = Fixture::new("stat-big", &body).expect("fixture written");
    let stat = proc::cpu_stat_from(f.path().expect("utf-8 path")).expect("parses");
    assert_eq!(stat.cores.len(), 512);
    assert_eq!(stat.core(511).map(|t| t.idle), Some(400_000 + 500_000));
}

#[test]
fn meminfo_is_read_to_the_end() {
    let mut body = String::new();
    for i in 0..200 {
        writeln!(body, "Filler{i:03}:      123456 kB").expect("String write");
    }
    body.push_str("MemTotal:  1000 kB\nMemFree:  250 kB\n");
    let f = Fixture::new("meminfo-big", &body).expect("fixture written");
    let mem = proc::memory_from(f.path().expect("utf-8 path")).expect("parses");
    assert_eq!((mem.total, mem.used), (1000 * 1024, 750 * 1024));
}

#[test]
fn a_line_too_long_to_hold_is_passed_over_and_the_next_one_is_read() {
    // `/proc/stat`'s `intr` row grows with the IRQ count and nothing samples
    // it; a row of that kind must cost the rows after it nothing.
    let intr = format!("intr {}\n", "12345 ".repeat(10_000));
    let body = format!("cpu  1 1 1 1 1\ncpu0 1 1 1 1 1\n{intr}cpu1 7 7 7 7 7\nctxt 1\n");
    let f = Fixture::new("stat-longline", &body).expect("fixture written");
    let stat = proc::cpu_stat_from(f.path().expect("utf-8 path")).expect("parses");
    assert_eq!(stat.core(1).map(|t| t.total), Some(35));
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

// ------------------------------------------------------------- the core clock

/// A `/sys/devices/system/cpu` tree with one `scaling_cur_freq` per reading.
fn cpufreq_tree(readings: &[&str]) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("tempdir");
    for (i, r) in readings.iter().enumerate() {
        let dir = root.path().join(format!("cpu{i}/cpufreq"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("scaling_cur_freq"), r).expect("write");
    }
    root
}

fn scaling(readings: &[&str]) -> Option<f64> {
    let root = cpufreq_tree(readings);
    multitop_agent::cpufreq::scaling_mhz_under(root.path().to_str().expect("utf-8"))
}

#[test]
fn a_cpufreq_reading_is_averaged_across_the_cores_that_report_one() {
    // The mean, not core 0: on a big.LITTLE machine core 0 is an efficiency
    // core and reporting it as "the" clock understates the box by half.
    let mhz = scaling(&["1200000\n", "3600000\n"]).expect("a clock");
    assert!((mhz - 2400.0).abs() < 0.01, "{mhz}");
}

#[test]
fn a_cpufreq_file_that_says_nothing_usable_yields_no_clock() {
    // A zero is not a measurement of an idle core; it is the absence of one,
    // and drawing it as 0 MHz would be inventing a reading.
    for readings in [&[][..], &["0\n"], &["x\n"]] {
        assert_eq!(scaling(readings), None, "{readings:?}");
    }
}

#[test]
fn cpuinfo_is_the_fallback_when_cpufreq_is_not_there() {
    let cpuinfo = "\
processor\t: 0
model name\t: AMD EPYC 7763
cpu MHz\t\t: 2445.406
processor\t: 1
model name\t: AMD EPYC 7763
cpu MHz\t\t: 2445.406
";
    let f = Fixture::new("cpuinfo", cpuinfo).expect("fixture written");
    let mhz =
        multitop_agent::cpufreq::cpuinfo_mhz_from(f.path().expect("utf-8 path")).expect("a clock");
    assert!((mhz - 2445.406).abs() < 0.01, "{mhz}");
}

#[test]
fn an_arm_cpuinfo_with_no_clock_field_yields_none() {
    // Many ARM kernels omit `cpu MHz` entirely. The panel says so rather than
    // showing a zero.
    let cpuinfo = "processor\t: 0\nBogoMIPS\t: 50.00\nFeatures\t: fp asimd\n";
    let f = Fixture::new("cpuinfo-arm", cpuinfo).expect("fixture written");
    assert_eq!(
        multitop_agent::cpufreq::cpuinfo_mhz_from(f.path().expect("utf-8 path")),
        None
    );
}
