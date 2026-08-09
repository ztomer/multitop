//! `/proc` and `statvfs` sampling.
//!
//! Everything here is pure parsing over strings the caller supplies, except
//! for the thin `read_proc` / `statvfs` wrappers — which keeps the whole
//! module unit-testable on a host that has no `/proc` at all.

use std::fs;
use std::path::Path;

/// Read a `/proc` file into a String with pre-allocated capacity, using an 8KB
/// stack buffer to eliminate standard library reallocations on `st_size = 0`
/// kernel pseudofiles.
/// Read a pseudofile into a fixed stack byte buffer with ZERO heap allocation.
pub fn read_proc_bytes<P: AsRef<Path>, const N: usize>(path: P, out: &mut [u8; N]) -> usize {
    let Ok(mut file) = fs::File::open(path) else {
        return 0;
    };
    use std::io::Read;
    file.read(out).unwrap_or_default()
}

/// Read a pseudofile into a caller-supplied String without allocating new memory buffers.
pub fn read_proc_into<P: AsRef<Path>>(path: P, out: &mut String) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; crate::consts::PROC_CHUNK_BUF];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                    out.push_str(s);
                } else {
                    out.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
            Err(_) => return false,
        }
    }
    !out.is_empty()
}

pub fn read_proc<P: AsRef<Path>>(path: P) -> String {
    let mut out = String::with_capacity(crate::consts::PROC_LINE_CAPACITY);
    read_proc_into(path, &mut out);
    out
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
        if n < crate::consts::PROC_STAT_MIN_FIELDS {
            vals[n] = v;
        }
        n += 1;
    }
    // idle and iowait are columns 4 and 5; without them the line is unusable.
    if n < crate::consts::PROC_STAT_MIN_FIELDS {
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

/// `/proc/stat`, or `None` when the file is absent, unreadable, or says
/// nothing — the three cases that send the caller to the platform sampler.
///
/// The path is a parameter so this, the Linux path that every deployed agent
/// actually runs, is reachable from a test on a host that has no `/proc`.
pub fn cpu_stat_from(path: &str) -> Option<CpuStat> {
    let mut buf = [0u8; crate::consts::PROC_STAT_BUF];
    let n = read_proc_bytes(path, &mut buf);
    if n == 0 {
        return None;
    }
    let stat = parse_proc_stat(std::str::from_utf8(&buf[..n]).ok()?);
    (!stat.cores.is_empty() || stat.aggregate.total != 0).then_some(stat)
}

pub fn get_cpu_stat() -> CpuStat {
    cpu_stat_from("/proc/stat").unwrap_or_else(crate::sys::get_cpu_stat_macos)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub pct: f64,
}

impl Usage {
    pub fn new(total: u64, used: u64) -> Self {
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

pub fn root_mount_point(mountinfo: &str) -> Option<&str> {
    mountinfo.lines().find_map(|line| {
        let mut parts = line.split_ascii_whitespace();
        let mount_point = parts.nth(4)?;
        (mount_point == "/").then_some(mount_point)
    })
}

pub fn statvfs_bytes(path: &str) -> Option<(u64, u64)> {
    let c_path = std::ffi::CString::new(path).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut st) != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
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

pub fn get_disk() -> Usage {
    let root = root_mount_from("/proc/self/mountinfo");
    let target = root.as_deref().unwrap_or("/");
    if let Some((total, free)) = statvfs_bytes(target) {
        Usage::new(total, total.saturating_sub(free))
    } else {
        Usage::default()
    }
}

/// `/proc/meminfo`, or `None` when it is absent or reports no total.
pub fn memory_from(path: &str) -> Option<Usage> {
    let mut buf = [0u8; crate::consts::PROC_MEMINFO_BUF];
    let n = read_proc_bytes(path, &mut buf);
    if n == 0 {
        return None;
    }
    let mem = parse_meminfo(std::str::from_utf8(&buf[..n]).ok()?);
    (mem.total != 0).then_some(mem)
}

pub fn get_memory() -> Usage {
    memory_from("/proc/meminfo").unwrap_or_else(crate::sys::get_memory_macos)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetTotals {
    pub rx: u64,
    pub tx: u64,
}

pub fn parse_net_dev(data: &str) -> NetTotals {
    let mut totals = NetTotals::default();
    for line in data.lines().skip(2) {
        let Some((iface, counters)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo" || iface.starts_with("lo:") {
            continue;
        }
        let mut iter = counters.split_ascii_whitespace();
        let Some(rx_str) = iter.next() else {
            continue;
        };
        let rx_val: u64 = rx_str.parse().unwrap_or(0);
        let Some(tx_str) = iter.nth(crate::consts::NET_DEV_TX_OFFSET) else {
            continue;
        };
        let tx_val: u64 = tx_str.parse().unwrap_or(0);
        totals.rx = totals.rx.saturating_add(rx_val);
        totals.tx = totals.tx.saturating_add(tx_val);
    }
    totals
}

/// `/proc/net/dev`, or `None` when it is absent or every counter is zero.
pub fn net_from(path: &str) -> Option<NetTotals> {
    let mut buf = [0u8; crate::consts::PROC_NET_DEV_BUF];
    let n = read_proc_bytes(path, &mut buf);
    if n == 0 {
        return None;
    }
    let net = parse_net_dev(std::str::from_utf8(&buf[..n]).ok()?);
    (net.rx != 0 || net.tx != 0).then_some(net)
}

pub fn get_net() -> NetTotals {
    net_from("/proc/net/dev").unwrap_or_else(crate::sys::get_net_macos)
}

pub use crate::sys::get_core_temps;

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub cpu: f64,
    pub mem: u64,
}

pub use crate::proc_sys::*;

impl ProcSampler {
    /// Prime the baseline without producing output.
    pub fn prime(&mut self) {
        self.scan();
        self.prev.clear();
        for s in &self.scanned {
            self.prev.insert(
                s.pid,
                PidSample {
                    ticks: s.ticks,
                    starttime: s.starttime,
                },
            );
        }
    }

    /// Top `n` processes over the last `elapsed` seconds, sorted by `sort_by`.
    pub fn top(&mut self, elapsed: f64, n: usize, sort_by: crate::SortBy) -> Vec<Proc> {
        self.scan();
        let scanned = std::mem::take(&mut self.scanned);
        self.temp_procs.clear();
        for (i, s) in scanned.iter().enumerate() {
            let cpu = match self.prev.get(&s.pid) {
                Some(p) if p.starttime == s.starttime && elapsed > 0.0 => {
                    s.ticks.saturating_sub(p.ticks) as f64 / self.clk_tck / elapsed * 100.0
                }
                _ => 0.0,
            };
            self.prev.insert(
                s.pid,
                PidSample {
                    ticks: s.ticks,
                    starttime: s.starttime,
                },
            );
            self.temp_procs.push((i, cpu, s.rss_pages * self.page_size));
        }
        self.active_pids.clear();
        for s in &scanned {
            self.active_pids.insert(s.pid);
        }
        self.prev.retain(|pid, _| self.active_pids.contains(pid));

        match sort_by {
            crate::SortBy::Cpu => {
                self.temp_procs
                    .sort_unstable_by(|&(i_a, cpu_a, mem_a), &(i_b, cpu_b, mem_b)| {
                        cpu_b
                            .partial_cmp(&cpu_a)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| mem_b.cmp(&mem_a))
                            .then_with(|| scanned[i_a].pid.cmp(&scanned[i_b].pid))
                    })
            }
            crate::SortBy::Mem => {
                self.temp_procs
                    .sort_unstable_by(|&(i_a, cpu_a, mem_a), &(i_b, cpu_b, mem_b)| {
                        mem_b
                            .cmp(&mem_a)
                            .then_with(|| {
                                cpu_b
                                    .partial_cmp(&cpu_a)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .then_with(|| scanned[i_a].pid.cmp(&scanned[i_b].pid))
                    })
            }
        }

        self.temp_procs.truncate(n);
        let result: Vec<Proc> = self
            .temp_procs
            .iter()
            .map(|&(i, cpu, mem)| Proc {
                pid: scanned[i].pid,
                name: if scanned[i].comm.is_empty() {
                    crate::proc_sys::read_comm(scanned[i].pid)
                } else {
                    scanned[i].comm.clone()
                },
                cpu,
                mem,
            })
            .collect();
        self.scanned = scanned;
        result
    }

    /// Every distinct process name the last scan saw, for the filter to search.
    ///
    /// Separate from `top`, which returns the rows the table draws. These are
    /// what the host is *running*; those are what there is room to *show*, and
    /// conflating the two is what made "filter by process" mean "filter by the
    /// processes that happen to fit".
    ///
    /// Read straight off the scan rather than out of a pool filled somewhere
    /// else. A pool would have to be filled by `top`, which makes this return
    /// nothing unless the caller happens to have called `top` first -- an
    /// ordering rule enforced by a comment, which is not enforced at all.
    #[must_use]
    pub fn scanned_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::with_capacity(self.scanned.len());
        for s in &self.scanned {
            // Whichever the platform filled. On Linux that is the truncated
            // one, and truncated is what a substring filter can live with; on
            // macOS the scanner has the full name already.
            let name = if s.comm.is_empty() {
                &s.stat_comm
            } else {
                &s.comm
            };
            if !name.is_empty() {
                names.push(name.clone());
            }
        }
        // Sorted so an unchanged host produces the same bytes each tick -- and
        // sorting is also what makes the dedup one pass instead of a set.
        names.sort_unstable();
        names.dedup();
        // Deduplicated first: a host with forty `nginx` workers spends one name
        // of the budget, not forty.
        names.truncate(crate::consts::MAX_FILTER_NAMES);
        names
    }
}

/// Hostname from `/proc`, falling back to `gethostname(2)`.
pub fn hostname() -> String {
    let from_proc = read_proc("/proc/sys/kernel/hostname").trim().to_string();
    if !from_proc.is_empty() {
        return from_proc;
    }
    let mut buf = vec![0u8; crate::consts::SYSCTL_BUF];
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
