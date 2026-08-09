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

/// Bytes of fixed header on every agent packet: magic(4) + version(1) +
/// mode(1) + payload length(2). The reader and the writer have to agree, and
/// the writer's copy lives in the agent crate's `proto`.
pub const PACKET_HEADER_LEN: usize = 8;

/// Messages drained from the channel per pass of the event loop.
///
/// Bounded on purpose: one producer flooding the channel — an upgrade
/// streaming output — must not starve the key-event branch, or the UI reads
/// keys and never acts on them. The next pass drains the rest.
pub const MSG_DRAIN_BUDGET: usize = 32;

/// Lines of an upgrade's stderr kept for the failure message. apt writes its
/// progress display there too, so a bound is what stops a hundred rewrites of
/// one bar evicting the actual error.
pub const MAX_UPGRADE_ERR_LINES: usize = 100;

/// Below this width the Upgrade pane drops its closing rule: a rule is the
/// first thing worth losing when there is no room.
pub const UPGRADE_RULE_MIN_WIDTH: usize = 20;

/// SGR parameters kept from one escape sequence. Every sequence this agent
/// emits is far shorter; the cap is what stops a hostile one overrunning the
/// array.
pub const MAX_SGR_PARAMS: usize = 8;
/// The SGR sub-parameter that introduces a 256-colour index (`38;5;N`).
pub const SGR_INDEXED_COLOUR: u16 = 5;

/// Header of the embedded logo database: magic(4) + version(2) + count(2).
pub const LOGO_DB_HEADER_LEN: usize = 8;
/// Lines one rendered fetch panel starts out able to hold.
pub const FETCH_LINE_CAPACITY: usize = 12;
