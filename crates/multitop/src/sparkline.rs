#[derive(Debug, Clone)]
pub struct SparklineHistory {
    samples: Vec<f32>,
    capacity: usize,
}

impl SparklineHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, val: f32) {
        if self.samples.len() >= self.capacity {
            self.samples.remove(0);
        }
        self.samples.push(val.clamp(0.0, 100.0));
    }

    pub fn render_bar(&self) -> String {
        if self.samples.is_empty() {
            return String::new();
        }
        const BARS: &[char] = &[' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        self.samples
            .iter()
            .map(|&v| {
                let idx = ((v / 100.0) * 7.0).round() as usize;
                BARS[idx.min(7)]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_capacity_and_rendering() {
        let mut history = SparklineHistory::new(5);
        history.push(0.0);
        history.push(25.0);
        history.push(50.0);
        history.push(75.0);
        history.push(100.0);
        assert_eq!(history.render_bar(), " ▃▅▆█");

        history.push(10.0);
        assert_eq!(history.samples.len(), 5);
        assert_eq!(history.samples[0], 25.0);
    }
}
