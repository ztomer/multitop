//! Core temperature sensing.
//!
//! Split out of `sys` because it is not what that module is: `sys` is the
//! non-Linux *sampling fallback*, while temperatures are their own concern and
//! are read on BOTH platforms -- through IOKit/HID on macOS and `/sys/class/hwmon`
//! on Linux. Keeping them together also kept `sys` over the file-length cap.

#![allow(
    unsafe_code,
    reason = "FFI boundary, and `cfg(macos)`-gated: on Linux the unsafe \
              vanishes and an `expect` would be unfulfilled, i.e. an error. \
              See the unsafe_code note in lib.rs"
)]

#[cfg(target_os = "macos")]
use std::collections::HashMap;

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn IOHIDEventSystemClientCreate(allocator: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn IOHIDEventSystemClientCopyServices(client: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn IOHIDServiceClientCopyProperty(
        service: *mut std::ffi::c_void,
        key: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
    fn IOHIDServiceClientCopyEvent(
        service: *mut std::ffi::c_void,
        event_type: i64,
        v0: u32,
        v1: u64,
    ) -> *mut std::ffi::c_void;
    fn IOHIDEventGetFloatValue(event: *mut std::ffi::c_void, field: u32) -> f64;
    fn CFArrayGetCount(array: *const std::ffi::c_void) -> isize;
    fn CFArrayGetValueAtIndex(
        array: *const std::ffi::c_void,
        index: isize,
    ) -> *const std::ffi::c_void;
    fn CFStringGetCString(
        cf: *const std::ffi::c_void,
        buffer: *mut u8,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
    fn CFStringCreateWithCString(
        allocator: *const std::ffi::c_void,
        c_str: *const std::ffi::c_char,
        encoding: u32,
    ) -> *const std::ffi::c_void;
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[cfg(target_os = "macos")]
#[must_use]
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "libc FFI widths — see the CASTS note at the top of this file"
)]
pub fn get_core_temps() -> HashMap<usize, f64> {
    let mut temps = HashMap::new();
    let mut die_temps: HashMap<usize, f64> = HashMap::new();
    let mut all_tdie = Vec::new();

    unsafe {
        let client = IOHIDEventSystemClientCreate(std::ptr::null());
        if client.is_null() {
            return temps;
        }
        let p_key = CFStringCreateWithCString(std::ptr::null(), c"Product".as_ptr(), 0x0800_0100);
        let services = IOHIDEventSystemClientCopyServices(client);
        if !services.is_null() {
            let count = CFArrayGetCount(services);
            for i in 0..count {
                let service = CFArrayGetValueAtIndex(services, i);
                let event = IOHIDServiceClientCopyEvent(service.cast_mut(), 15, 0, 0);
                if !event.is_null() {
                    let temp =
                        IOHIDEventGetFloatValue(event, crate::consts::HID_TEMPERATURE_PAGE << 16);
                    if (10.0..=120.0).contains(&temp) {
                        let prop = IOHIDServiceClientCopyProperty(service.cast_mut(), p_key);
                        if !prop.is_null() {
                            let mut buf = [0u8; crate::consts::IOKIT_NAME_BUF];
                            if CFStringGetCString(
                                prop,
                                buf.as_mut_ptr(),
                                crate::consts::IOKIT_NAME_BUF as _,
                                0x0800_0100,
                            ) {
                                let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                                let name = String::from_utf8_lossy(&buf[..len]);
                                if name.contains("tdie") {
                                    all_tdie.push(temp);
                                    if let Some(idx_str) = name.strip_prefix("PMU tdie") {
                                        if let Ok(idx) = idx_str.parse::<usize>() {
                                            die_temps.insert(idx.saturating_sub(1), temp);
                                        }
                                    }
                                }
                            }
                            CFRelease(prop);
                        }
                    }
                    CFRelease(event);
                }
            }
            CFRelease(services);
        }
        if !p_key.is_null() {
            CFRelease(p_key);
        }
        CFRelease(client);
    }

    let avg_temp = if all_tdie.is_empty() {
        0.0
    } else {
        all_tdie.iter().sum::<f64>() / all_tdie.len() as f64
    };

    if avg_temp > 0.0 {
        let num_cpus = macos_num_cpus();
        for i in 0..num_cpus {
            let t = die_temps.get(&i).copied().unwrap_or(avg_temp);
            temps.insert(i, t);
        }
    }

    temps
}

#[cfg(target_os = "macos")]
fn macos_num_cpus() -> usize {
    let mut count: u32 = 0;
    let mut size = std::mem::size_of::<u32>();
    if let Ok(name) = std::ffi::CString::new("hw.ncpu") {
        unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut count).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            );
        }
    }
    if count > 0 {
        count as usize
    } else {
        1
    }
}

/// Read temperatures for CPUs/cores from sysfs.
#[cfg(not(target_os = "macos"))]
pub fn get_core_temps() -> std::collections::HashMap<usize, f64> {
    use std::path::PathBuf;
    use std::sync::Mutex;

    static SENSOR_CACHE: Mutex<Option<Vec<(usize, PathBuf)>>> = Mutex::new(None);

    let mut guard = SENSOR_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if guard.is_none() {
        let mut sensors = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                if let Ok(files) = std::fs::read_dir(entry.path()) {
                    for f in files.flatten() {
                        let fname = f.file_name();
                        let fstr = fname.to_string_lossy();
                        if fstr.starts_with("temp") && fstr.ends_with("_input") {
                            let idx = fstr
                                .strip_prefix("temp")
                                .and_then(|s| s.strip_suffix("_input"))
                                .and_then(|s| s.parse::<usize>().ok())
                                .unwrap_or(1)
                                .saturating_sub(1);
                            sensors.push((idx, f.path()));
                        }
                    }
                }
            }
        }
        if sensors.is_empty() {
            if let Ok(entries) = std::fs::read_dir("/sys/class/thermal") {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("thermal_zone") {
                        let idx = name_str
                            .strip_prefix("thermal_zone")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        sensors.push((idx, entry.path().join("temp")));
                    }
                }
            }
        }
        *guard = Some(sensors);
    }

    let mut temps = std::collections::HashMap::new();
    if let Some(sensors) = guard.as_ref() {
        let mut buf = String::with_capacity(crate::consts::HWMON_READING_BUF);
        for (idx, path) in sensors {
            buf.clear();
            if crate::proc::read_proc_into(path, &mut buf) {
                if let Ok(val) = buf.trim().parse::<f64>() {
                    let c = if val > crate::consts::HWMON_MILLIDEGREE_THRESHOLD {
                        val / crate::consts::HWMON_MILLIDEGREE_THRESHOLD
                    } else {
                        val
                    };
                    if (0.0..=150.0).contains(&c) {
                        temps.entry(*idx).or_insert(c);
                    }
                }
            }
        }
    }

    // Release the cache lock before returning: the map is already built, and
    // holding a static Mutex across the return serialises every caller for no
    // reason (clippy::significant_drop_tightening).
    drop(guard);
    temps
}
