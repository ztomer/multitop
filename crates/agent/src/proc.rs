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
    let mut buf = [0u8; 1024];
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
    let mut out = String::with_capacity(512);
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

pub fn get_cpu_stat() -> CpuStat {
    let stat = parse_proc_stat(&read_proc("/proc/stat"));
    if stat.cores.is_empty() && stat.aggregate.total == 0 {
        crate::sys::get_cpu_stat_macos()
    } else {
        stat
    }
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
    let mount = root_mount_point(&mountinfo).unwrap_or("/");
    let Some((total, free)) = statvfs_bytes(mount) else {
        return Usage::default();
    };
    Usage::new(total, total.saturating_sub(free))
}

pub fn get_memory() -> Usage {
    let usage = parse_meminfo(&read_proc("/proc/meminfo"));
    if usage.total == 0 {
        crate::sys::get_memory_macos()
    } else {
        usage
    }
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
    let totals = parse_net_dev(&read_proc("/proc/net/dev"));
    if totals.rx == 0 && totals.tx == 0 {
        crate::sys::get_net_macos()
    } else {
        totals
    }
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

    /// Top `n` processes over the last `elapsed` seconds, sorted by `sort_by`.
    pub fn top(&mut self, elapsed: f64, n: usize, sort_by: crate::SortBy) -> Vec<Proc> {
        let scanned = self.scan();
        let mut temp_procs: Vec<(usize, f64, u64)> = Vec::with_capacity(scanned.len());
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
            temp_procs.push((i, cpu, s.rss_pages * self.page_size));
        }
        self.prev.retain(|pid, _| scanned.iter().any(|s| s.pid == *pid));

        match sort_by {
            crate::SortBy::Cpu => {
                temp_procs.sort_unstable_by(|&(i_a, cpu_a, mem_a), &(i_b, cpu_b, mem_b)| {
                    cpu_b
                        .partial_cmp(&cpu_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| mem_b.cmp(&mem_a))
                        .then_with(|| scanned[i_a].pid.cmp(&scanned[i_b].pid))
                })
            }
            crate::SortBy::Mem => {
                temp_procs.sort_unstable_by(|&(i_a, cpu_a, mem_a), &(i_b, cpu_b, mem_b)| {
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

        temp_procs.truncate(n);

        temp_procs
            .into_iter()
            .map(|(i, cpu, mem)| Proc {
                pid: scanned[i].pid,
                name: if scanned[i].comm.is_empty() {
                    crate::proc_sys::read_comm(scanned[i].pid)
                } else {
                    scanned[i].comm.clone()
                },
                cpu,
                mem,
            })
            .collect()
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
