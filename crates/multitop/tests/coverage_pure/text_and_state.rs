// ===========================================================================
// ansi.rs — pure functions (public)
// ===========================================================================

#[test]
fn ansi_strip_multiple_codes() {
    let input = "\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m";
    let plain = multitop_agent::color::strip_ansi(input);
    assert_eq!(plain, "red green");
}

#[test]
fn ansi_strip_no_codes() {
    assert_eq!(
        multitop_agent::color::strip_ansi("plain text"),
        "plain text"
    );
    assert_eq!(multitop_agent::color::strip_ansi(""), "");
}

// ===========================================================================
// fmt.rs — pure functions (public)
// ===========================================================================

#[test]
fn fmt_helpers_produce_output() {
    assert!(multitop::fmt::status_line("ready").contains("ready"));
    assert!(multitop::fmt::error_line(String::from("failed")).contains("failed"));
    assert!(multitop::fmt::header_line(String::from("Upgrade")).contains("Upgrade"));
}

// ===========================================================================
// refit.rs — pure functions (public)
// ===========================================================================

#[test]
fn refit_line_zero_width_returns_asis() {
    assert_eq!(multitop::ui::refit_line("hello", 0), "hello");
}

#[test]
fn refit_line_short_line_unchanged() {
    assert_eq!(multitop::ui::refit_line("hi", 10), "hi");
}

#[test]
fn refit_line_rule_expands() {
    // A line of box-drawing chars becomes a rule of the target width.
    let line = "\u{2500}\u{2500}\u{2500}";
    let fitted = multitop::ui::refit_line(line, 20);
    assert!(fitted.chars().count() > 3);
}

#[test]
fn refit_header_fits_width() {
    // refit_header requires a box-drawing char (\u{2500}) to proceed,
    // then formats the fullwidth Latin chars (0xFF01-0xFF5E) + spaces.
    let line = "\u{2500}\u{FF21}\u{FF22}\u{FF23}\u{2500}";
    let fitted = multitop::ui::refit_header(line, 30);
    let fitted = fitted.expect("has box + fullwidth chars");
    // Strip ANSI SGR before measuring visible width; the return value
    // includes colour codes (\x1b[90m, \x1b[36;1m, \x1b[0m).
    let visible = fitted
        .split('\x1b')
        .filter_map(|seg| seg.find('m').map(|i| &seg[i + 1..]))
        .collect::<String>();
    assert!(visible.chars().count() <= 30);
}

#[test]
fn refit_header_returns_none_without_box_chars() {
    assert!(multitop::ui::refit_header("Title", 20).is_none());
}

// ===========================================================================
// layout.rs — pure functions (public)
// ===========================================================================

#[test]
fn layout_wrap_words_respects_width() {
    let wrapped = multitop::layout::wrap_words("one two three four five", 10);
    for line in &wrapped {
        assert!(line.chars().count() <= 10);
    }
}

#[test]
fn layout_wrap_words_empty() {
    assert_eq!(
        multitop::layout::wrap_words("", 10),
        [] as [std::string::String; 0]
    );
}

#[test]
fn layout_fit_row_fits_within_budget() {
    let widths = vec![30, 30, 30];
    let kept = multitop::layout::fit_row(&widths, 2, 50, &[2, 1, 0]);
    let total: usize = kept.iter().map(|&i| widths[i]).sum();
    assert!(total <= 50 + 2 * kept.len().saturating_sub(1));
}

#[test]
fn layout_fit_row_empty_budget() {
    let kept = multitop::layout::fit_row(&[30, 30], 2, 0, &[1, 0]);
    assert_eq!(kept, [] as [usize; 0]);
}

// ===========================================================================
// state.rs — HostUpdate classification (public)
// ===========================================================================

#[test]
fn host_update_outcome_variants() {
    use multitop::state::{HostUpdate, Outcome};
    assert_eq!(HostUpdate::default().outcome(), Outcome::Never);
    assert_eq!(
        HostUpdate {
            started_at: Some(100),
            finished_at: None,
            success: false
        }
        .outcome(),
        Outcome::Interrupted
    );
    assert_eq!(
        HostUpdate {
            started_at: Some(100),
            finished_at: Some(172),
            success: true
        }
        .outcome(),
        Outcome::Ok
    );
    assert_eq!(
        HostUpdate {
            started_at: Some(100),
            finished_at: Some(150),
            success: false
        }
        .outcome(),
        Outcome::Failed
    );
}

#[test]
fn host_update_duration() {
    use multitop::state::HostUpdate;
    assert_eq!(
        HostUpdate {
            started_at: Some(100),
            finished_at: Some(172),
            success: true
        }
        .duration_secs(),
        Some(72)
    );
    assert_eq!(
        HostUpdate {
            started_at: Some(100),
            finished_at: None,
            success: false
        }
        .duration_secs(),
        None
    );
    // finished before start = None
    assert_eq!(
        HostUpdate {
            started_at: Some(200),
            finished_at: Some(100),
            success: true
        }
        .duration_secs(),
        None
    );
}

// ===========================================================================
// upgrade_view.rs — fmt helpers (public)
// ===========================================================================

#[test]
fn fmt_duration_variants() {
    assert_eq!(multitop::upgrade_view::fmt_duration(45), "45s");
    assert_eq!(multitop::upgrade_view::fmt_duration(72), "1m 12s");
    assert_eq!(multitop::upgrade_view::fmt_duration(7500), "2h 5m");
}

#[test]
fn fmt_ago_variants() {
    let now = 1_800_000_000;
    assert_eq!(multitop::upgrade_view::fmt_ago(now, now), "just now");
    assert_eq!(multitop::upgrade_view::fmt_ago(now - 120, now), "2 min ago");
    assert_eq!(
        multitop::upgrade_view::fmt_ago(now - 3600, now),
        "1 hour ago"
    );
    assert_eq!(
        multitop::upgrade_view::fmt_ago(now - 86400, now),
        "1 day ago"
    );
    assert_eq!(
        multitop::upgrade_view::fmt_ago(now + 100, now),
        "in the future"
    );
}
