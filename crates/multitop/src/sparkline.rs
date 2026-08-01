use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct SparklineHistory {
    samples: VecDeque<f32>,
    capacity: usize,
}

impl SparklineHistory {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, val: f32) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(val.clamp(0.0, 100.0));
    }

    #[must_use]
    pub fn render_bar(&self) -> String {
        self.render_bar_limited(self.capacity)
    }

    /// Render at most `max_chars` samples, keeping the MOST RECENT ones so a
    /// narrow header still shows the latest trend instead of dropping the whole
    /// bar. Zero-value samples render as the lowest visible block so idle bars
    /// are not invisible whitespace.
    #[must_use]
    pub fn render_bar_limited(&self, max_chars: usize) -> String {
        const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        if self.samples.is_empty() || max_chars == 0 {
            return String::new();
        }
        let start = self.samples.len().saturating_sub(max_chars);
        self.samples
            .iter()
            .skip(start)
            .map(|&v| {
                let clamped = v.clamp(0.0, 100.0);
                let scaled = (clamped / 100.0) * 7.0;
                let idx = scaled.round();
                let idx_usize = if idx <= 0.0 {
                    0
                } else if idx >= 7.0 {
                    7
                } else {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    { idx as usize }
                };
                BARS[idx_usize]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn sparkline_capacity_and_rendering() {
        let mut history = SparklineHistory::new(5);
        history.push(0.0);
        history.push(25.0);
        history.push(50.0);
        history.push(75.0);
        history.push(100.0);
        assert_eq!(history.render_bar(), "▁▃▅▆█");

        history.push(10.0);
        assert_eq!(history.samples.len(), 5);
        assert!((history.samples[0] - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_value_renders_visible_block() {
        let mut history = SparklineHistory::new(3);
        history.push(0.0);
        history.push(0.0);
        history.push(0.0);
        assert_eq!(history.render_bar(), "▁▁▁");
    }

    #[test]
    fn render_limited_keeps_most_recent_samples() {
        let mut history = SparklineHistory::new(6);
        for v in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0] {
            history.push(v);
        }
        let full = history.render_bar();
        assert_eq!(full.chars().count(), 6);
        let limited = history.render_bar_limited(3);
        assert_eq!(limited, "▄▅▅");
        assert_eq!(
            limited,
            full.chars().skip(3).collect::<String>(),
            "limited render must keep the most recent samples"
        );
        assert!(history.render_bar_limited(0).is_empty());
        assert!(SparklineHistory::new(5).render_bar_limited(5).is_empty());
    }
}
