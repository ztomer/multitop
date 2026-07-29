//! ANSI palette.
//!
//! The Python original toggled module-level globals via `_set_ansi`. Here the
//! palette is a value threaded through the renderers, so colored and plain
//! output can be produced side by side (which is what the render tests want).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub reset: &'static str,
    pub bold: &'static str,
    pub dim: &'static str,
    pub red: &'static str,
    pub green: &'static str,
    pub yellow: &'static str,
    pub blue: &'static str,
    pub cyan: &'static str,
    pub white: &'static str,
    pub gray: &'static str,
    pub purple: &'static str,
}

pub const ANSI: Palette = Palette {
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[0;31m",
    green: "\x1b[0;32m",
    yellow: "\x1b[0;33m",
    blue: "\x1b[0;34m",
    cyan: "\x1b[0;36m",
    white: "\x1b[0;37m",
    gray: "\x1b[0;90m",
    purple: "\x1b[0;35m",
};

pub const PLAIN: Palette = Palette {
    reset: "",
    bold: "",
    dim: "",
    red: "",
    green: "",
    yellow: "",
    blue: "",
    cyan: "",
    white: "",
    gray: "",
    purple: "",
};

impl Palette {
    pub fn cpu_bar(&self, pct: f64) -> &'static str {
        if pct >= 80.0 {
            self.red
        } else if pct >= 50.0 {
            self.yellow
        } else {
            self.green
        }
    }

    pub fn mem_bar(&self, pct: f64) -> &'static str {
        if pct >= 80.0 {
            self.red
        } else if pct >= 50.0 {
            self.yellow
        } else {
            self.cyan
        }
    }

    pub fn disk_bar(&self, pct: f64) -> &'static str {
        if pct >= 90.0 {
            self.red
        } else if pct >= 70.0 {
            self.yellow
        } else {
            self.green
        }
    }

    /// Green while running, yellow once exited, red for anything else.
    pub fn status_color(&self, status: &str) -> &'static str {
        if status.contains("Up") {
            self.green
        } else if status.contains("Exit") {
            self.yellow
        } else {
            self.red
        }
    }
}

/// Strip SGR sequences, for width math and for tests that assert on layout.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // We only ever emit CSI ... m, so consume through the final byte.
        if chars.next() != Some('[') {
            continue;
        }
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_bar_thresholds() {
        assert_eq!(ANSI.cpu_bar(10.0), ANSI.green);
        assert_eq!(ANSI.cpu_bar(49.9), ANSI.green);
        assert_eq!(ANSI.cpu_bar(50.0), ANSI.yellow);
        assert_eq!(ANSI.cpu_bar(60.0), ANSI.yellow);
        assert_eq!(ANSI.cpu_bar(79.9), ANSI.yellow);
        assert_eq!(ANSI.cpu_bar(80.0), ANSI.red);
        assert_eq!(ANSI.cpu_bar(85.0), ANSI.red);
    }

    #[test]
    fn mem_bar_thresholds() {
        assert_eq!(ANSI.mem_bar(10.0), ANSI.cyan);
        assert_eq!(ANSI.mem_bar(49.0), ANSI.cyan);
        assert_eq!(ANSI.mem_bar(50.0), ANSI.yellow);
        assert_eq!(ANSI.mem_bar(60.0), ANSI.yellow);
        assert_eq!(ANSI.mem_bar(85.0), ANSI.red);
    }

    #[test]
    fn disk_bar_thresholds() {
        assert_eq!(ANSI.disk_bar(10.0), ANSI.green);
        assert_eq!(ANSI.disk_bar(69.0), ANSI.green);
        assert_eq!(ANSI.disk_bar(70.0), ANSI.yellow);
        assert_eq!(ANSI.disk_bar(75.0), ANSI.yellow);
        assert_eq!(ANSI.disk_bar(90.0), ANSI.red);
        assert_eq!(ANSI.disk_bar(95.0), ANSI.red);
    }

    #[test]
    fn status_colors() {
        assert_eq!(ANSI.status_color("Up 3 hours"), ANSI.green);
        assert_eq!(ANSI.status_color("Exited (0) 2 min ago"), ANSI.yellow);
        assert_eq!(ANSI.status_color("Created"), ANSI.red);
    }

    #[test]
    fn plain_palette_is_empty() {
        assert_eq!(PLAIN.reset, "");
        assert_eq!(PLAIN.cpu_bar(99.0), "");
    }

    #[test]
    fn strip_ansi_removes_sgr() {
        assert_eq!(strip_ansi("\x1b[0;31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\x1b[1ma\x1b[0mb\x1b[0;90mc\x1b[0m"), "abc");
    }

    #[test]
    fn strip_ansi_preserves_unicode() {
        assert_eq!(
            strip_ansi("\x1b[0;36m\u{ff48}\u{2500}\x1b[0m"),
            "\u{ff48}\u{2500}"
        );
    }
}
