//! Platform-specific sampling fallback for non-Linux hosts (e.g. macOS).
//!
//! CASTS AT THE libc BOUNDARY. The `*_macos` functions carry narrow `#[expect]`
//! lists: the syscall signatures fix these widths and every return is
//! range-checked first (`num_pids <= 0`, `bytes_got <= 0` both bail). Per
//! function, not per module — `expect` errors when a declared lint does NOT
//! fire, which narrowed these lists and stops a blanket suppression.

#![allow(
    unsafe_code,
    reason = "FFI boundary, and `cfg(macos)`-gated: on Linux the unsafe \
              vanishes and an `expect` would be unfulfilled, i.e. an error. \
              See the unsafe_code note in lib.rs"
)]
#![allow(deprecated)]

use crate::proc::{CpuStat, NetTotals, RawProcStat, Usage};
// Only the macOS sampler builds `CpuTimes` values; on Linux the import is dead.
// Naming that with a cfg is what let the blanket `#[allow(unused_imports)]`
// that used to sit here go away -- it was hiding exactly one real fact.
#[cfg(target_os = "macos")]
use crate::proc::CpuTimes;

#[cfg(target_os = "macos")]
extern "C" {
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
                let vm_map = libc::mach_task_self();
                let size = self.msg_type as usize * std::mem::size_of::<libc::integer_t>();
                libc::vm_deallocate(
                    vm_map,
                    self.cpu_info as libc::vm_address_t,
                    size as libc::vm_size_t,
                );
            }
            if self.host_port != 0 {
                mach_port_deallocate(libc::mach_task_self(), self.host_port);
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

    let host_port = unsafe { libc::mach_host_self() };
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
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "libc FFI widths — see the CASTS note at the top of this file"
)]
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
    let mut count = (std::mem::size_of::<libc::vm_statistics64>()
        / std::mem::size_of::<libc::integer_t>())
        as libc::mach_msg_type_number_t;
    let host_port = unsafe { libc::mach_host_self() };
    let ret = unsafe {
        libc::host_statistics64(
            host_port,
            libc::HOST_VM_INFO64,
            (&raw mut vm_info).cast(),
            &raw mut count,
        )
    };
    unsafe {
        mach_port_deallocate(libc::mach_task_self(), host_port);
    }

    if ret == libc::KERN_SUCCESS {
        let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = if ps > 0 { ps as u64 } else { 4096 };
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
#[expect(
    clippy::cast_possible_truncation,
    reason = "libc FFI widths — see the CASTS note at the top of this file"
)]
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
                        if sa_family == libc::AF_LINK as u8 {
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
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "libc FFI widths — see the CASTS note at the top of this file"
)]
pub fn scan_macos() -> Vec<RawProcStat> {
    let mut out = Vec::with_capacity(crate::consts::IOKIT_SENSOR_CAPACITY);
    let num_pids = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if num_pids <= 0 {
        return out;
    }
    let mut pids = vec![0i32; num_pids as usize + 64];
    let bytes_got = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
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
                (&raw mut task_info).cast(),
                std::mem::size_of::<libc::proc_taskinfo>() as i32,
            )
        };
        if res <= 0 {
            continue;
        }

        let mut name_buf = [0u8; crate::consts::SYSCTL_BUF];
        let name_res =
            unsafe { libc::proc_name(pid, name_buf.as_mut_ptr().cast(), name_buf.len() as u32) };
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
