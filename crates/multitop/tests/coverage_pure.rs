//! Coverage tests for pure functions and state machine paths that the
//! integration tests don't reach. Each test exercises a specific uncovered
//! function or code path through the PUBLIC API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    assert!(multitop::layout::wrap_words("", 10).is_empty());
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
    assert!(kept.is_empty());
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

// ===========================================================================
// panel.rs — RingLines (public)
// ===========================================================================

#[test]
fn ring_lines_zero_capacity() {
    let mut ring = multitop::panel::RingLines::new(0);
    ring.push("won't be stored".into());
    assert_eq!(ring.len(), 0);
}

#[test]
fn ring_lines_one_capacity() {
    let mut ring = multitop::panel::RingLines::new(1);
    ring.push("first".into());
    ring.push("second".into());
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.last().map(String::as_str), Some("second"));
}

#[test]
fn ring_lines_clear_and_empty() {
    let mut ring = multitop::panel::RingLines::new(5);
    ring.push("a".into());
    ring.clear();
    assert_eq!(ring.len(), 0);
    assert!(ring.is_empty());
}

#[test]
fn ring_lines_get() {
    let mut ring = multitop::panel::RingLines::new(5);
    ring.push("a".into());
    ring.push("b".into());
    assert_eq!(ring.get(0).map(String::as_str), Some("a"));
    assert!(ring.get(9).is_none());
}

#[test]
fn ring_lines_set_cap_shrinks() {
    let mut ring = multitop::panel::RingLines::new(10);
    for i in 0..6 {
        ring.push(format!("line{i}"));
    }
    ring.set_cap(2);
    assert_eq!(ring.len(), 2);
    assert_eq!(ring.get(0).map(String::as_str), Some("line4"));
}

#[test]
fn ring_lines_slice() {
    let mut ring = multitop::panel::RingLines::new(5);
    for i in 0..5 {
        ring.push(format!("{i}"));
    }
    let slice: Vec<&str> = ring.slice(2, 2).map(String::as_str).collect();
    assert_eq!(slice, vec!["2", "3"]);
}

// ===========================================================================
// panel.rs — Panel (public)
// ===========================================================================

#[test]
fn panel_show_last_frame_with_cached_frame() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    p.show_frame(vec!["cached".into()]);
    p.show_last_frame();
    // show_last_frame overwrites view with last_frame (which was set by show_frame).
    assert!(p
        .view
        .iter()
        .any(|l| l.contains("cached") || l.contains("waiting")));
}

#[test]
fn panel_note_dedup() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    p.note("hello".into());
    p.note("hello".into()); // duplicate
    assert_eq!(p.notes.iter().filter(|n| *n == "hello").count(), 1);
}

#[test]
fn panel_note_bounded() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    for i in 0..10 {
        p.note(format!("note{i}"));
    }
    assert!(p.notes.len() <= 4); // MAX_NOTES = 4
}

#[test]
fn panel_show_body_reserves_row0() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    p.show_body(vec!["line1".into(), "line2".into()]);
    assert_eq!(p.view[0], "");
    assert_eq!(p.view[1], "line1");
}

// ===========================================================================
// passwords.rs — ServerDraft validation (public)
// ===========================================================================

#[test]
fn server_draft_valid() {
    use multitop::passwords::ServerDraft;
    let draft = ServerDraft {
        original: None,
        host: "valid-host".into(),
        user: "admin".into(),
        port: "22".into(),
        upgrade_cmd: "sudo apt update".into(),
        password: String::new(),
        field: 0,
    };
    let server = draft.into_server().expect("valid");
    assert_eq!(server.host, "valid-host");
    assert_eq!(server.port, 22);
}

#[test]
fn server_draft_invalid_port() {
    use multitop::passwords::ServerDraft;
    let draft = ServerDraft {
        original: None,
        host: "valid-host".into(),
        user: "admin".into(),
        port: "abc".into(),
        upgrade_cmd: "cmd".into(),
        password: String::new(),
        field: 0,
    };
    assert!(draft.into_server().is_err());
}

