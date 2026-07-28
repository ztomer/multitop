//! Platform-specific sampling fallback for non-Linux hosts (e.g. macOS).

use std::collections::HashMap;

#[allow(unused_imports)]
use crate::proc::{CpuStat, CpuTimes, NetTotals, RawProcStat, Usage};

#[cfg(target_os = "macos")]
pub fn get_cpu_stat_macos() -> CpuStat {
    let mut stat = CpuStat::default();
    let mut num_cpus: libc::natural_t = 0;
    let mut cpu_info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut msg_type: libc::mach_msg_type_number_t = 0;

    let ret = unsafe {
        libc::host_processor_info(
            libc::mach_host_self(),
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut num_cpus,
            &mut cpu_info,
            &mut msg_type,
        )
    };

    if ret == libc::KERN_SUCCESS && !cpu_info.is_null() {
        let cpu_load = cpu_info as *const libc::processor_cpu_load_info_data_t;
        let mut agg_total: u64 = 0;
        let mut agg_idle: u64 = 0;

        for i in 0..(num_cpus as usize) {
            let info = unsafe { *cpu_load.add(i) };
            let user = info.cpu_ticks[libc::CPU_STATE_USER as usize] as u64;
            let system = info.cpu_ticks[libc::CPU_STATE_SYSTEM as usize] as u64;
            let idle = info.cpu_ticks[libc::CPU_STATE_IDLE as usize] as u64;
            let nice = info.cpu_ticks[libc::CPU_STATE_NICE as usize] as u64;

            let total = user + system + idle + nice;
            agg_total += total;
            agg_idle += idle;

            stat.cores.push((i, CpuTimes { total, idle }));
        }

        unsafe {
            let vm_map = libc::mach_task_self();
            let size = msg_type as usize * std::mem::size_of::<libc::integer_t>();
            libc::vm_deallocate(vm_map, cpu_info as libc::vm_address_t, size as libc::vm_size_t);
        }

        stat.aggregate = CpuTimes {
            total: agg_total,
            idle: agg_idle,
        };
        stat.cores.sort_unstable_by_key(|(i, _)| *i);
    }
    stat
}

#[cfg(not(target_os = "macos"))]
pub fn get_cpu_stat_macos() -> CpuStat {
    CpuStat::default()
}

