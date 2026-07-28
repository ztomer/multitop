use multitop_agent::proc::*;

#[test]
fn read_proc_missing_is_empty() {
    assert_eq!(read_proc("/nonexistent/path/xyz"), "");
}

#[test]
fn read_proc_empty_file_is_empty() {
    assert_eq!(read_proc("/dev/null"), "");
}

#[test]
fn proc_stat_empty() {
    let stat = parse_proc_stat("");
    assert_eq!(stat.aggregate, CpuTimes::default());
    assert!(stat.cores.is_empty());
}

#[test]
fn proc_stat_aggregate_only() {
    let stat = parse_proc_stat("cpu  100 20 30 40 50 60 70 80 90 100");
    assert_eq!(
        stat.aggregate.total,
        100 + 20 + 30 + 40 + 50 + 60 + 70 + 80 + 90 + 100
    );
    assert_eq!(stat.aggregate.idle, 90);
    assert!(stat.cores.is_empty());
}

#[test]
fn proc_stat_with_cores() {
    let data = "cpu  100 20 30 40 50 60 70\n\
                cpu0 80 10 20 30 0 0 0\n\
                cpu1 60 15 10 20 0 0 0\n";
    let stat = parse_proc_stat(data);
    assert_eq!(stat.cores.len(), 2);
    assert_eq!(stat.core(0).unwrap().total, 80 + 10 + 20 + 30);
    assert_eq!(stat.core(0).unwrap().idle, 30);
    assert_eq!(stat.core(1).unwrap().total, 60 + 15 + 10 + 20);
    assert_eq!(stat.core(1).unwrap().idle, 20);
}

#[test]
fn proc_stat_ignores_non_cpu_lines() {
    let data = "cpu  100 20 30 40 50 60 70 80 90 100\n\
                intr 100 200 300\n\
                ctxt 5000\n";
    let stat = parse_proc_stat(data);
    assert!(stat.cores.is_empty());
    assert!(stat.aggregate.total > 0);
}

#[test]
fn proc_stat_ignores_short_and_garbage_lines() {
    let stat = parse_proc_stat("cpu 1 2 3\ncpu0 a b c d e\ncpufreq 1 2 3 4 5\n");
    assert_eq!(stat.aggregate, CpuTimes::default());
    assert!(stat.cores.is_empty());
}

#[test]
fn proc_stat_sorts_cores_by_index() {
    let stat = parse_proc_stat("cpu10 1 1 1 1 1\ncpu2 1 1 1 1 1\ncpu1 1 1 1 1 1\n");
    let idx: Vec<usize> = stat.cores.iter().map(|(i, _)| *i).collect();
    assert_eq!(idx, vec![1, 2, 10]);
}

#[test]
fn cpu_pct_from_deltas() {
    let prev = CpuTimes {
        total: 1000,
        idle: 800,
    };
    let curr = CpuTimes {
        total: 2000,
        idle: 1400,
    };
    assert!((curr.pct_since(&prev) - 40.0).abs() < 1e-9);
}

#[test]
fn cpu_pct_zero_window() {
    let t = CpuTimes {
        total: 100,
        idle: 50,
    };
    assert_eq!(t.pct_since(&t), 0.0);
}

#[test]
fn cpu_pct_survives_counter_reset() {
    let prev = CpuTimes {
        total: 5000,
        idle: 4000,
    };
    let curr = CpuTimes {
        total: 100,
        idle: 50,
    };
    let pct = curr.pct_since(&prev);
    assert!(pct.is_finite() && (0.0..=100.0).contains(&pct));
}

#[test]
fn meminfo_basic() {
    let data = "MemTotal: 8000000 kB\n\
                MemFree: 2000000 kB\n\
                Buffers: 500000 kB\n\
                Cached: 3000000 kB\n";
    let u = parse_meminfo(data);
    assert_eq!(u.total, 8_000_000 * 1024);
    assert_eq!(u.used, (8_000_000 - 2_000_000 - 500_000 - 3_000_000) * 1024);
    assert!(u.pct > 0.0);
}

#[test]
fn meminfo_empty() {
    assert_eq!(parse_meminfo(""), Usage::default());
}

#[test]
fn meminfo_skips_lines_without_colon() {
    let u = parse_meminfo("MemTotal: 4000000 kB\nsome junk line\nMemFree: 2000000 kB\n");
    assert_eq!(u.total, 4_000_000 * 1024);
}

#[test]
fn meminfo_never_underflows() {
    let u = parse_meminfo("MemTotal: 100 kB\nMemFree: 500 kB\n");
    assert_eq!(u.used, 0);
    assert_eq!(u.pct, 0.0);
}

#[test]
fn root_mount_found() {
    let data = "1 2 3 4 / - ext4 /dev/sda1 rw,relatime\n";
    assert_eq!(root_mount_point(data), Some("/"));
}

#[test]
fn root_mount_absent() {
    assert_eq!(
        root_mount_point("1 2 3 4 /subdir - ext4 /dev/sda1 rw\n"),
        None
    );
    assert_eq!(root_mount_point(""), None);
    assert_eq!(root_mount_point("too few\n"), None);
}

