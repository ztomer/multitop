//! `/proc` and `statvfs` sampling.
//!
//! Everything here is pure parsing over strings the caller supplies, except
//! for the thin `read_proc` / `statvfs` wrappers — which keeps the whole
//! module unit-testable on a host that has no `/proc` at all.
//!
//! COUNTER PERCENTAGES. Several functions here divide two monotonic counters
//! (CPU ticks since boot, byte totals) in f64. Both are orders of magnitude
//! below 2^53 and the result is rendered to one decimal, so f64 carries far
//! more precision than the output shows; integer division would lose the
//! fraction that IS the answer. `conv::unsigned` is that conversion, exact.

use crate::conv::unsigned;

pub use crate::proc_read::{for_each_line, read_proc, read_proc_bytes, read_proc_into};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub total: u64,
    /// idle + iowait
    pub idle: u64,
}

impl CpuTimes {
    /// Busy percentage over the window between two samples.
    #[must_use]
    pub fn pct_since(&self, prev: &Self) -> f64 {
        let total = self.total.saturating_sub(prev.total);
        let idle = self.idle.saturating_sub(prev.idle);
        if total == 0 {
            return 0.0;
        }
        unsigned(total.saturating_sub(idle)) / unsigned(total) * 100.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuStat {
    pub aggregate: CpuTimes,
    /// core index -> times, kept sorted by index
    pub cores: Vec<(usize, CpuTimes)>,
}

impl CpuStat {
    #[must_use]
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

impl CpuStat {
    /// Take one line of `/proc/stat`; non-`cpu` lines (`intr`, `ctxt`, ...)
    /// are ignored.
    fn add_line(&mut self, line: &str) {
        let Some(rest) = line.strip_prefix("cpu") else {
            return;
        };
        let Some((label, fields)) = rest.split_once(char::is_whitespace) else {
            return;
        };
        let Some(times) = parse_cpu_line(fields) else {
            return;
        };
        if label.is_empty() {
            self.aggregate = times;
        } else if let Ok(idx) = label.parse::<usize>() {
            self.cores.push((idx, times));
        }
    }

    fn finish(mut self) -> Self {
        self.cores.sort_unstable_by_key(|(i, _)| *i);
        self
    }
}

// reachability: fuzz/fuzz_targets/fuzz_proc_stat.rs drives this, from the fuzz crate's
// own workspace, which the scan does not read; production feeds the same
// `add_line` from `proc_read::for_each_line`.
/// Parse `/proc/stat`. Non-`cpu` lines (`intr`, `ctxt`, ...) are ignored.
#[must_use]
pub fn parse_proc_stat(data: &str) -> CpuStat {
    let mut stat = CpuStat::default();
    data.lines().for_each(|l| stat.add_line(l));
    stat.finish()
}

/// `/proc/stat`, or `None` when the file is absent, unreadable, or says
/// nothing — the three cases that send the caller to the platform sampler.
///
/// The path is a parameter so this, the Linux path that every deployed agent
/// actually runs, is reachable from a test on a host that has no `/proc`.
#[must_use]
pub fn cpu_stat_from(path: &str) -> Option<CpuStat> {
    let mut stat = CpuStat::default();
    cpu_stat_into(path, &mut stat).then_some(stat)
}

/// `/proc/stat` into `out`, whose core list is a buffer kept between ticks;
/// `false` for the three cases `cpu_stat_from` answers `None`.
#[must_use]
pub fn cpu_stat_into(path: &str, out: &mut CpuStat) -> bool {
    out.aggregate = CpuTimes::default();
    out.cores.clear();
    if !for_each_line(path, |l| out.add_line(l)) {
        return false;
    }
    out.cores.sort_unstable_by_key(|(i, _)| *i);
    !out.cores.is_empty() || out.aggregate.total != 0
}

#[must_use]
pub fn get_cpu_stat() -> CpuStat {
    cpu_stat_from("/proc/stat").unwrap_or_else(crate::sys::get_cpu_stat_macos)
}

/// This host's CPU times into `out`, with no heap work once its core list has
/// grown to the core count.
pub fn get_cpu_stat_into(out: &mut CpuStat) {
    if !cpu_stat_into("/proc/stat", out) {
        crate::sys::cpu_stat_macos_into(out);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub pct: f64,
}

impl Usage {
    #[must_use]
    pub fn new(total: u64, used: u64) -> Self {
        let pct = if total > 0 {
            unsigned(used) / unsigned(total) * 100.0
        } else {
            0.0
        };
        Self { total, used, pct }
    }
}

/// The `/proc/meminfo` rows memory use is computed from, in kB.
#[derive(Default)]
struct MemInfo {
    total: u64,
    free: u64,
    buffers: u64,
    cached: u64,
}

impl MemInfo {
    fn add_line(&mut self, line: &str) {
        let Some((key, value)) = line.split_once(':') else {
            return;
        };
        let Some(kb) = value
            .split_ascii_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            return;
        };
        match key.trim() {
            "MemTotal" => self.total = kb,
            "MemFree" => self.free = kb,
            "Buffers" => self.buffers = kb,
            "Cached" => self.cached = kb,
            _ => {}
        }
    }

    /// Used = total - free - buffers - cached.
    fn usage(&self) -> Usage {
        let total = self.total * 1024;
        let reclaimable = (self.free + self.buffers + self.cached) * 1024;
        Usage::new(total, total.saturating_sub(reclaimable))
    }
}

// reachability: fuzz/fuzz_targets/fuzz_meminfo.rs drives this, from the fuzz crate's
// own workspace, which the scan does not read; production feeds the same
// `add_line` from `proc_read::for_each_line`.
/// Parse `/proc/meminfo`. Used = total - free - buffers - cached.
#[must_use]
pub fn parse_meminfo(data: &str) -> Usage {
    let mut info = MemInfo::default();
    data.lines().for_each(|l| info.add_line(l));
    info.usage()
}

/// `/proc/meminfo`, or `None` when it is absent or reports no total.
#[must_use]
pub fn memory_from(path: &str) -> Option<Usage> {
    let mut info = MemInfo::default();
    if !for_each_line(path, |l| info.add_line(l)) {
        return None;
    }
    let mem = info.usage();
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

impl NetTotals {
    /// Take one interface row of `/proc/net/dev`. The two header rows have
    /// no counters after their colon-free first field, so they add nothing.
    fn add_line(&mut self, line: &str) {
        let Some((iface, counters)) = line.split_once(':') else {
            return;
        };
        let iface = iface.trim();
        if iface == "lo" || iface.starts_with("lo:") {
            return;
        }
        let mut iter = counters.split_ascii_whitespace();
        let Some(rx_str) = iter.next() else {
            return;
        };
        let rx_val: u64 = rx_str.parse().unwrap_or(0);
        let Some(tx_str) = iter.nth(crate::consts::NET_DEV_TX_OFFSET) else {
            return;
        };
        let tx_val: u64 = tx_str.parse().unwrap_or(0);
        self.rx = self.rx.saturating_add(rx_val);
        self.tx = self.tx.saturating_add(tx_val);
    }
}

// reachability: fuzz/fuzz_targets/fuzz_net_dev.rs drives this, from the fuzz crate's
// own workspace, which the scan does not read; production feeds the same
// `add_line` from `proc_read::for_each_line`.
#[must_use]
pub fn parse_net_dev(data: &str) -> NetTotals {
    let mut totals = NetTotals::default();
    data.lines().skip(2).for_each(|l| totals.add_line(l));
    totals
}

/// `/proc/net/dev`, or `None` when it is absent or every counter is zero.
#[must_use]
pub fn net_from(path: &str) -> Option<NetTotals> {
    let mut net = NetTotals::default();
    let mut row = 0usize;
    let read = for_each_line(path, |l| {
        // The two header rows, skipped as `parse_net_dev` skips them.
        if row >= 2 {
            net.add_line(l);
        }
        row += 1;
    });
    (read && (net.rx != 0 || net.tx != 0)).then_some(net)
}

pub fn get_net() -> NetTotals {
    net_from("/proc/net/dev").unwrap_or_else(crate::sys::get_net_macos)
}

pub use crate::proc_disk::{get_disk, statvfs_bytes};
pub use crate::sys_temps::fill_core_temps;

/// One row of the process table. `Copy`: the name is inline (`ProcName`), so
/// the rows a tick sends are refilled in place with no heap work (roadmap M2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: ProcName,
    pub cpu: f64,
    pub mem: u64,
}

use crate::proc_name::ProcName;
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

    /// Top `n` processes over the last `elapsed` seconds, sorted by `sort_by`,
    /// into `out` -- the caller's buffer, kept between ticks.
    pub fn top_into(
        &mut self,
        elapsed: f64,
        n: usize,
        sort_by: crate::SortBy,
        out: &mut Vec<Proc>,
    ) {
        self.scan();
        let scanned = std::mem::take(&mut self.scanned);
        self.temp_procs.clear();
        for (i, s) in scanned.iter().enumerate() {
            let cpu = match self.prev.get(&s.pid) {
                Some(p) if p.starttime == s.starttime && elapsed > 0.0 => {
                    unsigned(s.ticks.saturating_sub(p.ticks)) / self.clk_tck / elapsed * 100.0
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
                    });
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
                    });
            }
        }