#[cfg(target_os = "macos")]
pub fn get_memory_macos() -> Usage {
    let mut total: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    unsafe {
        let name = std::ffi::CString::new("hw.memsize").unwrap();
        libc::sysctlbyname(name.as_ptr(), &mut total as *mut _ as *mut _, &mut size, std::ptr::null_mut(), 0);
    }
    if total == 0 {
        return Usage::default();
    }

    let mut vm_info: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = (std::mem::size_of::<libc::vm_statistics64>() / std::mem::size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
    let host_port = unsafe { libc::mach_host_self() };
    let ret = unsafe {
        libc::host_statistics64(
            host_port,
            libc::HOST_VM_INFO64,
            &mut vm_info as *mut _ as *mut _,
            &mut count,
        )
    };

    if ret == libc::KERN_SUCCESS {
        let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = if ps > 0 { ps as u64 } else { 4096 };
        let active = vm_info.active_count as u64 * page_size;
        let wire = vm_info.wire_count as u64 * page_size;
        let compressed = vm_info.compressor_page_count as u64 * page_size;
        let used = active + wire + compressed;
        Usage::new(total, used.min(total))
    } else {
        Usage::new(total, 0)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_memory_macos() -> Usage {
    Usage::default()
}

#[cfg(target_os = "macos")]
pub fn get_net_macos() -> NetTotals {
    let mut totals = NetTotals::default();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) == 0 && !ifap.is_null() {
            let mut curr = ifap;
            while !curr.is_null() {
                let ifa = *curr;
                if !ifa.ifa_name.is_null() && !ifa.ifa_data.is_null() {
                    let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy();
                    if name != "lo0" && !name.starts_with("lo") {
                        let sa_family = (*ifa.ifa_addr).sa_family;
                        if sa_family == libc::AF_LINK as u8 {
                            let data = ifa.ifa_data as *const libc::if_data;
                            totals.rx = totals.rx.saturating_add((*data).ifi_ibytes as u64);
                            totals.tx = totals.tx.saturating_add((*data).ifi_obytes as u64);
                        }
                    }
                }
                curr = ifa.ifa_next;
            }
            libc::freeifaddrs(ifap);
        }
    }
    totals
}

#[cfg(not(target_os = "macos"))]
pub fn get_net_macos() -> NetTotals {
    NetTotals::default()
}

#[cfg(target_os = "macos")]
pub fn scan_macos() -> Vec<RawProcStat> {
    let mut out = Vec::with_capacity(256);
    let num_pids = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if num_pids <= 0 {
        return out;
    }
    let mut pids = vec![0i32; num_pids as usize + 64];
    let bytes_got = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr() as *mut _,
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if bytes_got <= 0 {
        return out;
    }
    let actual_count = bytes_got as usize / std::mem::size_of::<i32>();
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let hz = if clk_tck > 0 { clk_tck as u64 } else { 100 };

    for &pid in &pids[..actual_count] {
        if pid <= 0 {
            continue;
        }
        let mut task_info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let res = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKINFO,
                0,
                &mut task_info as *mut _ as *mut _,
                std::mem::size_of::<libc::proc_taskinfo>() as i32,
            )
        };
        if res <= 0 {
            continue;
        }

        let mut name_buf = [0u8; 256];
        let name_res = unsafe {
            libc::proc_name(pid, name_buf.as_mut_ptr() as *mut _, name_buf.len() as u32)
        };
        let comm = if name_res > 0 {
            String::from_utf8_lossy(&name_buf[..name_res as usize]).to_string()
        } else {
            format!("pid_{pid}")
        };

        let total_ns = task_info.pti_total_user + task_info.pti_total_system;
        let ticks = (total_ns as u64) * hz / 1_000_000_000;
        let rss_pages = task_info.pti_resident_size / 4096;

        out.push(RawProcStat {
            pid: pid as u32,
            comm,
            ticks,
            starttime: 0,
            rss_pages,
        });
    }
    out
}

#[cfg(not(target_os = "macos"))]
pub fn scan_macos() -> Vec<RawProcStat> {
    Vec::new()
}

/// Read temperatures for CPUs/cores from sysfs.
pub fn get_core_temps() -> HashMap<usize, f64> {
    let mut temps = HashMap::new();

    if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            if let Ok(files) = std::fs::read_dir(entry.path()) {
                for f in files.flatten() {
                    let fname = f.file_name();
                    let fstr = fname.to_string_lossy();
                    if fstr.starts_with("temp") && fstr.ends_with("_input") {
                        if let Ok(val) = crate::proc::read_proc(f.path()).trim().parse::<f64>() {
                            let c = if val > 1000.0 { val / 1000.0 } else { val };
                            if (0.0..=150.0).contains(&c) {
                                let idx = fstr
                                    .strip_prefix("temp")
                                    .and_then(|s| s.strip_suffix("_input"))
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or(1)
                                    .saturating_sub(1);
                                temps.insert(idx, c);
                            }
                        }
                    }
                }
            }
        }
    }

    if temps.is_empty() {
        if let Ok(entries) = std::fs::read_dir("/sys/class/thermal") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("thermal_zone") {
                    let temp_path = entry.path().join("temp");
                    if let Ok(val) = crate::proc::read_proc(&temp_path).trim().parse::<f64>() {
                        let c = if val > 1000.0 { val / 1000.0 } else { val };
                        if (0.0..=150.0).contains(&c) {
                            let idx = name_str
                                .strip_prefix("thermal_zone")
                                .and_then(|s| s.parse::<usize>().ok())
                                .unwrap_or(0);
                            temps.entry(idx).or_insert(c);
                        }
                    }
                }
            }
        }
    }

    temps
}
