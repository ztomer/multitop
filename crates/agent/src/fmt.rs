//! Size / rate formatting and bar drawing. Output is byte-identical to the
//! Python original — these strings are load-bearing for panel column widths.

const KI: u64 = 1024;
const MI: u64 = KI * 1024;
const GI: u64 = MI * 1024;
const TI: u64 = GI * 1024;

/// Widest string [`fmt_size`] produces for any value below [`SIZE_MAX`],
/// e.g. `1023.9KiB`.
///
/// Any column displaying a size must be at least this wide, or rows holding
/// values in the 1000..1024 band of a unit will be one or two characters
/// longer than their neighbours and knock the whole table out of alignment.
/// The `size_fits_column` test pins this constant to what the function
/// actually emits, so the two cannot drift.
pub const SIZE_W: usize = 9;

/// Widest `used/total` pair.
pub const SIZE_PAIR_W: usize = SIZE_W * 2 + 1;

/// At and above this, `fmt_size` needs more than [`SIZE_W`] columns.
///
/// The TiB branch is the only one that can overflow, and it does so once the
/// integer part reaches five digits. No real machine has ten petabytes of RAM
/// or root filesystem, but Docker reports a near-`u64::MAX` sentinel as the
/// memory limit of an unconstrained container, so callers must screen for it
/// rather than assume their inputs are sane.
pub const SIZE_MAX: u64 = 9_999 * TI;

pub fn fmt_size(b: u64) -> String {
    if b >= TI {
        format!("{:.1}TiB", b as f64 / TI as f64)
    } else if b >= GI {
        format!("{:.1}GiB", b as f64 / GI as f64)
    } else if b >= MI {
        format!("{:.1}MiB", b as f64 / MI as f64)
    } else if b >= KI {
        format!("{:.1}KiB", b as f64 / KI as f64)
    } else {
        format!("{}B", b)
    }
}

pub fn fmt_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= (MI as f64) {
        format!("{:.1}M", bytes_per_sec / MI as f64)
    } else if bytes_per_sec >= (KI as f64) {
        format!("{:.1}K", bytes_per_sec / KI as f64)
    } else {
        // Python's int() truncates toward zero.
        format!("{}", bytes_per_sec as i64)
    }
}

/// Number of filled cells for a percentage, truncated and clamped.
///
/// The Python version did a bare `int(pct / 100 * length)`; a percentage
/// outside 0..=100 (which a bad /proc delta can produce) made it emit a bar
/// wider than its own column and skew the whole panel. Clamping keeps the
/// column fixed-width no matter what the kernel hands us.
fn filled_cells(pct: f64, length: usize) -> usize {
    if !pct.is_finite() || pct <= 0.0 {
        return 0;
    }
    let n = (pct / 100.0 * length as f64) as usize;
    n.min(length)
}

/// Bracketed bar: `[####....]`, used for the aggregate CPU/MEM/DSK rows.
pub fn make_bar(pct: f64, length: usize, color: &str, reset: &str) -> String {
    let filled = filled_cells(pct, length);
    let mut s = String::with_capacity(color.len() + length + reset.len() + 2);
    s.push_str(color);
    s.push('[');
    for _ in 0..filled {
        s.push('#');
    }
    for _ in 0..length - filled {
        s.push('.');
    }
    s.push(']');
    s.push_str(reset);
    s
}

/// Unbracketed bar used inside a per-core cell, colored by its own load.
pub fn core_bar(pct: f64, length: usize, p: &crate::color::Palette) -> String {
    let filled = filled_cells(pct, length);
    let color = p.cpu_bar(pct);
    let mut s = String::with_capacity(color.len() + length + p.reset.len());
    s.push_str(color);
    for _ in 0..filled {
        s.push('#');
    }
    for _ in 0..length - filled {
        s.push('.');
    }
    s.push_str(p.reset);
    s
}

