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
use crate::fmt::{
    core_bar, fmt_rate, fmt_size, fullwidth, fullwidth_display_width, make_bar, SIZE_PAIR_W, SIZE_W,
};
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub host: String,
    pub cpu_pct: f64,
    /// (core index, busy percent), ascending by index
    pub cores: Vec<(usize, f64)>,
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
fn shows_net(rx_rate: f64, tx_rate: f64) -> bool {
    rx_rate > 1024.0 || tx_rate > 1024.0
}

fn horizontal_rule(n: usize) -> String {
    "\u{2500}".repeat(n)
}

// ------------------------------------------------------------------ geometry

/// Geometry of the per-core CPU grid. One definition, used to draw it and to
/// count how tall it will be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreGrid {
    pub idx_w: usize,
    pub bar_len: usize,
    pub show_bars: bool,
    /// Visible width of one cell, including its trailing gutter.
    pub cell_w: usize,
    pub num_cols: usize,
    pub rows: usize,
}

impl CoreGrid {
    pub fn new(max_idx: usize, num_cores: usize, cols: usize, bar_len: usize) -> Self {
        let idx_w = max_idx.to_string().len();
        let per_core = (bar_len / num_cores.max(1)).min(12);
        let show_bars = per_core >= 5;
        // "<idx>:" + optional bar + "NNN%"
        let seg_w = idx_w + 1 + if show_bars { per_core } else { 0 } + 4;
        let cell_w = seg_w + 1;
        let num_cols = (cols.saturating_sub(6) / cell_w).max(1);
        CoreGrid {
            idx_w,
            bar_len: per_core,
            show_bars,
            cell_w,
            num_cols,
            rows: num_cores.div_ceil(num_cols.max(1)),
        }
    }
}

/// Rows a frame uses for everything except the process table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub num_cores: usize,
    pub cols: usize,
    pub has_mem: bool,
    pub has_disk: bool,
    pub has_net: bool,
}

impl Chrome {
    pub fn of(snap: &Snapshot, cols: usize) -> Self {
        Chrome {
            num_cores: snap.cores.len(),
            cols,
            has_mem: snap.mem.total > 0,
            has_disk: snap.disk.total > 0,
            has_net: shows_net(snap.rx_rate, snap.tx_rate),
        }
    }

    fn cpu_rows(&self) -> usize {
        if self.num_cores < 2 {
            1
        } else {
            CoreGrid::new(
                self.num_cores.saturating_sub(1),
                self.num_cores,
                self.cols,
                bar_len_for(self.cols),
            )
            .rows
        }
    }

    /// Host header + CPU block + the optional MEM/DSK/NET rows.
    pub fn height(&self) -> usize {
        1 + self.cpu_rows()
            + usize::from(self.has_mem)
            + usize::from(self.has_disk)
            + usize::from(self.has_net)
    }

    fn two_column(&self, num_procs: usize) -> bool {
        self.cols >= TWO_COLUMN_MIN_COLS && num_procs > 1
    }

    /// Rows a process table of `n` entries occupies, including its rule and
    /// header. An empty table draws nothing at all.
    pub fn table_height(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let body = if self.two_column(n) { n.div_ceil(2) } else { n };
        2 + body
    }

    /// How many processes to sample so the table fills `lines` exactly.
    ///
    /// Returns 0 when the panel has no room for a table — a clipped frame is
    /// worse than an honestly omitted one.
    pub fn proc_budget(&self, lines: usize) -> usize {
        let available = lines.saturating_sub(self.height());
        // Two rows go to the rule and header before any process fits.
        let Some(body) = available.checked_sub(2).filter(|n| *n > 0) else {
            return 0;
        };
        if self.cols >= TWO_COLUMN_MIN_COLS {
            body * 2
        } else {
            body
        }
    }
}

