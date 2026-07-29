//! Layout arithmetic and geometry calculations for panel rendering.

use crate::render::{bar_len_for, shows_net, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreGrid {
    pub idx_w: usize,
    pub bar_len: usize,
    pub show_bars: bool,
    pub cell_w: usize,
    pub num_cols: usize,
    pub rows: usize,
}

impl CoreGrid {
    pub fn new(
        max_idx: usize,
        num_cores: usize,
        cols: usize,
        bar_len: usize,
        has_temps: bool,
    ) -> Self {
        let idx_w = max_idx.to_string().len();
        let per_core = (bar_len / num_cores.max(1)).min(12);
        let show_bars = per_core >= 5;
        let temp_w = if has_temps { 5 } else { 0 };
        let seg_w = idx_w + 1 + if show_bars { per_core } else { 0 } + 4 + temp_w;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    TooSmall,
    Micro,
    Minimal,
    Compact,
    Full,
}

impl Tier {
    pub fn for_lines(lines: usize) -> Self {
        match lines {
            0 => Tier::Full,
            1..=2 => Tier::TooSmall,
            3..=4 => Tier::Micro,
            5..=7 => Tier::Minimal,
            8..=11 => Tier::Compact,
            _ => Tier::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub num_cores: usize,
    pub has_temps: bool,
    pub cols: usize,
    pub has_mem: bool,
    pub has_disk: bool,
    pub has_net: bool,
    pub tier: Tier,
}

impl Chrome {
    pub fn of(snap: &Snapshot, cols: usize, lines: usize) -> Self {
        let tier = Tier::for_lines(lines);
        let num_cores = if tier <= Tier::Compact {
            1
        } else {
            snap.cores.len()
        };
        let has_temps = snap.cores.iter().any(|(_, _, t)| t.is_some());
        let has_net = if tier < Tier::Full {
            false
        } else {
            shows_net(snap.rx_rate, snap.tx_rate)
        };
        let has_disk = if tier <= Tier::Micro {
            false
        } else {
            snap.disk.total > 0
        };
        let has_mem = if tier <= Tier::Micro {
            false
        } else {
            snap.mem.total > 0
        };
        Chrome {
            num_cores,
            has_temps,
            cols,
            has_mem,
            has_disk,
            has_net,
            tier,
        }
    }

    pub fn cpu_rows(&self) -> usize {
        if self.tier == Tier::Micro || self.tier == Tier::TooSmall {
            return 0;
        }
        if self.num_cores < 2 {
            1
        } else {
            CoreGrid::new(
                self.num_cores.saturating_sub(1),
                self.num_cores,
                self.cols,
                bar_len_for(self.cols),
                self.has_temps,
            )
            .rows
        }
    }

    pub fn height(&self) -> usize {
        match self.tier {
            Tier::TooSmall => 1,
            Tier::Micro => 2,
            Tier::Minimal | Tier::Compact | Tier::Full => {
                1 + self.cpu_rows()
                    + usize::from(self.has_mem)
                    + usize::from(self.has_disk)
                    + usize::from(self.has_net)
            }
        }
    }
}