#[test]
fn root_mount_skips_non_root_first() {
    let data = "1 2 3 / /boot - ext4 /dev/sda1 rw\n2 3 4 / / - ext4 /dev/sda2 rw\n";
    assert_eq!(root_mount_point(data), Some("/"));
}

#[test]
fn net_dev_sums_non_loopback() {
    let data = "Inter-|   Receive\n\
                face |bytes    packets\n\
                eth0: 1000000 1000 0 0 0 0 0 0 2000000 2000\n\
                lo: 500000 500 0 0 0 0 0 0 500000 500\n";
    let n = parse_net_dev(data);
    assert_eq!(n.rx, 1_000_000);
    assert_eq!(n.tx, 2_000_000);
}

#[test]
fn net_dev_ignores_loopback_only() {
    let data = "h1\nh2\n  lo: 999999 500 0 0 0 0 0 0 999999 500\n";
    assert_eq!(parse_net_dev(data), NetTotals::default());
}

#[test]
fn net_dev_empty() {
    assert_eq!(parse_net_dev(""), NetTotals::default());
}

#[test]
fn net_dev_sums_multiple_interfaces() {
    let data = "h1\nh2\n\
                eth0: 100 1 0 0 0 0 0 0 200 2\n\
                wlan0: 300 3 0 0 0 0 0 0 400 4\n";
    let n = parse_net_dev(data);
    assert_eq!(n.rx, 400);
    assert_eq!(n.tx, 600);
}

#[test]
fn net_dev_handles_glued_name_and_counter() {
    let data = "h1\nh2\nenp0s31f6:12345678 1 0 0 0 0 0 0 87654321 2\n";
    let n = parse_net_dev(data);
    assert_eq!(n.rx, 12_345_678);
    assert_eq!(n.tx, 87_654_321);
}

#[test]
fn net_dev_only_skips_exact_loopback() {
    let data = "h1\nh2\nloom: 10 1 0 0 0 0 0 0 20 2\n";
    let n = parse_net_dev(data);
    assert_eq!(n.rx, 10);
    assert_eq!(n.tx, 20);
}

#[test]
fn net_dev_skips_truncated_rows() {
    assert_eq!(parse_net_dev("h1\nh2\neth0: 1 2 3\n"), NetTotals::default());
}

fn stat_line(pid: u32, comm: &str, utime: u64, stime: u64, start: u64, rss: u64) -> String {
    let mut t = vec!["0".to_string(); 22];
    t[0] = "S".into();
    t[11] = utime.to_string();
    t[12] = stime.to_string();
    t[19] = start.to_string();
    t[21] = rss.to_string();
    format!("{pid} ({comm}) {}", t.join(" "))
}

#[test]
fn pid_stat_field_offsets_match_proc5() {
    let fields: Vec<String> = (4..=30).map(|n| n.to_string()).collect();
    let line = format!("1234 (bash) S {}", fields.join(" "));
    let s = parse_pid_stat(&line).unwrap();
    assert_eq!(s.pid, 1234);
    assert_eq!(s.comm, "bash");
    assert_eq!(s.ticks, 14 + 15, "utime is field 14, stime is field 15");
    assert_eq!(s.starttime, 22, "starttime is field 22");
    assert_eq!(s.rss_pages, 24, "rss is field 24");
}

#[test]
fn pid_stat_basic() {
    let s = parse_pid_stat(&stat_line(1234, "python3", 100, 50, 900, 256)).unwrap();
    assert_eq!(s.pid, 1234);
    assert_eq!(s.comm, "python3");
    assert_eq!(s.ticks, 150);
    assert_eq!(s.starttime, 900);
    assert_eq!(s.rss_pages, 256);
}

#[test]
fn pid_stat_comm_with_parens_and_spaces() {
    let s = parse_pid_stat(&stat_line(7, "my (weird) proc", 1, 2, 3, 4)).unwrap();
    assert_eq!(s.comm, "my (weird) proc");
    assert_eq!(s.ticks, 3);
    assert_eq!(s.rss_pages, 4);
}

#[test]
fn pid_stat_rejects_malformed() {
    assert!(parse_pid_stat("").is_none());
    assert!(parse_pid_stat("1234 no-parens S 0 0").is_none());
    assert!(parse_pid_stat("1234 (short) S 1 2 3").is_none());
    assert!(parse_pid_stat("notapid (x) S 0 0").is_none());
}

#[test]
fn pid_stat_empty_comm() {
    let s = parse_pid_stat(&stat_line(9, "", 5, 5, 1, 1)).unwrap();
    assert_eq!(s.comm, "");
    assert_eq!(s.ticks, 10);
}

#[test]
fn host_info_uses_supplied_ip() {
    let r = host_info(Some("192.168.0.33"));
    assert!(r.contains("192.168.0.33"), "{r}");
    assert!(r.ends_with(')'), "{r}");
}

#[test]
fn host_info_ignores_empty_ip() {
    assert!(!host_info(Some("")).contains("()"));
}

#[test]
fn hostname_is_non_empty() {
    assert!(!hostname().is_empty());
}
