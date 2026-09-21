//! Platform-specific sampling fallback for non-Linux hosts (e.g. macOS).
//!
//! CONVERSIONS AT THE libc BOUNDARY. The syscall signatures fix these widths;
//! every return is range-checked first (`num_pids <= 0`, `bytes_got <= 0`
//! both bail) and every narrowing is a `try_from` with its fallback stated.

use crate::proc::{CpuStat, NetTotals, RawProcStat, Usage};
// Only the macOS sampler builds `CpuTimes` values; on Linux the import is dead,
// and the cfg says so.
#[cfg(target_os = "macos")]
use crate::proc::CpuTimes;

// libc deprecated its Mach bindings in favour of `mach2` (house rule: migrate
// on sight). `mach_task_self` comes from mach2; `mach_host_self` is not in
// mach2 0.4, so it is declared here beside `mach_port_deallocate` -- both are
// plain libSystem symbols, and a local declaration is what libc did anyway.
#[cfg(target_os = "macos")]
extern "C" {
    fn mach_host_self() -> libc::mach_port_t;
    fn mach_port_deallocate(
        target_task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

#[cfg(target_os = "macos")]
struct MachCpuInfoGuard {
    host_port: libc::mach_port_t,
    cpu_info: libc::processor_info_array_t,
    msg_type: libc::mach_msg_type_number_t,
}

#[cfg(target_os = "macos")]
impl Drop for MachCpuInfoGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.cpu_info.is_null() {
                let vm_map = mach2::traps::mach_task_self();
                let size = self.msg_type as usize * std::mem::size_of::<libc::integer_t>();
                libc::vm_deallocate(
                    vm_map,
                    self.cpu_info as libc::vm_address_t,
                    size as libc::vm_size_t,
                );
            }
            if self.host_port != 0 {
                mach_port_deallocate(mach2::traps::mach_task_self(), self.host_port);
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn get_cpu_stat_macos() -> CpuStat {
    let mut stat = CpuStat::default();
    let mut num_cpus: libc::natural_t = 0;
    let mut cpu_info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut msg_type: libc::mach_msg_type_number_t = 0;

    let host_port = unsafe { mach_host_self() };
    let ret = unsafe {
        libc::host_processor_info(
            host_port,
            libc::PROCESSOR_CPU_LOAD_INFO,
            &raw mut num_cpus,
            &raw mut cpu_info,
            &raw mut msg_type,
        )
    };

    let _guard = MachCpuInfoGuard {
        host_port,
        cpu_info,
        msg_type,
    };

    if ret == libc::KERN_SUCCESS && !cpu_info.is_null() {
        let cpu_load = cpu_info as *const libc::processor_cpu_load_info_data_t;
        let mut agg_total: u64 = 0;
        let mut agg_idle: u64 = 0;

        for i in 0..(num_cpus as usize) {
            let info = unsafe { *cpu_load.add(i) };
            let user = u64::from(info.cpu_ticks[libc::CPU_STATE_USER as usize]);
            let system = u64::from(info.cpu_ticks[libc::CPU_STATE_SYSTEM as usize]);
            let idle = u64::from(info.cpu_ticks[libc::CPU_STATE_IDLE as usize]);
            let nice = u64::from(info.cpu_ticks[libc::CPU_STATE_NICE as usize]);

            let total = user + system + idle + nice;
            agg_total += total;
            agg_idle += idle;

            stat.cores.push((i, CpuTimes { total, idle }));
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
#[must_use]
pub fn get_cpu_stat_macos() -> CpuStat {
    CpuStat::default()
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn get_memory_macos() -> Usage {
    let mut total: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    if let Ok(name) = std::ffi::CString::new("hw.memsize") {
        unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut total).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            );
        }
    }
    if total == 0 {
        return Usage::default();
    }

    let mut vm_info: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    // The struct is a handful of integers; the element count fits the
    // message-type width by a wide margin.
    let mut count: libc::mach_msg_type_number_t = libc::mach_msg_type_number_t::try_from(
        std::mem::size_of::<libc::vm_statistics64>() / std::mem::size_of::<libc::integer_t>(),
    )
    .unwrap_or(libc::mach_msg_type_number_t::MAX);
    let host_port = unsafe { mach_host_self() };
    let ret = unsafe {
        libc::host_statistics64(
            host_port,
            libc::HOST_VM_INFO64,
            (&raw mut vm_info).cast(),
            &raw mut count,
        )
    };
    unsafe {
        mach_port_deallocate(mach2::traps::mach_task_self(), host_port);
    }

    if ret == libc::KERN_SUCCESS {
        let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = u64::try_from(ps).ok().filter(|&p| p > 0).unwrap_or(4096);
        let active = u64::from(vm_info.active_count) * page_size;
        let wire = u64::from(vm_info.wire_count) * page_size;
        let compressed = u64::from(vm_info.compressor_page_count) * page_size;
        let used = active + wire + compressed;
        Usage::new(total, used.min(total))
    } else {
        Usage::new(total, 0)
    }
}

#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn get_memory_macos() -> Usage {
    Usage::default()
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn get_net_macos() -> NetTotals {
    let mut totals = NetTotals::default();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&raw mut ifap) == 0 && !ifap.is_null() {
            let mut curr = ifap;
            while !curr.is_null() {
                let ifa = *curr;
                // `ifa_addr` is checked here too. getifaddrs(3) says the field
                // "may reference a NULL pointer" -- an interface with no address
                // assigned still appears in the list, with a name and with
                // ifa_data. The guard covered the other two pointers and then
                // dereferenced this one unconditionally, so such an interface
                // was a null dereference. It is a child process, so it would
                // take out local monitoring and reconnect rather than the TUI.
                if !ifa.ifa_name.is_null() && !ifa.ifa_data.is_null() && !ifa.ifa_addr.is_null() {
                    let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy();
                    if name != "lo0" && !name.starts_with("lo") {
                        let sa_family = (*ifa.ifa_addr).sa_family;
                        // `AF_LINK` is a small positive constant; `sa_family` is a u8.
                        if u32::from(sa_family) == u32::try_from(libc::AF_LINK).unwrap_or(u32::MAX)
                        {
                            let data = ifa.ifa_data as *const libc::if_data;
                            totals.rx = totals.rx.saturating_add(u64::from((*data).ifi_ibytes));
                            totals.tx = totals.tx.saturating_add(u64::from((*data).ifi_obytes));
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
#[must_use]
pub fn get_net_macos() -> NetTotals {
    NetTotals::default()
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn scan_macos() -> Vec<RawProcStat> {
    let mut out = Vec::with_capacity(crate::consts::IOKIT_SENSOR_CAPACITY);
    let num_pids = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if num_pids <= 0 {
        return out;
    }
    // `num_pids` is positive here (checked above); the buffer size in bytes is
    // what the kernel wants and fits `i32` for any real process count.
    let mut pids = vec![0i32; usize::try_from(num_pids).unwrap_or(0) + 64];
    let buf_bytes = i32::try_from(pids.len() * std::mem::size_of::<i32>()).unwrap_or(i32::MAX);
    let bytes_got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), buf_bytes) };
    let Ok(bytes_got) = usize::try_from(bytes_got) else {
        return out;
    };
    if bytes_got == 0 {
        return out;
    }
    let actual_count = bytes_got / std::mem::size_of::<i32>();
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let hz = u64::try_from(clk_tck)
        .ok()
        .filter(|&h| h > 0)
        .unwrap_or(100);

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
                (&raw mut task_info).cast(),
                i32::try_from(std::mem::size_of::<libc::proc_taskinfo>()).unwrap_or(i32::MAX),
            )
        };
        if res <= 0 {
            continue;
        }

        let mut name_buf = [0u8; crate::consts::SYSCTL_BUF];
        let name_len = u32::try_from(name_buf.len()).unwrap_or(u32::MAX);
        let name_res = unsafe { libc::proc_name(pid, name_buf.as_mut_ptr().cast(), name_len) };
        let comm = match usize::try_from(name_res) {
            Ok(n) if n > 0 => {
                String::from_utf8_lossy(&name_buf[..n.min(name_buf.len())]).to_string()
            }
            _ => format!("pid_{pid}"),
        };

        let total_ns = task_info.pti_total_user + task_info.pti_total_system;
        let ticks = (total_ns as u64) * hz / 1_000_000_000;
        let rss_pages = task_info.pti_resident_size / 4096;

        out.push(RawProcStat {
            // `pid > 0` was checked at the top of the loop.
            pid: u32::try_from(pid).unwrap_or(0),
            stat_comm: String::new(),
            comm,
            ticks,
            starttime: 0,
            rss_pages,
        });
    }
    out
}

#[cfg(not(target_os = "macos"))]
#[must_use]
pub const fn scan_macos() -> Vec<RawProcStat> {
    Vec::new()
}
