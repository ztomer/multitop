//! What each panel remembers about the numbers it has already been shown.
//!
//! Every Monitor packet already carries CPU percent, memory usage and the two
//! network rates, and it arrives whatever view the panel is in. So the history
//! the graph view draws costs nothing on the wire and needs no change to the
//! agent -- which matters more than it sounds, because the agent is a binary
//! uploaded to every monitored host, and a protocol change means version skew
//! to handle on hosts nobody has re-uploaded to yet.
//!
//! The price is that a panel rebuilt a moment ago has no history yet. That is
//! honest and it heals itself: the graph says how many samples it has rather
//! than drawing a flat line that looks like an idle machine.

use std::collections::VecDeque;

/// How many samples a series keeps.
///
/// At the default refresh this is a few minutes of history, and it is wider
/// than any pane can draw -- a graph shows the newest `2 * width` of it. Kept
/// per panel, and one `f64` per sample per series, so the whole thing is a few
/// kilobytes for a screen full of hosts.
pub const SAMPLES: usize = 512;

/// One measured quantity over time, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    samples: VecDeque<f64>,
}

impl Series {
    pub fn push(&mut self, value: f64) {
        // A rate arithmetic produced from two counters can come out NaN or
        // negative across a counter reset. Neither is drawable, and letting
        // either into the series would poison the autoscale for every sample
        // after it, so it is clamped at the door rather than at each reader.
        let value = if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        };
        if self.samples.len() == SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The newest `n` samples, oldest first. Fewer than `n` if that is all
    /// there is -- the caller draws what exists rather than padding with
    /// zeroes, which would read as an idle machine.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<f64> {
        let skip = self.samples.len().saturating_sub(n);
        self.samples.iter().skip(skip).copied().collect()
    }

    #[must_use]
    pub fn latest(&self) -> Option<f64> {
        self.samples.back().copied()
    }
}

/// The three series a panel keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// Busy percent across all cores, 0..=100.
    pub cpu: Series,
    /// Memory in use as a percent of total, 0..=100.
    pub mem: Series,
    /// Received bytes per second.
    pub rx: Series,
    /// Transmitted bytes per second.
    pub tx: Series,
    /// Core clock in MHz. Empty on a machine that publishes no reading.
    pub mhz: Series,
}

impl History {
    /// Take one sample from a Monitor snapshot.
    ///
    /// Called wherever the packet lands, not where the graph is drawn: a view
    /// the user is not looking at still fills its history, so switching to the
    /// graphs does not start from nothing.
    pub fn record(&mut self, snap: &multitop_agent::render::Snapshot) {
        self.cpu.push(snap.cpu_pct);
        // `Usage` computes its own percent; recomputing it here would be a
        // second definition of "how full is memory" to keep in step by hand.
        self.mem.push(snap.mem.pct);
        self.rx.push(snap.rx_rate);
        self.tx.push(snap.tx_rate);
        // Only when there is one. Pushing a zero for "not measured" would put a
        // flat line on the graph that reads as a stalled CPU.
        if let Some(mhz) = snap.cpu_mhz {
            self.mhz.push(mhz);
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cpu.is_empty()
    }
}
