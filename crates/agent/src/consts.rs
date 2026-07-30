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
