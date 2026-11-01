//! Low-level process sampling and stat parser functions.

use crate::conv::signed;
use crate::proc_name::ProcName;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PidSample {
    pub ticks: u64,
    pub starttime: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProcStat {
    pub pid: u32,
    /// The authoritative name, when the scanner has one. Empty on Linux, where
    /// it is read lazily per drawn row -- see `stat_comm`.
    pub comm: ProcName,
    /// The name as `/proc/[pid]/stat` spells it: free, because that file was
    /// read anyway, and truncated to fifteen characters by the kernel.
    ///
    /// Good enough for the filter, which does substring matching, and not good
    /// enough for the table, which shows the name. The table therefore still
    /// pays `/proc/[pid]/comm` for the handful of rows it draws, and the filter
    /// pays nothing for the hundreds it does not.
    pub stat_comm: ProcName,
    pub ticks: u64,
    pub starttime: u64,
    pub rss_pages: u64,
}

/// A `/proc` entry's pid, when the entry is a process directory.
fn pid_of(name: &[u8]) -> Option<u32> {
    if !name.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(name).ok()?.parse().ok()
}

#[must_use]
pub fn parse_pid_stat(data: &str) -> Option<RawProcStat> {
    let open = data.find('(')?;
    let close = data.rfind(')')?;
    if close < open {
        return None;
    }
    let pid: u32 = data[..open].trim().parse().ok()?;
    let comm = ProcName::new(&data[open + 1..close]);

    let mut iter = data[close + 1..].split_ascii_whitespace();
    let utime_str = iter.nth(crate::consts::PROC_STAT_UTIME_FIELD)?;
    let stime_str = iter.next()?;
    let utime: u64 = utime_str.parse().ok()?;
    let stime: u64 = stime_str.parse().ok()?;

    let starttime_str = iter.nth(crate::consts::PROC_STAT_STARTTIME_FIELD)?;
    let starttime: u64 = starttime_str.parse().ok()?;

    let _skip20 = iter.next()?;
    let rss_str = iter.next()?;
    let rss_pages: u64 = rss_str.parse::<u64>().unwrap_or(0);

    Some(RawProcStat {
        pid,
        stat_comm: ProcName::default(),
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

/// The name `/proc/<pid>/comm` gives, or `unknown` -- inline, no heap.
#[must_use]
pub fn read_comm(pid: u32) -> ProcName {
    let mut path_buf = [0u8; crate::consts::PROC_PATH_BUF];
    let path = fmt_proc_comm_path(pid, &mut path_buf);
    let mut comm_buf = [0u8; crate::consts::PROC_COMM_BUF];
    let n = crate::proc::read_proc_bytes(path, &mut comm_buf);
    let name = std::str::from_utf8(&comm_buf[..n]).map_or("", str::trim);
    ProcName::new(if name.is_empty() { "unknown" } else { name })
}

pub struct ProcSampler {
    pub prev: HashMap<u32, PidSample>,
    pub clk_tck: f64,
    pub page_size: u64,
    pub scanned: Vec<RawProcStat>,
    pub active_pids: HashSet<u32>,
    pub temp_procs: Vec<(usize, f64, u64)>,
    /// The pid list macOS's scan asks the kernel for, kept between ticks.
    pub pid_buf: Vec<i32>,
}

impl Default for ProcSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcSampler {
    #[must_use]
    pub fn new() -> Self {
        let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            prev: HashMap::new(),
            clk_tck: if clk_tck > 0 { signed(clk_tck) } else { 100.0 },
            page_size: if page_size > 0 {
                u64::try_from(page_size).unwrap_or(4096)
            } else {
                4096
            },
            scanned: Vec::with_capacity(crate::consts::PROC_SCAN_CAPACITY),
            active_pids: HashSet::with_capacity(crate::consts::PROC_SCAN_CAPACITY),
            temp_procs: Vec::with_capacity(crate::consts::PROC_SCAN_CAPACITY),
            pid_buf: Vec::new(),
        }
    }

    /// Read every process into `scanned`, reusing its capacity.
    ///
    /// No heap allocation per process: names are inline (`ProcName`), and
    /// `/proc` is walked with `readdir(3)` rather than `std::fs::read_dir`,
    /// which allocates two strings per entry -- the per-process churn the
    /// static agent's allocator kept (roadmap M1). `scanned` itself grows only
    /// when the host runs more processes than it has held before.
    pub fn scan(&mut self) {
        self.scanned.clear();
        // SAFETY: a NUL-terminated literal path; a null return is handled.
        let dir = unsafe { libc::opendir(c"/proc".as_ptr()) };
        if dir.is_null() {
            crate::sys::scan_macos(&mut self.scanned, &mut self.pid_buf);
            return;
        }
        let mut path_buf = [0u8; crate::consts::PROC_PATH_BUF];
        let mut file_buf = [0u8; crate::consts::PROC_PID_STAT_BUF];

        loop {
            // SAFETY: `dir` is the open stream from above, closed only after
            // this loop; the entry it returns is valid until the next call.
            let entry = unsafe { libc::readdir(dir) };
            if entry.is_null() {
                break;
            }
            // SAFETY: `d_name` is a NUL-terminated array inside `*entry`.
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
            let Some(pid) = pid_of(name.to_bytes()) else {
                continue;
            };
            let path = fmt_proc_stat_path(pid, &mut path_buf);
            let n = crate::proc::read_proc_bytes(path, &mut file_buf);
            if n > 0 {
                if let Ok(data) = std::str::from_utf8(&file_buf[..n]) {
                    if let Some(mut st) = parse_pid_stat(data) {
                        // Moved, not dropped. `comm` stays empty so `top` knows
                        // to read the untruncated name for the rows it draws;
                        // the truncated one is kept for the filter, which would
                        // otherwise need a `/proc/[pid]/comm` read per process
                        // per tick to learn names nobody is looking at.
                        st.stat_comm = std::mem::take(&mut st.comm);
                        self.scanned.push(st);
                    }
                }
            }
        }
        // SAFETY: `dir` came from `opendir` and is closed exactly once.
        unsafe { libc::closedir(dir) };
    }
}