        self.temp_procs.truncate(n);
        out.clear();
        out.extend(self.temp_procs.iter().map(|&(i, cpu, mem)| Proc {
            pid: scanned[i].pid,
            name: if scanned[i].comm.is_empty() {
                crate::proc_sys::read_comm(scanned[i].pid)
            } else {
                scanned[i].comm
            },
            cpu,
            mem,
        }));
        self.scanned = scanned;
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
    ///
    /// Filled into the caller's buffer, which the monitor keeps between ticks:
    /// a list built fresh every tick, at the size of the process table, was
    /// part of the churn the static agent's allocator kept (roadmap M1).
    pub fn fill_names(&self, names: &mut Vec<ProcName>) {
        names.clear();
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
                names.push(*name);
            }
        }
        // Sorted so an unchanged host produces the same bytes each tick -- and
        // sorting is also what makes the dedup one pass instead of a set.
        names.sort_unstable();
        names.dedup();
        // Deduplicated first: a host with forty `nginx` workers spends one name
        // of the budget, not forty.
        names.truncate(crate::consts::MAX_FILTER_NAMES);
    }
}

/// Hostname from `/proc`, falling back to `gethostname(2)`.
#[must_use]
pub fn hostname() -> String {
    let from_proc = read_proc("/proc/sys/kernel/hostname").trim().to_string();
    if !from_proc.is_empty() {
        return from_proc;
    }
    let mut buf = vec![0u8; crate::consts::SYSCTL_BUF];
    // SAFETY: buf is a valid writable allocation of the length we pass.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
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
#[must_use]
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
