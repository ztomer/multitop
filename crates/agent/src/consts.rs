//! Centralized constants for multitop-agent.

/// Frame marker on the wire.
pub const FRAME_MARKER: &str = "===MONITOR===";

/// Agent binary version (set from Cargo.toml at compile time).
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default refresh interval in seconds.
pub const DEFAULT_INTERVAL: f64 = 2.0;

/// Process table layout dimensions.
pub const PID_W: usize = 7;
pub const CPU_W: usize = 5;
pub const GAP: usize = 2;
pub const COL_SEP_W: usize = 3;
pub const INDENT: usize = 1;

pub const TWO_COLUMN_MIN_COLS: usize = 72;
pub const NAME_W_MIN: usize = 12;
pub const NAME_W_MAX: usize = 20;
pub const NAME_W_MIN_SPLIT: usize = 4;

/// Docker view layout dimensions.
pub const DOCKER_NAME_W: usize = 20;
pub const DOCKER_STATUS_W: usize = 16;
pub const DOCKER_CPU_W: usize = 7;
pub const DOCKER_CHROME_ROWS: usize = 4;

/// Thermal paths for Linux system monitoring.
pub const HWMON_GLOB: &str = "/sys/class/hwmon/hwmon*/temp*_input";
pub const THERMAL_ZONE_GLOB: &str = "/sys/class/thermal/thermal_zone*/temp";

/// Meter colour thresholds, as percentages. Each meter has its own pair
/// because "hot" means something different for a CPU than for a disk: a disk
/// at 70% is fine, a disk at 90% is about to stop being fine.
pub const CPU_HIGH_PCT: f64 = 80.0;
pub const CPU_MID_PCT: f64 = 50.0;
pub const MEM_HIGH_PCT: f64 = 85.0;
pub const MEM_MID_PCT: f64 = 50.0;
pub const DISK_HIGH_PCT: f64 = 90.0;
pub const DISK_MID_PCT: f64 = 70.0;

/// A container's CPU share, as a percentage, above which its row is coloured.
pub const DOCKER_CPU_HIGH_PCT: f64 = 80.0;
pub const DOCKER_CPU_MID_PCT: f64 = 20.0;

/// A process is worth colouring once it is using this much of one core.
pub const PROC_BUSY_PCT: f64 = 10.0;
/// Per-core temperature thresholds, in degrees Celsius.
pub const CORE_TEMP_HIGH_C: f64 = 75.0;
pub const CORE_TEMP_WARM_C: f64 = 55.0;
/// Network traffic below this many bytes per second is not worth a row.
pub const NET_VISIBLE_BYTES_PER_SEC: f64 = 1024.0;

/// Buffer sizes for the pseudofiles each sampler reads. Sized to hold the
/// whole file in one read on a large host, because a short read means a
/// truncated line and a truncated line means a wrong number.
pub const PROC_STAT_BUF: usize = 4096;
pub const PROC_MEMINFO_BUF: usize = 2048;
pub const PROC_NET_DEV_BUF: usize = 2048;
pub const PROC_MOUNTINFO_BUF: usize = 4096;
pub const PROC_CHUNK_BUF: usize = 1024;
pub const PROC_LINE_CAPACITY: usize = 512;
/// `sysctl` string replies: names, models and kernel versions all fit.
pub const SYSCTL_BUF: usize = 256;

/// Columns of `/proc/<pid>/stat`, counted from the field after `comm`. Reading
/// the wrong index is silent: a plausible number, attributed to the wrong
/// thing.
pub const PROC_STAT_UTIME_FIELD: usize = 11;
pub const PROC_STAT_STARTTIME_FIELD: usize = 6;
/// Columns of one `/proc/net/dev` row between the receive and transmit byte
/// counters.
pub const NET_DEV_TX_OFFSET: usize = 7;
/// A `/proc/stat` cpu line without idle and iowait cannot yield a busy
/// percentage, so this many columns is the minimum usable row.
pub const PROC_STAT_MIN_FIELDS: usize = 5;

/// A `/proc/<pid>/...` path built on the stack: `/proc/` plus ten digits of
/// PID plus the longest suffix this agent asks for.
pub const PROC_PATH_BUF: usize = 48;
/// Decimal digits a `u32` PID can take, with room to spare.
pub const PID_DIGITS_BUF: usize = 16;
/// `/proc/<pid>/comm` holds a truncated command name.
pub const PROC_COMM_BUF: usize = 64;
/// One `/proc/<pid>/stat` line.
pub const PROC_PID_STAT_BUF: usize = 512;
/// Processes to reserve room for before the first scan. A busy host has more,
/// and the vectors grow; a quiet one has fewer, and this costs nothing.
pub const PROC_SCAN_CAPACITY: usize = 64;
/// One HTTP response from the Docker daemon, and one repainted terminal frame.
pub const HTTP_RESPONSE_CAPACITY: usize = 8192;
pub const FRAME_BUF_CAPACITY: usize = 8192;
/// One encoded protocol packet, and one rendered process row.
pub const PACKET_CAPACITY: usize = 512;
pub const PROC_ROW_CAPACITY: usize = 96;
/// Lines a rendered frame starts out able to hold.
pub const FRAME_LINE_CAPACITY: usize = 16;
/// Bytes read from stdin at a time while watching for the reader to hang up.
pub const STDIN_WATCH_BUF: usize = 64;

/// A per-core bar narrower than this is unreadable, so the bars are dropped
/// and the grid shows figures alone.
pub const MIN_READABLE_BAR_W: usize = 5;
/// IOKit sensor plumbing: the HID page for thermal events, and the buffer a
/// sensor name is copied into.
pub const HID_TEMPERATURE_PAGE: u32 = 15;
pub const IOKIT_NAME_BUF: usize = 128;
/// Sensors reachable at once on a Mac, before the vector grows.
pub const IOKIT_SENSOR_CAPACITY: usize = 256;
/// One `hwmon` temperature reading, as text.
pub const HWMON_READING_BUF: usize = 32;
/// `hwmon` reports millidegrees; a value above this is millidegrees rather
/// than degrees, and dividing is what makes the two agree.
pub const HWMON_MILLIDEGREE_THRESHOLD: f64 = 1000.0;
