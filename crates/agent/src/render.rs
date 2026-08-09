//! Panel rendering and the layout arithmetic behind it.
//!
//! The renderer and the height/budget calculations share one geometry
//! implementation ([`CoreGrid`], [`Chrome`]). They used to be separate copies
//! of the same formulas, which is how the Python original ended up
//! under-counting per-core rows and overflowing the panel: its
//! `_calc_core_rows` omitted the one-column gutter that `_render_output`
//! added. Anything that needs to know how tall a frame is must go through
//! these types, never re-derive it.

use std::fmt::Write as _;

use crate::color::{strip_ansi, Palette};
use crate::fmt::{center_header, core_bar, fmt_rate, fmt_size, make_bar, SIZE_PAIR_W, SIZE_W};
use crate::proc::{Proc, Usage};

/// Width of the `used/total` field on the MEM and DSK rows.
const SIZE_FIELD_W: usize = SIZE_PAIR_W;

/// Process table column widths.
const PID_W: usize = 7;
const CPU_W: usize = 5;
/// Sized so the column never stretches and shifts the row.
const MEM_W: usize = SIZE_W;
/// Spacing between process table columns.
const GAP: usize = 2;
/// ` \u{2502} ` between the two halves of a wide table.
const COL_SEP_W: usize = 3;
/// Leading indent on every body row.
const INDENT: usize = 1;

/// Everything but the name in one process row.
const CELL_FIXED_W: usize = PID_W + GAP + GAP + CPU_W + GAP + MEM_W;

/// Below this width a second process column leaves no room for a name.
const TWO_COLUMN_MIN_COLS: usize = 72;

const NAME_W_MIN: usize = 12;
const NAME_W_MAX: usize = 20;
const NAME_W_MIN_SPLIT: usize = 4;

/// Everything one frame draws, as a single value.
///
/// Grouping these rather than passing fourteen positional arguments is what
/// lets callers diff whole frames for equality instead of tracking which
/// individual field changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TempUnit {
    #[default]
    C,
    F,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub host: String,
    pub agent_version: String,
    pub cpu_pct: f64,
    /// What the cores are clocked at right now, in MHz, or `None` on a machine
    /// that publishes no current-frequency reading.
    pub cpu_mhz: Option<f64>,
    /// (core index, busy percent, temp deg C), ascending by index
    pub cores: Vec<(usize, f64, Option<f64>)>,
    pub temp_unit: TempUnit,
    pub mem: Usage,
    pub disk: Usage,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub procs: Vec<Proc>,
}

/// Bar width for the aggregate rows, derived from the panel width.
pub fn bar_len_for(cols: usize) -> usize {
    cols.saturating_sub(32).max(4)
}

/// The NET row only appears once there is traffic worth reporting.
pub fn shows_net(rx_rate: f64, tx_rate: f64) -> bool {
    rx_rate > crate::consts::NET_VISIBLE_BYTES_PER_SEC
        || tx_rate > crate::consts::NET_VISIBLE_BYTES_PER_SEC
}

fn horizontal_rule(n: usize) -> &'static str {
    static RULE_CACHE: &str = concat!(
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );
    let count = n.min(256);
    let end_byte = count * 3;
    &RULE_CACHE[..end_byte]
}

// ------------------------------------------------------------------ geometry

pub use crate::render_layout::*;

/// Rows a process table spends on its own rule and header before the first
/// process fits.
const TABLE_CHROME_ROWS: usize = 2;

impl Chrome {
    fn two_column(&self, num_procs: usize) -> bool {
        self.cols >= TWO_COLUMN_MIN_COLS && num_procs > 1
    }

    /// Rows a process table of `n` entries occupies, including its rule and
    /// header. An empty table draws nothing at all.
    pub fn table_height(&self, n: usize) -> usize {
        if n == 0 || self.tier < Tier::Compact {
            return 0;
        }
        let body = if self.two_column(n) { n.div_ceil(2) } else { n };
        TABLE_CHROME_ROWS + body
    }

    /// How many processes to sample so the table fills `lines` exactly.
    ///
    /// Returns 0 when the panel has no room for a table — a clipped frame is
    /// worse than an honestly omitted one.
    pub fn proc_budget(&self, lines: usize) -> usize {
        if self.tier < Tier::Compact {
            return 0;
        }
        if lines == 0 {
            return usize::MAX;
        }
        let available = lines.saturating_sub(self.height());
        // Two rows go to the rule and header before any process fits.
        let Some(body) = available.checked_sub(TABLE_CHROME_ROWS).filter(|n| *n > 0) else {
            return 0;
        };
        let estimate = if self.cols >= TWO_COLUMN_MIN_COLS {
            body * 2
        } else {
            body
        };
        // Checked against `table_height`, which is the same relationship read
        // the other way round. Two independent copies of one arithmetic is how
        // a budget and the height it is supposed to produce drift apart, and
        // the symptom of that is a clipped frame — a pane sized from a number
        // that no longer describes what gets drawn.
        let mut n = estimate;
        while n > 0 && self.table_height(n) > available {
            n -= 1;
        }
        n
    }
}

