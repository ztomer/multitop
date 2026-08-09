//! Low-level process sampling and stat parser functions.

use std::collections::{HashMap, HashSet};
use std::fs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PidSample {
    pub ticks: u64,
    pub starttime: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProcStat {
    pub pid: u32,
    pub comm: String,
    pub ticks: u64,
    pub starttime: u64,
    pub rss_pages: u64,
}

pub fn parse_pid_stat(data: &str) -> Option<RawProcStat> {
    let open = data.find('(')?;
    let close = data.rfind(')')?;
    if close < open {
        return None;
    }
    let pid: u32 = data[..open].trim().parse().ok()?;
    let comm = data[open + 1..close].to_string();

    let mut iter = data[close + 1..].split_ascii_whitespace();
    let utime_str = iter.nth(crate::consts::PROC_STAT_UTIME_FIELD)?;
    let stime_str = iter.next()?;
    let utime: u64 = utime_str.parse().ok()?;
    let stime: u64 = stime_str.parse().ok()?;

    let starttime_str = iter.nth(crate::consts::PROC_STAT_STARTTIME_FIELD)?;
    let starttime: u64 = starttime_str.parse().ok()?;

    let _skip20 = iter.next()?;
    let rss_str = iter.next()?;
    let rss_pages: u64 = rss_str.parse::<i64>().unwrap_or(0).max(0) as u64;

    Some(RawProcStat {
        pid,
        comm,
        ticks: utime.saturating_add(stime),
        starttime,
        rss_pages,
    })
}

/// Format `/proc/<pid>/stat` into a stack byte buffer with ZERO heap allocation.
pub fn fmt_proc_stat_path(pid: u32, out: &mut [u8; crate::consts::PROC_PATH_BUF]) -> &str {
    let mut b = [0u8; crate::consts::PID_DIGITS_BUF];
    let mut i = b.len();
    let mut n = pid;
    if n == 0 {
        i -= 1;
        b[i] = b'0';
    } else {
        while n > 0 {
            i -= 1;
            b[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    let prefix = b"/proc/";
    let suffix = b"/stat";
    let len = prefix.len() + (b.len() - i) + suffix.len();
    out[..prefix.len()].copy_from_slice(prefix);
    out[prefix.len()..prefix.len() + (b.len() - i)].copy_from_slice(&b[i..]);
    out[prefix.len() + (b.len() - i)..len].copy_from_slice(suffix);
    std::str::from_utf8(&out[..len]).unwrap_or("")
}

/// Format `/proc/<pid>/comm` into a stack byte buffer with ZERO heap allocation.
pub fn fmt_proc_comm_path(pid: u32, out: &mut [u8; crate::consts::PROC_PATH_BUF]) -> &str {
    let mut b = [0u8; crate::consts::PID_DIGITS_BUF];
    let mut i = b.len();
    let mut n = pid;
    if n == 0 {
        i -= 1;
        b[i] = b'0';
    } else {
        while n > 0 {
            i -= 1;
            b[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    let prefix = b"/proc/";
    let suffix = b"/comm";
    let len = prefix.len() + (b.len() - i) + suffix.len();
    out[..prefix.len()].copy_from_slice(prefix);
    out[prefix.len()..prefix.len() + (b.len() - i)].copy_from_slice(&b[i..]);
    out[prefix.len() + (b.len() - i)..len].copy_from_slice(suffix);
    std::str::from_utf8(&out[..len]).unwrap_or("")
}

pub fn read_comm(pid: u32) -> String {
    let mut path_buf = [0u8; crate::consts::PROC_PATH_BUF];
    let path = fmt_proc_comm_path(pid, &mut path_buf);
    let mut comm_buf = [0u8; crate::consts::PROC_COMM_BUF];
    let n = crate::proc::read_proc_bytes(path, &mut comm_buf);
    if n > 0 {
        std::str::from_utf8(&comm_buf[..n])
            .unwrap_or("unknown")
            .trim()
            .to_string()
    } else {
        "unknown".to_string()
    }
}

pub struct ProcSampler {
    pub prev: HashMap<u32, PidSample>,
    pub clk_tck: f64,
    pub page_size: u64,
    pub scanned: Vec<RawProcStat>,
    pub active_pids: HashSet<u32>,
    pub temp_procs: Vec<(usize, f64, u64)>,
}

impl Default for ProcSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcSampler {
    pub fn new() -> Self {
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
            scanned: Vec::with_capacity(crate::consts::PROC_SCAN_CAPACITY),
            active_pids: HashSet::with_capacity(crate::consts::PROC_SCAN_CAPACITY),
            temp_procs: Vec::with_capacity(crate::consts::PROC_SCAN_CAPACITY),
        }
    }

    pub fn scan(&mut self) {
        self.scanned.clear();
        let Ok(entries) = fs::read_dir("/proc") else {
            let macos = crate::sys::scan_macos();
            self.scanned.reserve(macos.len());
            for s in macos {
                self.scanned.push(s);
            }
            return;
        };
        let mut path_buf = [0u8; crate::consts::PROC_PATH_BUF];
        let mut file_buf = [0u8; crate::consts::PROC_PID_STAT_BUF];

        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                continue;
            }
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            let path = fmt_proc_stat_path(pid, &mut path_buf);
            let n = crate::proc::read_proc_bytes(path, &mut file_buf);
            if n > 0 {
                if let Ok(data) = std::str::from_utf8(&file_buf[..n]) {
                    if let Some(mut st) = parse_pid_stat(data) {
                        st.comm = String::new();
                        self.scanned.push(st);
                    }
                }
            }
        }
    }
}
