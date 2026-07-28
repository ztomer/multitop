//! The sampling loop's state: what one tick needs to remember from the last.

use crate::proc::{self, CpuStat, NetTotals, ProcSampler};
use crate::render::{Chrome, Snapshot};

pub struct Monitor {
    host: String,
    prev_cpu: CpuStat,
    prev_net: NetTotals,
    sampler: ProcSampler,
}

impl Monitor {
    /// Read the baselines every rate calculation is a delta against, so the
    /// first `tick` produces real numbers rather than a frame of zeroes.
    pub fn new(host: String) -> Self {
        let mut sampler = ProcSampler::new();
        sampler.prime();
        Monitor {
            host,
            prev_cpu: proc::get_cpu_stat(),
            prev_net: proc::get_net(),
            sampler,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Sample everything once and return the frame to draw.
    pub fn tick(&mut self, interval: f64, cols: usize, lines: usize, sort_by: crate::SortBy) -> Snapshot {
        let cpu = proc::get_cpu_stat();
        let net = proc::get_net();
        let mem = proc::get_memory();
        let disk = proc::get_disk();

        let cpu_pct = cpu.aggregate.pct_since(&self.prev_cpu.aggregate);

        let temps = proc::get_core_temps();
        let cores: Vec<(usize, f64, Option<f64>)> = cpu
            .cores
            .iter()
            .filter_map(|(idx, curr)| {
                let prev = self.prev_cpu.core(*idx)?;
                let cp = curr.pct_since(&prev);
                let t = temps.get(idx).copied().or_else(|| temps.get(&0).copied());
                Some((*idx, cp, t))
            })
            .collect();

        let rate = |curr: u64, prev: u64| {
            if interval > 0.0 {
                curr.saturating_sub(prev) as f64 / interval
            } else {
                0.0
            }
        };
        let rx_rate = rate(net.rx, self.prev_net.rx);
        let tx_rate = rate(net.tx, self.prev_net.tx);

        // Everything except the process list is known now, so the panel can
        // ask for exactly as many processes as it has room to draw.
        let mut snap = Snapshot {
            host: self.host.clone(),
            cpu_pct,
            cores,
            temp_unit: Default::default(),
            mem,
            disk,
            rx_rate,
            tx_rate,
            procs: Vec::new(),
        };
        let budget = Chrome::of(&snap, cols, lines).proc_budget(lines);
        snap.procs = self.sampler.top(interval, budget, sort_by);

        self.prev_cpu = cpu;
        self.prev_net = net;
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ANSI;
    use crate::proc::{Proc, Usage};
    use crate::render::{bar_len_for, frame_height, render};

    fn snapshot(cores: usize, procs: usize) -> Snapshot {
        Snapshot {
            host: "h".into(),
            cores: (0..cores).map(|i| (i, 10.0, None)).collect(),
            mem: Usage {
                total: 1 << 31,
                used: 1 << 30,
                pct: 50.0,
            },
            disk: Usage {
                total: 1 << 40,
                used: 1 << 38,
                pct: 80.0,
            },
            rx_rate: 100_000.0,
            tx_rate: 100_000.0,
            procs: (0..procs as u32)
                .map(|i| Proc {
                    pid: i,
                    name: "p".into(),
                    cpu: 1.0,
                    mem: 1000,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The budget a tick uses must produce a frame the panel can hold.
    #[test]
    fn tick_budget_produces_a_frame_that_fits() {
        for cols in [40usize, 72, 100, 200] {
            for lines in [4usize, 8, 12, 24, 50] {
                for cores in [1usize, 4, 16] {
                    let chrome = Chrome::of(&snapshot(cores, 0), cols, lines);
                    let budget = chrome.proc_budget(lines);
                    let s = snapshot(cores, budget);
                    let height = frame_height(&s, cols, lines);
                    // Below the irreducible chrome height nothing fits; the
                    // budget must at least not add to the overflow.
                    let limit = lines.max(chrome.height());
                    assert!(
                        height <= limit,
                        "cols={cols} lines={lines} cores={cores}: {height} > {limit}"
                    );
                    assert_eq!(height, render(&s, cols, lines, bar_len_for(cols), &ANSI).len());
                }
            }
        }
    }
}