/// Name column width for the process table.
fn name_width(cols: usize, two_column: bool) -> usize {
    if two_column {
        let fixed = INDENT + COL_SEP_W + 2 * CELL_FIXED_W;
        (cols.saturating_sub(fixed) / 2).max(NAME_W_MIN_SPLIT)
    } else {
        cols.saturating_sub(INDENT + CELL_FIXED_W)
            .clamp(NAME_W_MIN, NAME_W_MAX)
    }
}

// ----------------------------------------------------------------- rendering

/// Truncate to `width` display cells, marking the cut with an ellipsis.
pub fn truncate_name(name: &str, width: usize) -> String {
    if name.chars().count() < width {
        return name.to_string();
    }
    let mut s: String = name.chars().take(width.saturating_sub(3)).collect();
    s.push_str("...");
    s
}

fn proc_cell(p: &Proc, name_w: usize, pal: &Palette) -> String {
    let cpu_c = if p.cpu >= crate::consts::PROC_BUSY_PCT {
        pal.meter_mid()
    } else {
        pal.text()
    };
    let mut s = String::with_capacity(crate::consts::PROC_ROW_CAPACITY);
    let _ = write!(
        s,
        "{}{:>PID_W$}{}  {}{:<name_w$}{}  {}{:>CPU_W$.1}{}  {}{:>MEM_W$}{}",
        pal.muted(),
        p.pid,
        pal.reset,
        pal.text(),
        truncate_name(&p.name, name_w),
        pal.reset,
        cpu_c,
        p.cpu,
        pal.reset,
        pal.primary(),
        fmt_size(p.mem),
        pal.reset,
    );
    s
}

fn proc_header(name_w: usize, pal: &Palette) -> String {
    format!(
        "{}{}{:>PID_W$}  {:<name_w$}  {:>CPU_W$}  {:>MEM_W$}{}",
        pal.primary(),
        pal.bold,
        "PID",
        "NAME",
        "CPU",
        "MEM",
        pal.reset,
    )
}

fn push_core_rows(
    out: &mut Vec<String>,
    cores: &[(usize, f64, Option<f64>)],
    unit: TempUnit,
    cols: usize,
    bar_len: usize,
    pal: &Palette,
) {
    let max_idx = cores.iter().map(|(i, _, _)| *i).max().unwrap_or(0);
    let has_temps = cores.iter().any(|(_, _, t)| t.is_some());
    let grid = CoreGrid::new(max_idx, cores.len(), cols, bar_len, has_temps);

    let segs: Vec<String> = cores
        .iter()
        .map(|&(idx, cp, temp)| {
            let temp_str = match temp {
                Some(c) => {
                    let tc = if c >= crate::consts::CORE_TEMP_HIGH_C {
                        pal.meter_high()
                    } else if c >= crate::consts::CORE_TEMP_WARM_C {
                        pal.meter_mid()
                    } else {
                        pal.meter_low()
                    };
                    match unit {
                        TempUnit::F => format!(" {}{:.0}°F{}", tc, c * 1.8 + 32.0, pal.reset),
                        TempUnit::C => format!(" {}{:.0}°C{}", tc, c, pal.reset),
                    }
                }
                None => String::new(),
            };
            if grid.show_bars {
                format!(
                    "{:>w$}:{}{:3.0}%{}",
                    idx,
                    core_bar(cp, grid.bar_len, pal),
                    cp,
                    temp_str,
                    w = grid.idx_w
                )
            } else {
                format!("{:>w$}:{:3.0}%{}", idx, cp, temp_str, w = grid.idx_w)
            }
        })
        .collect();

    const LABEL_W: usize = " CPU ".len();
    let mut i = 0;
    while i < cores.len() {
        let mut row = if i == 0 {
            format!(" {}{}CPU{} ", pal.primary(), pal.bold, pal.reset)
        } else {
            " ".repeat(LABEL_W)
        };
        for seg in &segs[i..(i + grid.num_cols).min(cores.len())] {
            // Pad by the raw length so the *visible* width lands on cell_w.
            let overhead = seg.chars().count() - strip_ansi(seg).chars().count();
            let _ = write!(row, "{:<pad$}", seg, pad = grid.cell_w + overhead);
        }
        out.push(row.trim_end().to_string());
        i += grid.num_cols;
    }
}

