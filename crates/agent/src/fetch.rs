//! System information sampling.
//!
//! Data collection only — rendering (including logo lookup) lives in the
//! monitor crate's `fetch_render` module to keep the agent binary small.

use std::path::Path;

use crate::fmt::fmt_size;
use crate::proc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchSnapshot {
    pub user_host: String,
    pub os: String,
    pub kernel: String,
    pub uptime: String,
    pub host_model: String,
    pub cpu_model: String,
    pub memory_str: String,
    pub disk_str: String,
}

pub fn sample_uptime() -> String {
    let raw = proc::read_proc("/proc/uptime");
    if let Some(sec_str) = raw.split_ascii_whitespace().next() {
        if let Ok(secs) = sec_str.parse::<f64>() {
            let total_sec = secs as u64;
            let days = total_sec / 86400;
            let hours = (total_sec % 86400) / 3600;
            let mins = (total_sec % 3600) / 60;
            if days > 0 {
                return format!("{days}d {hours}h {mins}m");
            } else if hours > 0 {
                return format!("{hours}h {mins}m");
            } else {
                return format!("{mins}m");
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut boot_time: libc::timeval = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::timeval>();
        let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
        let res = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                2,
                &mut boot_time as *mut _ as *mut _,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if res == 0 && boot_time.tv_sec > 0 {
            let now = unsafe { libc::time(std::ptr::null_mut()) };
            let elapsed = now.saturating_sub(boot_time.tv_sec) as u64;
            let days = elapsed / 86400;
            let hours = (elapsed % 86400) / 3600;
            let mins = (elapsed % 3600) / 60;
            if days > 0 {
                return format!("{days}d {hours}h {mins}m");
            } else if hours > 0 {
                return format!("{hours}h {mins}m");
            } else {
                return format!("{mins}m");
            }
        }
    }

    "unknown".to_string()
}

pub fn sample_os() -> String {
    if Path::new("/etc/os-release").exists() {
        let content = proc::read_proc("/etc/os-release");
        let mut pretty = String::new();
        let mut name = String::new();
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                pretty = val.trim_matches('"').to_string();
            } else if let Some(val) = line.strip_prefix("NAME=") {
                name = val.trim_matches('"').to_string();
            }
        }
        if !pretty.is_empty() {
            return pretty;
        } else if !name.is_empty() {
            return name;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut os_rev = [0u8; 256];
        let mut size = os_rev.len();
        let name = std::ffi::CString::new("kern.osproductversion").unwrap();
        let res = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                os_rev.as_mut_ptr() as *mut _,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if res == 0 && size > 0 {
            let ver = String::from_utf8_lossy(&os_rev[..size.saturating_sub(1)]).to_string();
            return format!("macOS {ver}");
        }
        "macOS".to_string()
    }

    #[cfg(not(target_os = "macos"))]
    "Linux".to_string()
}

pub fn sample_kernel() -> String {
    let ver = proc::read_proc("/proc/sys/kernel/osrelease").trim().to_string();
    if !ver.is_empty() {
        return ver;
    }

    #[cfg(target_os = "macos")]
    {
        let mut k_ver = [0u8; 256];
        let mut size = k_ver.len();
        let name = std::ffi::CString::new("kern.osrelease").unwrap();
        let res = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                k_ver.as_mut_ptr() as *mut _,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if res == 0 && size > 0 {
            let ver = String::from_utf8_lossy(&k_ver[..size.saturating_sub(1)]).to_string();
            return format!("Darwin {ver}");
        }
    }

    "unknown".to_string()
}

pub fn sample_host_model() -> String {
    let sys_vendor = proc::read_proc("/sys/class/dmi/id/sys_vendor").trim().to_string();
    let product_name = proc::read_proc("/sys/class/dmi/id/product_name").trim().to_string();
    if !product_name.is_empty() {
        if !sys_vendor.is_empty() && !product_name.starts_with(&sys_vendor) {
            return format!("{sys_vendor} {product_name}");
        }
        return product_name;
    }
    let dt_model = proc::read_proc("/sys/firmware/devicetree/base/model").trim_matches('\0').trim().to_string();
    if !dt_model.is_empty() {
        return dt_model;
    }

    #[cfg(target_os = "macos")]
    {
        let mut model_buf = [0u8; 256];
        let mut size = model_buf.len();
        let name = std::ffi::CString::new("hw.model").unwrap();
        let res = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                model_buf.as_mut_ptr() as *mut _,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if res == 0 && size > 0 {
            return String::from_utf8_lossy(&model_buf[..size.saturating_sub(1)]).to_string();
        }
    }

    "Generic Hardware".to_string()
}

pub fn sample_cpu_model() -> String {
    let cpuinfo = proc::read_proc("/proc/cpuinfo");
    let mut model = String::new();
    let mut count = 0;
    for line in cpuinfo.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k_trim = k.trim();
            if k_trim == "model name" || k_trim == "Hardware" || k_trim == "Processor" {
                if model.is_empty() {
                    model = v.trim().to_string();
                }
            } else if k_trim == "processor" {
                count += 1;
            }
        }
    }
    if !model.is_empty() {
        let cores = if count > 0 { count } else { 1 };
        return format!("{model} ({cores})");
    }

    #[cfg(target_os = "macos")]
    {
        let mut cpu_buf = [0u8; 256];
        let mut size = cpu_buf.len();
        let name = std::ffi::CString::new("machdep.cpu.brand_string").unwrap();
        let res = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                cpu_buf.as_mut_ptr() as *mut _,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        let mut num_cores: libc::natural_t = 0;
        let mut c_size = std::mem::size_of::<libc::natural_t>();
        let c_name = std::ffi::CString::new("hw.logicalcpu").unwrap();
        let _ = unsafe {
            libc::sysctlbyname(
                c_name.as_ptr(),
                &mut num_cores as *mut _ as *mut _,
                &mut c_size,
                std::ptr::null_mut(),
                0,
            )
        };
        let c_str = if num_cores > 0 { num_cores } else { 1 };
        if res == 0 && size > 0 {
            let brand = String::from_utf8_lossy(&cpu_buf[..size.saturating_sub(1)]).to_string();
            return format!("{brand} ({c_str})");
        }
        format!("Apple Silicon ({c_str})")
    }

    #[cfg(not(target_os = "macos"))]
    "Generic CPU".to_string()
}

pub fn sample_fetch(host: &str) -> FetchSnapshot {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let user_host = format!("{user}@{host}");

    let mem = proc::get_memory();
    let mem_str = if mem.total > 0 {
        format!("{}/{} ({:.0}%)", fmt_size(mem.used), fmt_size(mem.total), mem.pct)
    } else {
        "-".into()
    };

    let disk = proc::get_disk();
    let disk_str = if disk.total > 0 {
        format!("{}/{} ({:.0}%)", fmt_size(disk.used), fmt_size(disk.total), disk.pct)
    } else {
        "-".into()
    };

    FetchSnapshot {
        user_host,
        os: sample_os(),
        kernel: sample_kernel(),
        uptime: sample_uptime(),
        host_model: sample_host_model(),
        cpu_model: sample_cpu_model(),
        memory_str: mem_str,
        disk_str,
    }
}
