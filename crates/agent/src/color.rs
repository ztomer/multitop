//! ANSI palette.
//!
//! The Python original toggled module-level globals via `_set_ansi`. Here the
//! palette is a value threaded through the renderers, so colored and plain
//! output can be produced side by side (which is what the render tests want).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub name: &'static str,
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
    pub ratatui_keybar_bg: (u8, u8, u8),
    pub ratatui_border: (u8, u8, u8),
    pub ratatui_accent: (u8, u8, u8),
}

// 1. Kare (Default theme)
pub const KARE: Palette = Palette {
    name: "Kare",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;255;85;85m",
    green: "\x1b[38;2;80;250;123m",
    yellow: "\x1b[38;2;241;250;140m",
    blue: "\x1b[38;2;98;114;164m",
    cyan: "\x1b[38;2;139;233;253m",
    white: "\x1b[38;2;248;248;242m",
    gray: "\x1b[38;2;98;114;164m",
    purple: "\x1b[38;2;189;147;249m",
    ratatui_keybar_bg: (24, 26, 36),
    ratatui_border: (139, 233, 253),
    ratatui_accent: (80, 250, 123),
};

// 2. Dracula
pub const DRACULA: Palette = Palette {
    name: "Dracula",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;255;85;85m",
    green: "\x1b[38;2;80;250;123m",
    yellow: "\x1b[38;2;241;250;140m",
    blue: "\x1b[38;2;98;114;164m",
    cyan: "\x1b[38;2;139;233;253m",
    white: "\x1b[38;2;248;248;242m",
    gray: "\x1b[38;2;98;114;164m",
    purple: "\x1b[38;2;255;121;198m",
    ratatui_keybar_bg: (40, 42, 54),
    ratatui_border: (189, 147, 249),
    ratatui_accent: (255, 121, 198),
};

// 3. Nord
pub const NORD: Palette = Palette {
    name: "Nord",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;191;97;106m",
    green: "\x1b[38;2;163;190;140m",
    yellow: "\x1b[38;2;235;203;139m",
    blue: "\x1b[38;2;129;161;193m",
    cyan: "\x1b[38;2;136;192;208m",
    white: "\x1b[38;2;236;239;244m",
    gray: "\x1b[38;2;76;86;106m",
    purple: "\x1b[38;2;180;142;173m",
    ratatui_keybar_bg: (46, 52, 64),
    ratatui_border: (136, 192, 208),
    ratatui_accent: (163, 190, 140),
};

// 4. Gruvbox
pub const GRUVBOX: Palette = Palette {
    name: "Gruvbox",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;251;73;52m",
    green: "\x1b[38;2;184;187;38m",
    yellow: "\x1b[38;2;250;189;47m",
    blue: "\x1b[38;2;131;165;152m",
    cyan: "\x1b[38;2;142;192;124m",
    white: "\x1b[38;2;235;219;178m",
    gray: "\x1b[38;2;146;131;116m",
    purple: "\x1b[38;2;211;134;155m",
    ratatui_keybar_bg: (40, 40, 40),
    ratatui_border: (250, 189, 47),
    ratatui_accent: (254, 128, 25),
};

// 5. Catppuccin
pub const CATPPUCCIN: Palette = Palette {
    name: "Catppuccin",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;243;139;168m",
    green: "\x1b[38;2;166;227;161m",
    yellow: "\x1b[38;2;249;226;175m",
    blue: "\x1b[38;2;137;180;250m",
    cyan: "\x1b[38;2;148;226;213m",
    white: "\x1b[38;2;205;214;244m",
    gray: "\x1b[38;2;108;112;134m",
    purple: "\x1b[38;2;203;166;247m",
    ratatui_keybar_bg: (30, 30, 46),
    ratatui_border: (203, 166, 247),
    ratatui_accent: (148, 226, 213),
};

// 6. Tokyo Night
pub const TOKYO_NIGHT: Palette = Palette {
    name: "Tokyo Night",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;247;118;142m",
    green: "\x1b[38;2;158;206;106m",
    yellow: "\x1b[38;2;224;175;104m",
    blue: "\x1b[38;2;122;162;247m",
    cyan: "\x1b[38;2;125;207;255m",
    white: "\x1b[38;2;192;202;245m",
    gray: "\x1b[38;2;86;95;137m",
    purple: "\x1b[38;2;187;154;247m",
    ratatui_keybar_bg: (26, 27, 38),
    ratatui_border: (122, 162, 247),
    ratatui_accent: (125, 207, 255),
};