#[test]
fn server_draft_port_zero() {
    use multitop::passwords::ServerDraft;
    let draft = ServerDraft {
        original: None,
        host: "valid-host".into(),
        user: "admin".into(),
        port: "0".into(),
        upgrade_cmd: "cmd".into(),
        password: String::new(),
        field: 0,
    };
    assert!(draft.into_server().is_err());
}

// ===========================================================================
// config.rs — validation (public)
// ===========================================================================

#[test]
fn config_validate_host() {
    assert!(multitop::config::validate_host("has space").is_err());
    assert!(multitop::config::validate_host("valid-host").is_ok());
}

#[test]
fn config_validate_user() {
    assert!(multitop::config::validate_user("has space").is_err());
    assert!(multitop::config::validate_user("validuser").is_ok());
}

// ===========================================================================
// ui.rs — layout + windowing (public)
// ===========================================================================

#[test]
fn regions_layout() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let (panels, keybar) = multitop::ui::regions(area, 4);
    assert_eq!(panels.len(), 4);
    assert_eq!(keybar.height, 1);
}

#[test]
fn regions_single_panel() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let (panels, _) = multitop::ui::regions(area, 1);
    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].width, 80);
}

#[test]
fn regions_empty() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let (panels, _) = multitop::ui::regions(area, 0);
    assert!(panels.is_empty());
}

#[test]
fn visible_shows_all_when_fits() {
    let lines = vec!["a".into(), "b".into(), "c".into()];
    let (out, badge) = multitop::ui::visible(&lines, 10, 1, 80, 0);
    assert_eq!(out.len(), 3);
    assert_eq!(badge, 0);
}

#[test]
fn visible_clamps_height() {
    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, _) = multitop::ui::visible(&lines, 5, 1, 80, 0);
    assert!(out.len() <= 5);
}

#[test]
fn visible_scrolls_with_offset() {
    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, badge) = multitop::ui::visible(&lines, 10, 1, 80, 5);
    assert!(badge > 0);
    assert!(out.len() <= 10);
}

#[test]
fn visible_zero_height() {
    let (out, badge) = multitop::ui::visible(&["a".into()], 0, 1, 80, 0);
    assert!(out.is_empty());
    assert_eq!(badge, 0);
}

#[test]
fn visible_upgrade_composes() {
    let header = vec!["Status: ready".into()];
    let mut body = multitop::panel::RingLines::new(100);
    body.push("output".into());
    let tail = vec!["notice".into()];
    let (out, badge) = multitop::ui::visible_upgrade(&header, 1, &body, &tail, 10, 80, 0);
    assert!(!out.is_empty());
    assert_eq!(badge, 0);
}

#[test]
fn keybar_badges_shed() {
    use multitop_agent::SortBy;
    use ratatui::style::{Color, Style};
    let pal = &multitop_agent::color::THEMES[0];
    let label = Style::default().fg(Color::DarkGray);
    let key_hi = Style::default().fg(Color::White);
    let sort_label = Style::default().fg(Color::DarkGray);
    let badges =
        multitop::ui::keybar_badges(SortBy::Cpu, pal, label, key_hi, sort_label, Color::Yellow);
    assert_eq!(badges.len(), 3);
}

#[test]
fn mode_pair_highlights_active() {
    use multitop::app::Mode;
    use multitop::ui::mode_pair;
    use ratatui::style::{Color, Style};
    let active = Style::default().fg(Color::Black);
    let off = Style::default().fg(Color::White);
    let (k, l) = mode_pair(Mode::Docker, Mode::Docker, active, off, off);
    assert_eq!(k, active);
    assert_eq!(l, active);
}

// ===========================================================================
// agent_dims — public
// ===========================================================================

#[test]
fn agent_dims_minimum() {
    use ratatui::layout::Size;
    let dims = multitop::ui::agent_dims(
        Size {
            width: 10,
            height: 4,
        },
        0,
    );
    assert!(dims.0 >= multitop::ui::MIN_AGENT_COLS);
    assert!(dims.1 >= multitop::ui::MIN_AGENT_ROWS);
}