fn push_proc_table(out: &mut Vec<String>, procs: &[Proc], cols: usize, pal: &Palette) {
    if procs.is_empty() {
        return;
    }
    out.push(format!(
        " {}{}{}",
        pal.secondary(),
        horizontal_rule(cols.saturating_sub(2)),
        pal.reset
    ));

    let two_column = cols >= TWO_COLUMN_MIN_COLS && procs.len() > 1;
    let name_w = name_width(cols, two_column);
    let hdr = proc_header(name_w, pal);

    if two_column {
        let sep = format!(" {}\u{2502}{} ", pal.secondary(), pal.reset);
        out.push(format!(" {hdr}{sep}{hdr}"));

        let mid = procs.len().div_ceil(2);
        for i in 0..mid {
            let left = proc_cell(&procs[i], name_w, pal);
            match procs.get(i + mid) {
                Some(right) => out.push(format!(" {left}{sep}{}", proc_cell(right, name_w, pal))),
                None => out.push(format!(" {left}")),
            }
        }
    } else {
        out.push(format!(" {hdr}"));
        for p in procs {
            out.push(format!(" {}", proc_cell(p, name_w, pal)));
        }
    }
}

/// Render one frame.
pub fn render(
    snap: &Snapshot,
    cols: usize,
    lines: usize,
    bar_len: usize,
    pal: &Palette,
) -> Vec<String> {
    let chrome = Chrome::of(snap, cols, lines);
    let tier = chrome.tier;
    let mut out: Vec<String> = Vec::with_capacity(crate::consts::FRAME_LINE_CAPACITY);

    out.push(center_header(&snap.host, cols, pal));

    match tier {
        Tier::TooSmall => {
            if lines > 1 {
                out.push(format!(" {}too small{}", pal.muted(), pal.reset));
            }
        }
        Tier::Micro => {
            let mut summary = Vec::new();
            summary.push(format!("CPU {:.0}%", snap.cpu_pct));
            if snap.mem.total > 0 {
                summary.push(format!("MEM {:.0}%", snap.mem.pct));
            }
            if snap.disk.total > 0 {
                summary.push(format!("DSK {:.0}%", snap.disk.pct));
            }
            out.push(format!(
                " {}{}{}",
                pal.muted(),
                summary.join("  "),
                pal.reset
            ));
        }
        Tier::Minimal | Tier::Compact | Tier::Full => {
            if tier == Tier::Full && snap.cores.len() >= 2 {
                push_core_rows(&mut out, &snap.cores, snap.temp_unit, cols, bar_len, pal);
            } else {
                let bc = pal.cpu_bar(snap.cpu_pct);
                out.push(format!(
                    " {}{}CPU{} {} {}{:.0}%{}",
                    pal.primary(),
                    pal.bold,
                    pal.reset,
                    make_bar(snap.cpu_pct, bar_len, bc, pal.reset),
                    bc,
                    snap.cpu_pct,
                    pal.reset,
                ));
            }

            for (label, usage, color, label_color) in [
                ("MEM", snap.mem, pal.mem_bar(snap.mem.pct), pal.primary()),
                (
                    "DSK",
                    snap.disk,
                    pal.disk_bar(snap.disk.pct),
                    pal.secondary(),
                ),
            ] {
                if usage.total == 0 {
                    continue;
                }
                let size = format!("{}/{}", fmt_size(usage.used), fmt_size(usage.total));
                out.push(format!(
                    " {}{}{}{} {} {}{:3.0}%{} {}{:<w$}{}",
                    label_color,
                    pal.bold,
                    label,
                    pal.reset,
                    make_bar(usage.pct, bar_len, color, pal.reset),
                    color,
                    usage.pct,
                    pal.reset,
                    pal.muted(),
                    size,
                    pal.reset,
                    w = SIZE_FIELD_W,
                ));
            }

            if tier == Tier::Full && shows_net(snap.rx_rate, snap.tx_rate) {
                out.push(format!(
                    " {}{}NET{} {}\u{2191} {}{}  {}\u{2193} {}{}",
                    pal.secondary(),
                    pal.bold,
                    pal.reset,
                    pal.primary(),
                    fmt_rate(snap.tx_rate),
                    pal.reset,
                    pal.meter_low(),
                    fmt_rate(snap.rx_rate),
                    pal.reset,
                ));
            }

            if tier >= Tier::Compact {
                let budget = chrome.proc_budget(lines);
                if budget > 0 && !snap.procs.is_empty() {
                    let show = budget.min(snap.procs.len());
                    push_proc_table(&mut out, &snap.procs[..show], cols, pal);
                }
            }
        }
    }

    out
}

/// Render one frame directly into a string buffer.
pub fn render_to_buf(
    snap: &Snapshot,
    cols: usize,
    lines: usize,
    bar_len: usize,
    pal: &Palette,
    buf: &mut String,
) {
    let frame = render(snap, cols, lines, bar_len, pal);
    for line in &frame {
        buf.push_str(line);
        buf.push('\n');
    }
}
