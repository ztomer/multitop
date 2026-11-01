//! What the cores are clocked at right now.
//!
//! Its own module because the answer comes from three different places
//! depending on the kernel and the hardware, and because none of them exist on
//! every machine -- so every function here returns an `Option` and the caller
//! is expected to say "not measured" rather than to substitute a zero. A clock
//! of 0 MHz on screen is a measurement; this is the absence of one.

use crate::conv::count;
use crate::proc_read::{fmt_into, for_each_line, read_proc_bytes};

/// The clock a `cpufreq` reading is in: kilohertz.
const KHZ_PER_MHZ: f64 = 1000.0;

/// A mean that states nothing until it has a reading. Readings of zero or
/// below are not readings: a clock of 0 MHz on screen is a measurement, and a
/// zero in these files is the absence of one.
#[derive(Default)]
struct Mean {
    sum: f64,
    n: usize,
}

impl Mean {
    fn add(&mut self, v: Option<f64>) {
        if let Some(v) = v.filter(|v| *v > 0.0) {
            self.sum += v;
            self.n += 1;
        }
    }

    fn get(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / count(self.n))
    }
}

/// Mean of the `scaling_cur_freq` readings under `root` (`cpu0`, `cpu1`, ...
/// up to the first core without one), in MHz -- read through stack buffers,
/// with no heap work, because this runs every tick.
///
/// The mean rather than core 0: on a big.LITTLE machine core 0 is an efficiency
/// core and reporting it as "the" clock understates the box by a factor of two.
#[must_use]
pub fn scaling_mhz_under(root: &str) -> Option<f64> {
    let mut mean = Mean::default();
    for cpu in 0..crate::consts::CPUFREQ_PROBE_CORES {
        let mut path = [0u8; crate::consts::SYSFS_PATH_BUF];
        let Some(path) = fmt_into(
            &mut path,
            format_args!("{root}/cpu{cpu}/cpufreq/scaling_cur_freq"),
        ) else {
            break;
        };
        let mut buf = [0u8; crate::consts::HWMON_READING_BUF];
        let n = read_proc_bytes(path, &mut buf);
        if n == 0 {
            break;
        }
        mean.add(
            std::str::from_utf8(&buf[..n])
                .ok()
                .and_then(|r| r.trim().parse::<f64>().ok())
                .map(|khz| khz / KHZ_PER_MHZ),
        );
    }
    mean.get()
}

/// Mean of the `cpu MHz` lines in the `/proc/cpuinfo` at `path`.
///
/// The fallback for a kernel built without `cpufreq`, and for a container that
/// cannot see `/sys`. Some ARM kernels omit the field entirely, which is why
/// this returns an `Option` rather than a zero -- a clock of 0 MHz on screen is
/// a measurement, and this is the absence of one. Streamed a line at a time:
/// the file is tens of kilobytes on a many-core host, and read every tick.
#[must_use]
pub fn cpuinfo_mhz_from(path: &str) -> Option<f64> {
    let mut mean = Mean::default();
    for_each_line(path, |l| {
        mean.add(l.split_once(':').and_then(|(key, value)| {
            key.trim()
                .eq_ignore_ascii_case("cpu MHz")
                .then(|| value.trim().parse::<f64>().ok())
                .flatten()
        }));
    });
    mean.get()
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
    scaling_mhz_under("/sys/devices/system/cpu").or_else(|| cpuinfo_mhz_from("/proc/cpuinfo"))
}
