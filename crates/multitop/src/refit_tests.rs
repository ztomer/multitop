//! Refitting, and the ink it is drawn in.
//!
//! The palette is the process's published one, which a concurrent test may
//! change at any moment, so these assert the KIND of escape (24-bit, from a
//! palette) rather than its value. The defect they pin: the refit header and
//! rule were `ESC[36;1m` and `ESC[90m`, colours the terminal chooses and no
//! adaptation sees.

use super::*;
use multitop_agent::color::strip_ansi;

const HEADER: &str = "\u{2500}\u{2500}\u{2500} \u{ff23}\u{ff30}\u{ff30} \u{2500}\u{2500}\u{2500}";

/// Every colour in `line` is a 24-bit one: a palette's, not the terminal's.
fn palette_coloured(line: &str) -> bool {
    line.contains("\x1b[38;2;") && !line.contains("\x1b[36") && !line.contains("\x1b[90m")
}

#[test]
fn refit_header_wide_terminal() {
    let result = refit_header(HEADER, 40).unwrap();
    assert!(result.contains("\u{ff23}\u{ff30}\u{ff30}"));
    assert!(result.contains('\u{2500}'));
    assert_eq!(strip_ansi(&result).chars().count(), 40 - 3);
    assert!(palette_coloured(&result), "{result:?}");
}

#[test]
fn refit_header_narrow_terminal() {
    let result = refit_header(HEADER, 5).unwrap();
    assert_eq!(strip_ansi(&result), "\u{ff23}\u{ff30}\u{ff30}");
    assert!(palette_coloured(&result), "{result:?}");
}

#[test]
fn refit_header_fullwidth_chars() {
    let result = refit_header(HEADER, 20).unwrap();
    assert!(result.contains("\u{ff23}\u{ff30}\u{ff30}"));
}

#[test]
fn refit_line_horizontal_rule() {
    let line = "\u{2500}".repeat(8);
    let result = refit_line(&line, 20);
    assert_eq!(strip_ansi(&result), format!(" {}", "\u{2500}".repeat(18)));
    assert!(palette_coloured(&result), "{result:?}");
}

#[test]
fn refit_line_plain_text() {
    assert_eq!(refit_line("hello world", 20), "hello world");
}

#[test]
fn refit_line_empty() {
    assert_eq!(refit_line("", 20), "");
}
