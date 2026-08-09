//! What the cores are clocked at right now.
//!
//! Its own module because the answer comes from three different places
//! depending on the kernel and the hardware, and because none of them exist on
//! every machine -- so every function here returns an `Option` and the caller
//! is expected to say "not measured" rather than to substitute a zero. A clock
//! of 0 MHz on screen is a measurement; this is the absence of one.

use crate::proc::read_proc;

/// The clock a `cpufreq` reading is in: kilohertz.
const KHZ_PER_MHZ: f64 = 1000.0;

/// Mean of the `scaling_cur_freq` readings handed in, in MHz.
///
/// The mean rather than core 0: on a big.LITTLE machine core 0 is an efficiency
/// core and reporting it as "the" clock understates the box by a factor of two.
#[must_use]
pub fn parse_scaling_khz(readings: &[String]) -> Option<f64> {
    let mhz: Vec<f64> = readings
        .iter()
        .filter_map(|r| r.trim().parse::<f64>().ok())
        .filter(|khz| *khz > 0.0)
        .map(|khz| khz / KHZ_PER_MHZ)
        .collect();
    if mhz.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some(mhz.iter().sum::<f64>() / mhz.len() as f64)
}

/// Mean of the `cpu MHz` lines in `/proc/cpuinfo`.
///
/// The fallback for a kernel built without `cpufreq`, and for a container that
/// cannot see `/sys`. Some ARM kernels omit the field entirely, which is why
/// this returns an `Option` rather than a zero -- a clock of 0 MHz on screen is
/// a measurement, and this is the absence of one.
#[must_use]
pub fn parse_cpuinfo_mhz(content: &str) -> Option<f64> {
    let values: Vec<f64> = content
        .lines()
        .filter_map(|l| {
            let (key, value) = l.split_once(':')?;
            key.trim().eq_ignore_ascii_case("cpu MHz").then_some(value)
        })
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .filter(|mhz| *mhz > 0.0)
        .collect();
    if values.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

/// What the cores are clocked at right now, in MHz.
///
/// `cpufreq` first, because it is the live value; `/proc/cpuinfo` second, which
/// on many kernels is also live and on others is the nominal base clock. `None`
/// on a machine that publishes neither -- notably Apple Silicon, which exposes
/// no current-frequency reading at all, so the panel says so rather than
/// inventing a number.
#[must_use]
pub fn get_cpu_mhz() -> Option<f64> {
    let mut readings = Vec::new();
    for cpu in 0..crate::consts::CPUFREQ_PROBE_CORES {
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq");
        let text = read_proc(&path);
        if text.is_empty() {
            break;
        }
        readings.push(text);
    }
    parse_scaling_khz(&readings).or_else(|| parse_cpuinfo_mhz(&read_proc("/proc/cpuinfo")))
}