/// Map printable ASCII into the fullwidth block so the host header reads as a
/// distinct, wider title. Space and non-ASCII pass through unchanged.
pub fn fullwidth(s: &str) -> String {
    s.chars()
        .map(|c| {
            let n = c as u32;
            if (0x21..=0x7E).contains(&n) {
                char::from_u32(n + 0xFEE0).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// Terminal cells occupied by `fullwidth(s)`: printable ASCII becomes a
/// double-width glyph, everything else stays single-width.
pub fn fullwidth_display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let n = c as u32;
            if (0x21..=0x7E).contains(&n) {
                2
            } else {
                1
            }
        })
        .sum()
}

use crate::color::Palette;

/// Center-aligned header line: `────── ｈｏｓｔｎａｍｅ ──────`
pub fn center_header(host: &str, cols: usize, pal: &Palette) -> String {
    let fw = fullwidth(host);
    let disp_w = fullwidth_display_width(host);
    if cols <= disp_w {
        return format!("{}{}{}{}", pal.primary(), pal.bold, fw, pal.reset);
    }
    let space_needed = disp_w + 2;
    if cols < space_needed {
        return format!("{}{}{}{}", pal.primary(), pal.bold, fw, pal.reset);
    }
    let rem = cols - space_needed;
    let left_len = rem / 2;
    let right_len = rem - left_len;
    format!(
        "{}{}{}{}{}{} {}{}{}{}",
        pal.secondary(),
        "\u{2500}".repeat(left_len),
        pal.reset,
        pal.primary(),
        pal.bold,
        fw,
        pal.reset,
        pal.secondary(),
        "\u{2500}".repeat(right_len),
        pal.reset
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{strip_ansi, ANSI};

    #[test]
    fn size_bytes() {
        assert_eq!(fmt_size(512), "512B");
        assert_eq!(fmt_size(0), "0B");
        assert_eq!(fmt_size(1023), "1023B");
    }

    #[test]
    fn size_kib() {
        assert_eq!(fmt_size(1024), "1.0KiB");
    }

    #[test]
    fn size_mib() {
        assert_eq!(fmt_size(2 * 1024 * 1024), "2.0MiB");
    }

    #[test]
    fn size_gib() {
        assert_eq!(fmt_size(1024u64.pow(3)), "1.0GiB");
        assert!(fmt_size(3 * 1024u64.pow(3)).contains("GiB"));
    }

    #[test]
    fn size_tib() {
        assert_eq!(fmt_size(1024u64.pow(4)), "1.0TiB");
    }

    /// Matches Python: 1048575/1024 rounds up to 1024.0 rather than rolling
    /// over to MiB, because the branch is chosen on the raw byte count.
    #[test]
    fn size_edge_below_mib() {
        assert_eq!(fmt_size(1024 * 1024 - 1), "1024.0KiB");
    }

    /// The constant every size column is sized from must actually bound the
    /// formatter. Sweeps the boundaries of each unit, where the widest
    /// strings ("1023.9KiB") occur.
    #[test]
    fn size_fits_column() {
        let mut widest = (0usize, String::new());
        let mut check = |b: u64| {
            let s = fmt_size(b);
            let w = s.chars().count();
            assert!(
                w <= SIZE_W,
                "fmt_size({b}) = {s:?} is {w} wide, over SIZE_W={SIZE_W}"
            );
            if w > widest.0 {
                widest = (w, s);
            }
        };
        for unit in [1u64, KI, MI, GI, TI] {
            for k in [0u64, 1, 2, 500, 999, 1000, 1022, 1023] {
                check(unit.saturating_mul(k));
                check(unit.saturating_mul(k).saturating_add(unit - 1));
            }
        }
        check(SIZE_MAX - 1);
        assert_eq!(
            widest.0, SIZE_W,
            "SIZE_W is loose; widest seen was {widest:?}"
        );
    }

    /// The bound is worth screening against, not decorative: the values it
    /// excludes really do overflow the column.
    #[test]
    fn size_max_excludes_values_that_overflow() {
        assert_eq!(fmt_size(SIZE_MAX - 1).chars().count(), SIZE_W);
        for over in [
            SIZE_MAX * 2,
            1 << 60,
            u64::MAX / 2,
            9_223_372_036_854_771_712,
        ] {
            assert!(
                fmt_size(over).chars().count() > SIZE_W,
                "fmt_size({over}) = {:?} unexpectedly fits",
                fmt_size(over)
            );
        }
    }

    #[test]
    fn size_pair_width_is_two_sizes_and_a_slash() {
        let pair = format!("{}/{}", fmt_size(1023 * KI), fmt_size(1023 * GI));
        assert_eq!(pair.chars().count(), SIZE_PAIR_W);
    }

    #[test]
    fn rate_bytes() {
        assert_eq!(fmt_rate(500.0), "500");
        assert_eq!(fmt_rate(1023.0), "1023");
        assert_eq!(fmt_rate(0.0), "0");
    }

    #[test]
    fn rate_truncates_toward_zero() {
        assert_eq!(fmt_rate(999.9), "999");
    }

    #[test]
    fn rate_kilo() {
        assert_eq!(fmt_rate(1500.0), "1.5K");
    }

    #[test]
    fn rate_mega() {
        assert_eq!(fmt_rate(2.0 * 1024.0 * 1024.0), "2.0M");
    }

    #[test]
    fn bar_basic() {
        let bar = make_bar(50.0, 10, ANSI.green, ANSI.reset);
        assert!(bar.starts_with(ANSI.green));
        assert!(bar.ends_with(ANSI.reset));
        assert!(bar.contains('#'));
        assert!(bar.contains('.'));
    }

    #[test]
    fn bar_zero_and_full() {
        assert_eq!(make_bar(0.0, 10, "", "").matches('.').count(), 10);
        assert_eq!(make_bar(0.0, 10, "", "").matches('#').count(), 0);
        assert_eq!(make_bar(100.0, 10, "", "").matches('#').count(), 10);
        assert_eq!(make_bar(100.0, 10, "", "").matches('.').count(), 0);
    }

    #[test]
    fn bar_length_is_fixed() {
        let bar = make_bar(50.0, 8, ANSI.green, ANSI.reset);
        assert_eq!(bar.len(), 8 + ANSI.green.len() + ANSI.reset.len() + 2);
    }

    #[test]
    fn bar_truncates_not_rounds() {
        assert_eq!(make_bar(55.0, 10, "", "").matches('#').count(), 5);
    }

    /// The class of bug the clamp exists for: an out-of-range percentage must
    /// never widen the column.
    #[test]
    fn bar_clamps_out_of_range() {
        assert_eq!(strip_ansi(&make_bar(140.0, 10, "", "")).len(), 12);
        assert_eq!(strip_ansi(&make_bar(-20.0, 10, "", "")).len(), 12);
        assert_eq!(strip_ansi(&make_bar(f64::NAN, 10, "", "")).len(), 12);
        assert_eq!(make_bar(-20.0, 10, "", "").matches('#').count(), 0);
        assert_eq!(make_bar(140.0, 10, "", "").matches('#').count(), 10);
    }

    #[test]
    fn core_bar_has_no_brackets() {
        let bar = core_bar(50.0, 10, &ANSI);
        assert!(!bar.contains('['.to_string().as_str()) || bar.contains("\x1b["));
        assert_eq!(strip_ansi(&bar).len(), 10);
        assert!(bar.contains('#'));
        assert!(bar.contains('.'));
    }

    #[test]
    fn core_bar_colors_by_own_load() {
        assert!(core_bar(10.0, 10, &ANSI).starts_with(ANSI.green));
        assert!(core_bar(60.0, 10, &ANSI).starts_with(ANSI.yellow));
        assert!(core_bar(90.0, 10, &ANSI).starts_with(ANSI.red));
    }

    #[test]
    fn core_bar_zero_and_full() {
        assert_eq!(core_bar(0.0, 10, &ANSI).matches('.').count(), 10);
        assert_eq!(core_bar(100.0, 10, &ANSI).matches('#').count(), 10);
    }

    #[test]
    fn core_bar_length() {
        let bar = core_bar(50.0, 8, &ANSI);
        assert_eq!(bar.len(), 8 + ANSI.reset.len() + ANSI.cpu_bar(50.0).len());
    }

    #[test]
    fn fullwidth_maps_ascii() {
        assert_eq!(fullwidth("h"), "\u{ff48}");
        assert_eq!(fullwidth("test"), "\u{ff54}\u{ff45}\u{ff53}\u{ff54}");
    }

    #[test]
    fn fullwidth_leaves_space_and_unicode() {
        assert_eq!(fullwidth(" "), " ");
        assert_eq!(fullwidth("\u{2500}"), "\u{2500}");
    }

    #[test]
    fn fullwidth_width_counts_cells() {
        assert_eq!(fullwidth_display_width("ab"), 4);
        assert_eq!(fullwidth_display_width("a b"), 5);
        assert_eq!(fullwidth_display_width(""), 0);
    }

    #[test]
    fn center_header_is_centered() {
        use crate::color::strip_ansi;
        let line = strip_ansi(&center_header("h", 40, &ANSI));
        assert!(line.contains("\u{ff48}"));
        let left_rules = line.chars().take_while(|c| *c == '\u{2500}').count();
        let right_rules = line.chars().rev().take_while(|c| *c == '\u{2500}').count();
        assert_eq!(left_rules, right_rules);
    }
}
