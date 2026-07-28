//! `/proc` and `statvfs` sampling.
//!
//! Everything here is pure parsing over strings the caller supplies, except
//! for the thin `read_proc` / `statvfs` wrappers — which keeps the whole
//! module unit-testable on a host that has no `/proc` at all.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Read a `/proc` file, collapsing every failure to an empty string. `/proc`
/// entries vanish mid-read routinely (a process exits between readdir and
/// open); there is nothing useful to report for a single missing sample.
pub fn read_proc<P: AsRef<Path>>(path: P) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub total: u64,
    /// idle + iowait
    pub idle: u64,
}

impl CpuTimes {
    /// Busy percentage over the window between two samples.
    pub fn pct_since(&self, prev: &CpuTimes) -> f64 {
        let total = self.total.saturating_sub(prev.total);
        let idle = self.idle.saturating_sub(prev.idle);
        if total == 0 {
            return 0.0;
        }
        (total.saturating_sub(idle)) as f64 / total as f64 * 100.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuStat {
    pub aggregate: CpuTimes,
    /// core index -> times, kept sorted by index
    pub cores: Vec<(usize, CpuTimes)>,
}

impl CpuStat {
    pub fn core(&self, idx: usize) -> Option<CpuTimes> {
        self.cores.iter().find(|(i, _)| *i == idx).map(|(_, t)| *t)
    }
}

fn parse_cpu_line(fields: &str) -> Option<CpuTimes> {
    let mut total: u64 = 0;
    let mut vals = [0u64; 5];
    let mut n = 0;
    for tok in fields.split_ascii_whitespace() {
        let v: u64 = tok.parse().ok()?;
        total = total.saturating_add(v);
        if n < 5 {
            vals[n] = v;
        }
        n += 1;
    }
    // idle and iowait are columns 4 and 5; without them the line is unusable.
    if n < 5 {
        return None;
    }
    Some(CpuTimes {
        total,
        idle: vals[3].saturating_add(vals[4]),
    })
}

/// Parse `/proc/stat`. Non-`cpu` lines (`intr`, `ctxt`, ...) are ignored.
pub fn parse_proc_stat(data: &str) -> CpuStat {
    let mut stat = CpuStat::default();
    for line in data.lines() {
        let Some(rest) = line.strip_prefix("cpu") else {
            continue;
        };
        let Some((label, fields)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let Some(times) = parse_cpu_line(fields) else {
            continue;
        };
        if label.is_empty() {
            stat.aggregate = times;
        } else if let Ok(idx) = label.parse::<usize>() {
            stat.cores.push((idx, times));
        }
    }
    stat.cores.sort_unstable_by_key(|(i, _)| *i);
    stat
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub pct: f64,
}

impl Usage {
    fn new(total: u64, used: u64) -> Self {
        let pct = if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        Usage { total, used, pct }
    }
}

/// Parse `/proc/meminfo`. Used = total - free - buffers - cached.
pub fn parse_meminfo(data: &str) -> Usage {
    if data.is_empty() {
        return Usage::default();
    }
    let (mut total, mut free, mut buffers, mut cached) = (0u64, 0u64, 0u64, 0u64);
    for line in data.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = value
            .split_ascii_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        match key.trim() {
            "MemTotal" => total = kb,
            "MemFree" => free = kb,
            "Buffers" => buffers = kb,
            "Cached" => cached = kb,
            _ => {}
        }
    }
    let total = total * 1024;
    let reclaimable = (free + buffers + cached) * 1024;
    Usage::new(total, total.saturating_sub(reclaimable))
}

/// Mount point of the root filesystem, if `/proc/self/mountinfo` names one.
///
/// mountinfo columns are: id, parent, dev, root-within-fs, mount-point, ...
/// The Python original matched on column 4 (the mount point) but then
/// `statvfs`'d column 3 (the path inside the filesystem); that only worked
/// because the two coincide for the root mount.
pub fn root_mount_point(mountinfo: &str) -> Option<&str> {
    mountinfo.lines().find_map(|line| {
        let mut parts = line.split_ascii_whitespace();
        let mount_point = parts.nth(4)?;
        (mount_point == "/").then_some(mount_point)
    })
}

/// `statvfs(3)` wrapper returning (total, available) bytes.
pub fn statvfs_bytes(path: &str) -> Option<(u64, u64)> {
    let c_path = std::ffi::CString::new(path).ok()?;
    // SAFETY: c_path is a valid NUL-terminated string; statvfs only writes
    // into the buffer we hand it, and we check the return code before reading.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut st) != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
    }
}

pub fn get_disk() -> Usage {
    let mountinfo = read_proc("/proc/self/mountinfo");
    let Some(mount) = root_mount_point(&mountinfo) else {
        return Usage::default();
    };
    let Some((total, free)) = statvfs_bytes(mount) else {
        return Usage::default();
    };
    Usage::new(total, total.saturating_sub(free))
}

pub fn get_memory() -> Usage {
    parse_meminfo(&read_proc("/proc/meminfo"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetTotals {
    pub rx: u64,
    pub tx: u64,
}

/// Sum rx/tx byte counters across every non-loopback interface.
pub fn parse_net_dev(data: &str) -> NetTotals {
    let mut totals = NetTotals::default();
    // Two header rows.
    for line in data.lines().skip(2) {
        // Split on the interface colon rather than on whitespace: the kernel's
        // field widths run the name and the first counter together once the
        // name is long enough (`enp0s31f6:12345678`).
        let Some((iface, counters)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        // Loopback only — an interface merely *starting* with "lo" is real.
        if iface == "lo" || iface.starts_with("lo:") {
            continue;
        }
        let cols: Vec<&str> = counters.split_ascii_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        totals.rx = totals
            .rx
            .saturating_add(cols[0].parse::<u64>().unwrap_or(0));
        totals.tx = totals
            .tx
            .saturating_add(cols[8].parse::<u64>().unwrap_or(0));
    }
    totals
}

pub fn get_net() -> NetTotals {
    parse_net_dev(&read_proc("/proc/net/dev"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub cpu: f64,
    pub mem: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PidSample {
    ticks: u64,
    starttime: u64,
}

/// One process's `/proc/<pid>/stat`, reduced to what the panel shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProcStat {
    pub pid: u32,
    pub comm: String,
    pub ticks: u64,
    pub starttime: u64,
    pub rss_pages: u64,
}

/// Parse a `/proc/<pid>/stat` line.
///
/// `comm` is delimited by parentheses and may itself contain spaces and
/// parens, so the field split starts after the *last* `)`.
pub fn parse_pid_stat(data: &str) -> Option<RawProcStat> {
    let open = data.find('(')?;
    let close = data.rfind(')')?;
    if close < open {
        return None;
    }
    let pid: u32 = data[..open].trim().parse().ok()?;
    let comm = data[open + 1..close].to_string();

    // Fields from `state` (field 3) onward, so field N sits at index N-3.
    let f: Vec<&str> = data[close + 1..].split_ascii_whitespace().collect();
    if f.len() < 22 {
        return None;
    }
    let utime: u64 = f[11].parse().ok()?;
    let stime: u64 = f[12].parse().ok()?;
    let starttime: u64 = f[19].parse().ok()?;
    let rss_pages: u64 = f[21].parse().unwrap_or(0);

    Some(RawProcStat {
        pid,
        comm,
        ticks: utime.saturating_add(stime),
        starttime,
        rss_pages,
    })
}

/// Samples per-process CPU by differencing `/proc/<pid>/stat` between ticks.
///
/// The Python original shelled out to `ps -eo pcpu`, which reports a
/// process's *lifetime average* CPU — a long-lived daemon that is busy right
/// now reads near zero. Differencing gives the instantaneous figure the panel
/// implies, and skips a fork+exec per refresh.
pub struct ProcSampler {
    prev: HashMap<u32, PidSample>,
    clk_tck: f64,
    page_size: u64,
}

impl Default for ProcSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcSampler {
    pub fn new() -> Self {
        // SAFETY: sysconf with a valid name; returns -1 on failure, handled.
        let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        ProcSampler {
            prev: HashMap::new(),
            clk_tck: if clk_tck > 0 { clk_tck as f64 } else { 100.0 },
            page_size: if page_size > 0 {
                page_size as u64
            } else {
                4096
            },
        }
    }

    /// Read every `/proc/<pid>/stat` once.
    fn scan(&self) -> Vec<RawProcStat> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(256);
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.as_bytes()[0].is_ascii_digit() {
                continue;
            }
            // A process exiting between readdir and open is normal; skip it.
            let Ok(data) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            if let Some(stat) = parse_pid_stat(&data) {
                out.push(stat);
            }
        }
        out
    }

    /// Prime the baseline without producing output.
    pub fn prime(&mut self) {
        self.prev = self
            .scan()
            .into_iter()
            .map(|s| {
                (
                    s.pid,
                    PidSample {
                        ticks: s.ticks,
                        starttime: s.starttime,
                    },
                )
            })
            .collect();
    }

    /// Top `n` processes by CPU over the last `elapsed` seconds.
    pub fn top(&mut self, elapsed: f64, n: usize) -> Vec<Proc> {
        let scanned = self.scan();
        let mut procs = Vec::with_capacity(scanned.len());
        let mut next = HashMap::with_capacity(scanned.len());

        for s in &scanned {
            // starttime guards against PID reuse handing us a bogus delta.
            let cpu = match self.prev.get(&s.pid) {
                Some(p) if p.starttime == s.starttime && elapsed > 0.0 => {
                    s.ticks.saturating_sub(p.ticks) as f64 / self.clk_tck / elapsed * 100.0
                }
                _ => 0.0,
            };
            next.insert(
                s.pid,
                PidSample {
                    ticks: s.ticks,
                    starttime: s.starttime,
                },
            );
            procs.push(Proc {
                pid: s.pid,
                name: s.comm.clone(),
                cpu,
                mem: s.rss_pages * self.page_size,
            });
        }
        self.prev = next;

        // Descending CPU, then by pid so equal-CPU rows do not shuffle
        // between frames.
        procs.sort_unstable_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.pid.cmp(&b.pid))
        });
        procs.truncate(n);
        procs
    }
}

/// Hostname from `/proc`, falling back to `gethostname(2)`.
pub fn hostname() -> String {
    let from_proc = read_proc("/proc/sys/kernel/hostname").trim().to_string();
    if !from_proc.is_empty() {
        return from_proc;
    }
    let mut buf = vec![0u8; 256];
    // SAFETY: buf is a valid writable allocation of the length we pass.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return "unknown".to_string();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Primary outbound IPv4 address.
///
/// Connecting a UDP socket sends nothing — it just asks the kernel which
/// source address the default route would pick. That replaces the original's
/// fork of `ip -4 addr` and its `/proc/net/fib_trie` fallback.
pub fn primary_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:53").ok()?;
    let addr = sock.local_addr().ok()?.ip();
    if addr.is_loopback() || addr.is_unspecified() {
        return None;
    }
    Some(addr.to_string())
}

/// Panel header: `hostname (ip)`, or bare hostname when no address is known.
pub fn host_info(display_ip: Option<&str>) -> String {
    let host = hostname();
    match display_ip
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(primary_ip)
    {
        Some(ip) => format!("{host} ({ip})"),
        None => host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A counter that appears to go backwards (CPU hotplug, container
    /// restart) must not produce a negative or NaN percentage.
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

    /// The kernel's field widths run a long name into its first counter.
    #[test]
    fn net_dev_handles_glued_name_and_counter() {
        let data = "h1\nh2\nenp0s31f6:12345678 1 0 0 0 0 0 0 87654321 2\n";
        let n = parse_net_dev(data);
        assert_eq!(n.rx, 12_345_678);
        assert_eq!(n.tx, 87_654_321);
    }

    /// "lo" is loopback; "lo0"/"loom" are not, and must still be counted.
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

    /// Build a `/proc/<pid>/stat` line. Index `i` of the token list is
    /// field `i + 3`, matching the parser and `proc(5)`.
    fn stat_line(pid: u32, comm: &str, utime: u64, stime: u64, start: u64, rss: u64) -> String {
        let mut t = vec!["0".to_string(); 22];
        t[0] = "S".into(); // field 3, state
        t[11] = utime.to_string(); // field 14
        t[12] = stime.to_string(); // field 15
        t[19] = start.to_string(); // field 22
        t[21] = rss.to_string(); // field 24
        format!("{pid} ({comm}) {}", t.join(" "))
    }

    /// Independent oracle: a line whose every field carries its own field
    /// number, so a shifted index shows up as an obviously wrong value
    /// rather than as agreement with a matching bug in the test helper.
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

    /// comm is arbitrary bytes between parens; splitting on the first `)`
    /// would misalign every field for a process named e.g. `(sd-pam)`.
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
        // Falls through to auto-detect; must never render an empty "()".
        assert!(!host_info(Some("")).contains("()"));
    }

    #[test]
    fn hostname_is_non_empty() {
        assert!(!hostname().is_empty());
    }
}
