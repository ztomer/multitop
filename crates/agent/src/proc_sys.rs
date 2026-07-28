//! Low-level process sampling and stat parser functions.

use std::collections::HashMap;
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

pub struct ProcSampler {
    pub prev: HashMap<u32, PidSample>,
    pub clk_tck: f64,
    pub page_size: u64,
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
            page_size: if page_size > 0 { page_size as u64 } else { 4096 },
        }
    }

    pub fn scan(&self) -> Vec<RawProcStat> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return crate::sys::scan_macos();
        };
        let mut out = Vec::with_capacity(256);
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.as_bytes()[0].is_ascii_digit() {
                continue;
            }
            let Ok(data) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            if let Some(st) = parse_pid_stat(&data) {
                out.push(st);
            }
        }
        out
    }
}