// 7. Monokai
pub const MONOKAI: Palette = Palette {
    name: "Monokai",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;249;38;114m",
    green: "\x1b[38;2;166;226;46m",
    yellow: "\x1b[38;2;230;219;116m",
    blue: "\x1b[38;2;102;217;239m",
    cyan: "\x1b[38;2;166;226;46m",
    white: "\x1b[38;2;248;248;242m",
    gray: "\x1b[38;2;117;113;94m",
    purple: "\x1b[38;2;174;129;255m",
    ratatui_keybar_bg: (39, 40, 34),
    ratatui_border: (249, 38, 114),
    ratatui_accent: (166, 226, 46),
};

// 8. Cyberpunk
pub const CYBERPUNK: Palette = Palette {
    name: "Cyberpunk",
    reset: "\x1b[0m",
    bold: "\x1b[1m",
    dim: "\x1b[2m",
    red: "\x1b[38;2;255;0;85m",
    green: "\x1b[38;2;0;255;102m",
    yellow: "\x1b[38;2;255;230;0m",
    blue: "\x1b[38;2;157;0;255m",
    cyan: "\x1b[38;2;0;240;255m",
    white: "\x1b[38;2;255;255;255m",
    gray: "\x1b[38;2;80;80;100m",
    purple: "\x1b[38;2;255;0;255m",
    ratatui_keybar_bg: (15, 15, 30),
    ratatui_border: (0, 240, 255),
    ratatui_accent: (255, 0, 85),
};

pub const THEMES: &[Palette] = &[
    KARE,
    DRACULA,
    NORD,
    GRUVBOX,
    CATPPUCCIN,
    TOKYO_NIGHT,
    MONOKAI,
    CYBERPUNK,
];

pub const ANSI: Palette = KARE;

pub const PLAIN: Palette = Palette {
    name: "Plain",
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
    ratatui_keybar_bg: (0, 0, 0),
    ratatui_border: (0, 0, 0),
    ratatui_accent: (0, 0, 0),
};

impl Palette {
    /// Primary accent color for headers, main metrics, and titles.
    pub fn primary(&self) -> &'static str {
        self.cyan
    }

    /// Secondary accent color for rule dividers, subtitles, and secondary metrics.
    pub fn secondary(&self) -> &'static str {
        self.purple
    }

    /// Muted color for PIDs, total limits, indents, and secondary text.
    pub fn muted(&self) -> &'static str {
        self.gray
    }

    /// Text color for process names, container names, and primary values.
    pub fn text(&self) -> &'static str {
        self.white
    }

    /// Meter color for low resource usage (0-50% CPU, normal temps).
    pub fn meter_low(&self) -> &'static str {
        self.green
    }

    /// Meter color for medium resource usage (50-80% CPU, warm temps).
    pub fn meter_mid(&self) -> &'static str {
        self.yellow
    }

    /// Meter color for high/critical resource usage (>80% CPU, hot temps).
    pub fn meter_high(&self) -> &'static str {
        self.red
    }

    pub fn cpu_bar(&self, pct: f64) -> &'static str {
        if pct >= 80.0 {
            self.meter_high()
        } else if pct >= 50.0 {
            self.meter_mid()
        } else {
            self.meter_low()
        }
    }

    pub fn mem_bar(&self, pct: f64) -> &'static str {
        if pct >= 85.0 {
            self.meter_high()
        } else if pct >= 50.0 {
            self.meter_mid()
        } else {
            self.primary()
        }
    }

    pub fn disk_bar(&self, pct: f64) -> &'static str {
        if pct >= 90.0 {
            self.meter_high()
        } else if pct >= 70.0 {
            self.meter_mid()
        } else {
            self.meter_low()
        }
    }

    /// Green while running, yellow once exited, red for anything else.
    pub fn status_color(&self, status: &str) -> &'static str {
        if status.starts_with("Up") || status == "running" {
            self.meter_low()
        } else if status.contains("Exited (0)") || status.contains("paused") {
            self.meter_mid()
        } else {
            self.meter_high()
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
