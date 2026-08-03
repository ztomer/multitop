//! Centralized constants for multitop TUI.

use std::time::Duration;

pub const KEYBAR_H: u16 = 1;
pub const SIDE_MARGIN: u16 = 1;

pub const MIN_AGENT_COLS: u16 = 40;
pub const MIN_AGENT_ROWS: u16 = 4;

pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(250);
pub const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];

pub const MAX_AUX_LINES: usize = 2000;
pub const MAX_STDERR_LINES: usize = 8;

pub const DEFAULT_PORT: u16 = 22;

/// The key that opens Server Settings, as the user must press it.
///
/// One constant because three separate help lines named two different keys --
/// `p` and `o` -- neither of which was bound to anything. They appeared at the
/// exact moment an operator was stuck and needed the instruction to work first
/// time. `tools/check_key_hints.py` is the gate that stops a fourth.
pub const SETTINGS_KEY: &str = "e";

pub const ANSI_YELLOW: &str = "\x1b[0;33m";
pub const ANSI_RED: &str = "\x1b[0;31m";
pub const ANSI_GRAY: &str = "\x1b[0;90m";
pub const ANSI_RESET: &str = "\x1b[0m";