/// Rows the rendered frame will occupy.
pub fn frame_height(snap: &Snapshot, cols: usize) -> usize {
    let chrome = Chrome::of(snap, cols);
    chrome.height() + chrome.table_height(snap.procs.len())
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
fn truncate_name(name: &str, width: usize) -> String {
    if name.chars().count() < width {
        return name.to_string();
    }
    let mut s: String = name.chars().take(width.saturating_sub(3)).collect();
    s.push_str("...");
    s
}

fn proc_cell(p: &Proc, name_w: usize, pal: &Palette) -> String {
    let cpu_c = if p.cpu >= 10.0 { pal.yellow } else { pal.white };
    let mut s = String::with_capacity(96);
    let _ = write!(
        s,
        "{}{:>PID_W$}{}  {}{:<name_w$}{}  {}{:>CPU_W$.1}{}  {}{:>MEM_W$}{}",
        pal.gray,
        p.pid,
        pal.reset,
        pal.white,
        truncate_name(&p.name, name_w),
        pal.reset,
        cpu_c,
        p.cpu,
        pal.reset,
        pal.cyan,
        fmt_size(p.mem),
        pal.reset,
    );
    s
}

fn proc_header(name_w: usize, pal: &Palette) -> String {
    format!(
        "{}{:>PID_W$}  {:<name_w$}  {:>CPU_W$}  {:>MEM_W$}{}",
        pal.bold, "PID", "NAME", "CPU", "MEM", pal.reset,
    )
}

fn push_core_rows(
    out: &mut Vec<String>,
    cores: &[(usize, f64)],
    cols: usize,
    bar_len: usize,
    pal: &Palette,
) {
    let max_idx = cores.iter().map(|(i, _)| *i).max().unwrap_or(0);
    let grid = CoreGrid::new(max_idx, cores.len(), cols, bar_len);

    let segs: Vec<String> = cores
        .iter()
        .map(|&(idx, cp)| {
            if grid.show_bars {
                format!(
                    "{:>w$}:{}{:3.0}%",
                    idx,
                    core_bar(cp, grid.bar_len, pal),
                    cp,
                    w = grid.idx_w
                )
            } else {
                format!("{:>w$}:{:3.0}%", idx, cp, w = grid.idx_w)
            }
        })
        .collect();

    const LABEL_W: usize = " CPU ".len();
    let mut i = 0;
    while i < cores.len() {
        let mut row = if i == 0 {
            format!(" {}CPU{} ", pal.bold, pal.reset)
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
        pal.gray,
        horizontal_rule(cols.saturating_sub(2)),
        pal.reset
    ));

    let two_column = cols >= TWO_COLUMN_MIN_COLS && procs.len() > 1;
    let name_w = name_width(cols, two_column);
    let hdr = proc_header(name_w, pal);

    if two_column {
        let sep = format!(" {}\u{2502}{} ", pal.gray, pal.reset);
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
pub fn render(snap: &Snapshot, cols: usize, bar_len: usize, pal: &Palette) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(16);

    let disp_w = fullwidth_display_width(&snap.host);
    out.push(format!(
        "{}{}{}{}  {}{}{}",
        pal.cyan,
        pal.bold,
        fullwidth(&snap.host),
        pal.reset,
        pal.gray,
        horizontal_rule(cols.saturating_sub(disp_w).saturating_sub(6)),
        pal.reset,
    ));

    if snap.cores.len() >= 2 {
        push_core_rows(&mut out, &snap.cores, cols, bar_len, pal);
    } else {
        let bc = pal.cpu_bar(snap.cpu_pct);
        out.push(format!(
            " {}CPU{} {} {}{:.0}%{}",
            pal.bold,
            pal.reset,
            make_bar(snap.cpu_pct, bar_len, bc, pal.reset),
            bc,
            snap.cpu_pct,
            pal.reset,
        ));
    }

    for (label, usage, color) in [
        ("MEM", snap.mem, pal.mem_bar(snap.mem.pct)),
        ("DSK", snap.disk, pal.disk_bar(snap.disk.pct)),
    ] {
        if usage.total == 0 {
            continue;
        }
        let size = format!("{}/{}", fmt_size(usage.used), fmt_size(usage.total));
        out.push(format!(
            " {}{}{} {} {}{:3.0}%{} {}{:<w$}{}",
            pal.bold,
            label,
            pal.reset,
            make_bar(usage.pct, bar_len, color, pal.reset),
            color,
            usage.pct,
            pal.reset,
            pal.gray,
            size,
            pal.reset,
            w = SIZE_FIELD_W,
        ));
    }

    if shows_net(snap.rx_rate, snap.tx_rate) {
        out.push(format!(
            " {}NET{} {}\u{2191} {}{}  {}\u{2193} {}{}",
            pal.bold,
            pal.reset,
            pal.green,
            fmt_rate(snap.tx_rate),
            pal.reset,
            pal.cyan,
            fmt_rate(snap.rx_rate),
            pal.reset,
        ));
    }

    push_proc_table(&mut out, &snap.procs, cols, pal);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ANSI;

    fn usage(total: u64, used: u64, pct: f64) -> Usage {
        Usage { total, used, pct }
    }

    fn proc(pid: u32, name: &str, cpu: f64, mem: u64) -> Proc {
        Proc {
            pid,
            name: name.to_string(),
            cpu,
            mem,
        }
    }

    fn snap() -> Snapshot {
        Snapshot {
            host: "h".into(),
            ..Default::default()
        }
    }

    fn find(out: &[String], needle: &str) -> Vec<String> {
        out.iter().filter(|l| l.contains(needle)).cloned().collect()
    }

    fn labeled(out: &[String], label: &str) -> Vec<String> {
        let tag = format!("{}{}{}", ANSI.bold, label, ANSI.reset);
        out.iter().filter(|l| l.contains(&tag)).cloned().collect()
    }

    /// A snapshot with every optional row present, for layout tests.
    fn full(cores: usize, procs: usize, cols: usize) -> Snapshot {
        let _ = cols;
        Snapshot {
            host: "host (10.0.0.1)".into(),
            cores: (0..cores).map(|i| (i, 50.0)).collect(),
            mem: usage(1 << 31, 1 << 30, 50.0),
            disk: usage(1 << 40, 1 << 38, 80.0),
            rx_rate: 100_000.0,
            tx_rate: 100_000.0,
            procs: (0..procs as u32)
                .map(|i| proc(i, "proc", 1.0, 1000))
                .collect(),
            ..Default::default()
        }
    }

    // ------------------------------------------------------------- contents

    #[test]
    fn host_line_is_fullwidth() {
        assert!(render(&snap(), 80, 50, &ANSI)[0].contains('\u{ff48}'));
    }

    #[test]
    fn host_line_never_overflows_narrow_panel() {
        let s = Snapshot {
            host: "a-very-long-hostname (10.0.0.1)".into(),
            ..snap()
        };
        assert!(strip_ansi(&render(&s, 20, 4, &ANSI)[0]).contains('\u{ff41}'));
    }

    #[test]
    fn single_core_uses_aggregate_bar() {
        let s = Snapshot {
            cpu_pct: 42.0,
            ..snap()
        };
        let out = render(&s, 80, 50, &ANSI);
        assert!(out[1].contains("CPU") && out[1].contains("42%") && out[1].contains('['));
    }

    #[test]
    fn dual_core_shows_per_core_cells() {
        let s = Snapshot {
            cores: vec![(0, 75.0), (1, 25.0)],
            ..snap()
        };
        let out = render(&s, 80, 50, &ANSI);
        for want in ["CPU", "0:", "1:", "75%", "25%"] {
            assert!(out[1].contains(want), "missing {want} in {:?}", out[1]);
        }
    }

    #[test]
    fn many_cores_wrap_to_multiple_rows() {
        let s = Snapshot {
            cores: (0..8).map(|i| (i, i as f64 * 10.0)).collect(),
            ..snap()
        };
        let out = render(&s, 40, 20, &ANSI);
        assert!(
            out.iter()
                .filter(|l| l.contains(':') && l.contains('%'))
                .count()
                >= 2
        );
    }

    #[test]
    fn mem_shown_when_total_present() {
        let s = Snapshot {
            mem: usage(1 << 31, 1 << 30, 50.0),
            ..snap()
        };
        let rows = labeled(&render(&s, 80, 50, &ANSI), "MEM");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("50%") && rows[0].contains("GiB"));
    }

    #[test]
    fn mem_omitted_when_zero() {
        let s = Snapshot {
            disk: usage(1 << 40, 1 << 38, 80.0),
            ..snap()
        };
        assert!(labeled(&render(&s, 80, 50, &ANSI), "MEM").is_empty());
    }

    #[test]
    fn disk_shown_when_total_present() {
        let s = Snapshot {
            disk: usage(1 << 40, 1 << 38, 80.0),
            ..snap()
        };
        let rows = labeled(&render(&s, 80, 50, &ANSI), "DSK");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("80%") && rows[0].contains("TiB"));
    }

    #[test]
    fn disk_omitted_when_zero() {
        let s = Snapshot {
            mem: usage(1 << 31, 1 << 30, 50.0),
            ..snap()
        };
        assert!(labeled(&render(&s, 80, 50, &ANSI), "DSK").is_empty());
    }

    #[test]
    fn net_shown_when_traffic() {
        let s = Snapshot {
            rx_rate: 2e6,
            tx_rate: 3e6,
            ..snap()
        };
        let rows = find(&render(&s, 80, 50, &ANSI), "NET");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains('\u{2191}') && rows[0].contains('\u{2193}'));
    }

    #[test]
    fn net_omitted_when_idle() {
        let s = Snapshot {
            rx_rate: 500.0,
            tx_rate: 500.0,
            ..snap()
        };
        assert!(find(&render(&s, 80, 50, &ANSI), "NET").is_empty());
    }

    #[test]
    fn net_threshold_is_strictly_above_1k() {
        assert!(!shows_net(1024.0, 1024.0));
        assert!(shows_net(1025.0, 0.0));
        assert!(shows_net(0.0, 1025.0));
    }

    #[test]
    fn procs_are_listed_with_header() {
        let s = Snapshot {
            procs: vec![
                proc(100, "python3", 2.5, 45_000),
                proc(200, "bash", 0.5, 12_000),
            ],
            ..snap()
        };
        let all = render(&s, 80, 50, &ANSI).join("\n");
        assert!(all.contains("python3") && all.contains("bash") && all.contains("PID"));
    }

    /// An empty table is chrome with no content; drawing its rule and header
    /// would waste two rows the caller already budgeted away.
    #[test]
    fn empty_proc_list_draws_no_table() {
        let out = render(&snap(), 80, 50, &ANSI);
        assert!(!out.iter().any(|l| l.contains("PID")));
    }

    #[test]
    fn long_proc_name_is_truncated() {
        let s = Snapshot {
            procs: vec![proc(1, "verylongprocessnameishere", 1.0, 1000)],
            ..snap()
        };
        assert!(render(&s, 80, 50, &ANSI).join("\n").contains("..."));
    }

    #[test]
    fn truncated_name_fits_exactly() {
        assert_eq!(truncate_name("abcdefghij", 6), "abc...");
        assert_eq!(truncate_name("abc", 6), "abc");
        assert_eq!(truncate_name("abcdef", 6), "abc...");
        assert_eq!(truncate_name("abcde", 6), "abcde");
    }

    #[test]
    fn truncate_name_is_char_safe() {
        assert_eq!(
            truncate_name("\u{4f60}\u{597d}\u{4e16}\u{754c}\u{ff01}", 4),
            "\u{4f60}..."
        );
    }

    #[test]
    fn hot_proc_is_highlighted() {
        let s = Snapshot {
            procs: vec![proc(1, "hungry", 95.0, 1000)],
            ..snap()
        };
        let out = render(&s, 80, 50, &ANSI);
        assert!(out
            .iter()
            .find(|l| l.contains("hungry"))
            .unwrap()
            .contains(ANSI.yellow));
    }

    #[test]
    fn highlight_threshold_is_ten_percent() {
        for (cpu, hot) in [(9.9, false), (10.0, true)] {
            let s = Snapshot {
                procs: vec![proc(1, "solo", cpu, 0)],
                ..snap()
            };
            let out = render(&s, 80, 50, &ANSI);
            let line = out.iter().find(|l| l.contains("solo")).unwrap();
            assert_eq!(line.contains(ANSI.yellow), hot, "cpu={cpu}");
        }
    }

    // -------------------------------------------------------- fixed widths

    #[test]
    fn mem_and_dsk_rows_have_constant_width() {
        let a = Snapshot {
            mem: usage(1 << 31, 1 << 30, 50.0),
            ..snap()
        };
        let b = Snapshot {
            mem: usage(1 << 30, 1 << 20, 12.5),
            ..snap()
        };
        let wa = strip_ansi(&labeled(&render(&a, 80, 48, &ANSI), "MEM")[0])
            .chars()
            .count();
        let wb = strip_ansi(&labeled(&render(&b, 80, 48, &ANSI), "MEM")[0])
            .chars()
            .count();
        assert_eq!(wa, wb);
    }

    #[test]
    fn mem_and_dsk_rows_match_each_other() {
        let s = Snapshot {
            mem: usage(1 << 31, 1 << 30, 50.0),
            disk: usage(1 << 40, 1 << 38, 80.0),
            ..snap()
        };
        let out = render(&s, 80, 48, &ANSI);
        assert_eq!(
            strip_ansi(&labeled(&out, "MEM")[0]).chars().count(),
            strip_ansi(&labeled(&out, "DSK")[0]).chars().count()
        );
    }

    #[test]
    fn percentage_is_right_aligned() {
        let s = Snapshot {
            mem: usage(1 << 31, 1 << 30, 50.0),
            ..snap()
        };
        assert!(strip_ansi(&labeled(&render(&s, 80, 48, &ANSI), "MEM")[0]).contains("  50%"));
    }

    /// The regression the MEM column width exists for: `fmt_size` emits up to
    /// nine characters ("1023.9KiB"), and a narrower column would push every
    /// following row out of alignment for exactly those values.
    #[test]
    fn proc_rows_stay_aligned_across_all_size_magnitudes() {
        let sizes = [
            0u64,
            999,
            1024,
            1000 * 1024,
            1023 * 1024 + 1023,
            1024 * 1024,
            1005 * 1024 * 1024,
            1 << 30,
            1023 * (1 << 30),
            1 << 40,
        ];
        let procs: Vec<Proc> = sizes
            .iter()
            .enumerate()
            .map(|(i, &m)| proc(i as u32 + 1, "proc", i as f64, m))
            .collect();
        let s = Snapshot { procs, ..snap() };

        for cols in [60usize, 80, 120] {
            let out = render(&s, cols, bar_len_for(cols), &ANSI);
            let widths: Vec<usize> = out
                .iter()
                .skip_while(|l| !l.contains("PID"))
                .skip(1)
                .map(|l| strip_ansi(l).chars().count())
                .collect();
            assert!(widths.len() >= 5);
            assert!(
                widths.windows(2).all(|w| w[0] == w[1]),
                "cols={cols} widths={widths:?}"
            );
        }
    }

    #[test]
    fn proc_rows_stay_aligned_across_pid_and_cpu_ranges() {
        let procs: Vec<Proc> = [(1u32, 0.0), (99, 9.9), (99999, 100.0), (4194304, 7.5)]
            .iter()
            .map(|&(pid, cpu)| proc(pid, "name", cpu, 1 << 20))
            .collect();
        let s = Snapshot { procs, ..snap() };
        let out = render(&s, 60, bar_len_for(60), &ANSI);
        let widths: Vec<usize> = out
            .iter()
            .skip_while(|l| !l.contains("PID"))
            .map(|l| strip_ansi(l).chars().count())
            .collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
    }

    #[test]
    fn two_column_rows_align() {
        let procs: Vec<Proc> = (1..=6)
            .map(|i| {
                proc(
                    i * 1000,
                    &format!("name{i}"),
                    i as f64 * 3.0,
                    i as u64 * 1_000_000,
                )
            })
            .collect();
        let s = Snapshot { procs, ..snap() };
        let out = render(&s, 100, 68, &ANSI);
        let rows: Vec<usize> = out
            .iter()
            .filter(|l| l.contains('\u{2502}'))
            .map(|l| strip_ansi(l).chars().count())
            .collect();
        assert!(rows.len() >= 3);
        assert!(rows.windows(2).all(|w| w[0] == w[1]), "{rows:?}");
    }

    #[test]
    fn core_grid_cells_are_fixed_width() {
        let s = Snapshot {
            cores: (0..16).map(|i| (i, (i * 7 % 100) as f64)).collect(),
            ..snap()
        };
        let out = render(&s, 100, 48, &ANSI);
        let rows: Vec<String> = out
            .iter()
            .filter(|l| l.contains(':') && l.contains('%'))
            .map(|l| strip_ansi(l))
            .collect();
        assert!(rows.len() >= 2);
        let widths: Vec<usize> = rows[..rows.len() - 1]
            .iter()
            .map(|r| r.chars().count())
            .collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
    }

    #[test]
    fn core_grid_indices_are_right_aligned() {
        let s = Snapshot {
            cores: (0..12).map(|i| (i, 5.0)).collect(),
            ..snap()
        };
        let joined = strip_ansi(&render(&s, 100, 48, &ANSI).join("\n"));
        assert!(joined.contains(" 0:") && joined.contains("11:"), "{joined}");
    }

    // ------------------------------------------------------------- columns

    #[test]
    fn two_columns_when_wide() {
        let procs: Vec<Proc> = (1..=4).map(|i| proc(i, "p", i as f64, 1000)).collect();
        let s = Snapshot { procs, ..snap() };
        let out = render(&s, 80, 48, &ANSI);
        assert_eq!(
            out.iter().map(|l| l.matches("PID").count()).sum::<usize>(),
            2
        );
    }

    #[test]
    fn two_columns_pair_rows() {
        let s = Snapshot {
            procs: vec![proc(1, "left", 1.0, 1000), proc(2, "right", 2.0, 2000)],
            ..snap()
        };
        let out = render(&s, 80, 48, &ANSI);
        assert_eq!(
            out.iter()
                .filter(|l| l.contains("left") || l.contains("right"))
                .count(),
            1
        );
    }

    #[test]
    fn two_columns_odd_count_keeps_all() {
        let procs = vec![
            proc(1, "aa", 1.0, 1),
            proc(2, "bb", 2.0, 2),
            proc(3, "cc", 3.0, 3),
        ];
        let s = Snapshot { procs, ..snap() };
        let all = strip_ansi(&render(&s, 80, 48, &ANSI).join("\n"));
        for name in ["aa", "bb", "cc"] {
            assert!(all.contains(name), "missing {name}");
        }
    }

    #[test]
    fn single_column_when_narrow() {
        let s = Snapshot {
            procs: vec![proc(1, "a", 1.0, 1000), proc(2, "b", 2.0, 2000)],
            ..snap()
        };
        let out = render(&s, 60, 28, &ANSI);
        assert_eq!(
            out.iter().map(|l| l.matches("PID").count()).sum::<usize>(),
            1
        );
    }

    #[test]
    fn single_column_when_one_proc() {
        let s = Snapshot {
            procs: vec![proc(1, "solo", 1.0, 1000)],
            ..snap()
        };
        let out = render(&s, 80, 48, &ANSI);
        assert_eq!(
            out.iter().map(|l| l.matches("PID").count()).sum::<usize>(),
            1
        );
    }

    // ------------------------------------------------------------ geometry

    #[test]
    fn bar_len_has_a_floor() {
        assert_eq!(bar_len_for(0), 4);
        assert_eq!(bar_len_for(36), 4);
        assert_eq!(bar_len_for(80), 48);
    }

    #[test]
    fn core_grid_cell_includes_its_gutter() {
        let g = CoreGrid::new(3, 4, 80, 48);
        assert!(g.show_bars);
        assert_eq!(g.cell_w, g.idx_w + 1 + g.bar_len + 4 + 1);
    }

    #[test]
    fn core_grid_drops_bars_when_they_would_be_unreadable() {
        let g = CoreGrid::new(31, 32, 40, 8);
        assert!(!g.show_bars);
        assert_eq!(g.cell_w, g.idx_w + 1 + 4 + 1);
    }

    #[test]
    fn core_grid_always_has_at_least_one_column() {
        let g = CoreGrid::new(63, 64, 1, 1);
        assert_eq!(g.num_cols, 1);
        assert_eq!(g.rows, 64);
    }

    /// The bug this consolidation exists to prevent: the predicted row count
    /// must equal what the renderer actually draws, for every shape.
    #[test]
    fn predicted_core_rows_match_rendered_rows() {
        for cores in [2usize, 3, 4, 8, 16, 32, 64, 128] {
            for cols in [40usize, 60, 80, 120, 200] {
                let bar_len = bar_len_for(cols);
                let s = Snapshot {
                    cores: (0..cores).map(|i| (i, 10.0)).collect(),
                    ..snap()
                };
                let drawn = render(&s, cols, bar_len, &ANSI)
                    .iter()
                    .filter(|l| l.contains(':') && l.contains('%'))
                    .count();
                assert_eq!(
                    CoreGrid::new(cores - 1, cores, cols, bar_len).rows,
                    drawn,
                    "cores={cores} cols={cols}"
                );
            }
        }
    }

    /// Same invariant for the whole frame, not just the CPU block.
    #[test]
    fn predicted_frame_height_matches_render() {
        for cols in [40usize, 60, 72, 100, 200] {
            for cores in [1usize, 2, 8, 32] {
                for procs in [0usize, 1, 5, 12, 33] {
                    let s = full(cores, procs, cols);
                    assert_eq!(
                        frame_height(&s, cols),
                        render(&s, cols, bar_len_for(cols), &ANSI).len(),
                        "cols={cols} cores={cores} procs={procs}"
                    );
                }
            }
        }
    }

    /// The budget's whole purpose: a frame built from it fits the panel.
    ///
    /// The host header, CPU block and MEM/DSK/NET rows are irreducible. When
    /// a panel is shorter than those alone the budget cannot rescue it, but
    /// it must not make things worse by asking for processes there is no room
    /// to draw.
    #[test]
    fn budgeted_frame_fits_the_panel() {
        for cols in [40usize, 60, 72, 100, 200] {
            for lines in [4usize, 8, 12, 24, 50] {
                for cores in [1usize, 4, 16, 64] {
                    let chrome = Chrome::of(&full(cores, 0, cols), cols);
                    let budget = chrome.proc_budget(lines);
                    let height = frame_height(&full(cores, budget, cols), cols);
                    let ctx = format!("cols={cols} lines={lines} cores={cores}");
                    if lines >= chrome.height() {
                        assert!(height <= lines, "{ctx}: {height} > {lines}");
                    } else {
                        assert_eq!(budget, 0, "{ctx}: asked for processes with no room");
                        assert_eq!(height, chrome.height(), "{ctx}");
                    }
                }
            }
        }
    }

    /// ...and it should not leave usable rows unused.
    #[test]
    fn budget_fills_the_panel() {
        for cols in [60usize, 80, 120] {
            for lines in [12usize, 24, 40] {
                let chrome = Chrome::of(&full(4, 0, cols), cols);
                let budget = chrome.proc_budget(lines);
                let one_more = frame_height(&full(4, budget + 1, cols), cols);
                assert!(
                    one_more > lines,
                    "cols={cols} lines={lines}: room for {} more",
                    lines - one_more + 1
                );
            }
        }
    }

    #[test]
    fn budget_is_zero_when_there_is_no_room() {
        let chrome = Chrome::of(&full(64, 0, 40), 40);
        assert_eq!(chrome.proc_budget(4), 0);
        assert_eq!(chrome.table_height(0), 0);
    }

    /// A wide panel draws two processes per row, so it can carry twice as
    /// many. Compared at one core, where the CPU block is a single row at
    /// both widths and the rest of the chrome is identical.
    #[test]
    fn wide_panels_budget_two_per_row() {
        let wide_chrome = Chrome::of(&full(1, 0, 100), 100);
        let narrow_chrome = Chrome::of(&full(1, 0, 60), 60);
        assert_eq!(wide_chrome.height(), narrow_chrome.height());
        assert_eq!(
            wide_chrome.proc_budget(20),
            narrow_chrome.proc_budget(20) * 2
        );
    }

    #[test]
    fn empty_snapshot_renders_without_panic() {
        assert!(!render(&Snapshot::default(), 1, 1, &ANSI).is_empty());
    }
}
