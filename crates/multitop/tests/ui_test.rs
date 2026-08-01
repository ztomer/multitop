use multitop::ui::{
    agent_dims, keybar_line, regions, visible, KEYBAR_H, MIN_AGENT_COLS, MIN_AGENT_ROWS,
};
use ratatui::layout::{Rect, Size};

const fn size(w: u16, h: u16) -> Size {
    Size {
        width: w,
        height: h,
    }
}

#[test]
fn agent_dims_leave_room_for_margins_and_keybar() {
    let (cols, rows) = agent_dims(size(100, 31), 3);
    assert_eq!(cols, 48, "half width minus margins for 2-column grid");
    assert_eq!(rows, 15, "30 body rows over 2 grid rows");
}

#[test]
fn agent_dims_have_floors() {
    let (cols, rows) = agent_dims(size(10, 3), 4);
    assert_eq!(cols, MIN_AGENT_COLS);
    assert_eq!(rows, MIN_AGENT_ROWS);
}

#[test]
fn agent_dims_handle_no_panels() {
    let (cols, rows) = agent_dims(size(100, 30), 0);
    assert_eq!((cols, rows), (MIN_AGENT_COLS, MIN_AGENT_ROWS));
}

#[test]
fn agent_dims_single_panel_gets_the_body() {
    let (_, rows) = agent_dims(size(80, 25), 1);
    assert_eq!(rows, 24);
}

#[test]
fn regions_reserve_one_row_for_the_keybar() {
    let (panels, keybar) = regions(Rect::new(0, 0, 80, 24), 2);
    assert_eq!(keybar.height, KEYBAR_H);
    assert_eq!(keybar.y, 23);
    assert_eq!(panels.len(), 2);
    assert_eq!(panels.iter().map(|r| r.height).sum::<u16>(), 23);
}

#[test]
fn regions_grid_layout_for_three_panels() {
    let (panels, keybar) = regions(Rect::new(0, 0, 80, 30), 3);
    assert_eq!(panels.len(), 3);
    assert_eq!(keybar.y, 29);
    assert_eq!(panels[0].y, 0);
    assert_eq!(panels[1].y, 0);
    assert_eq!(panels[2].y, 15);
    assert_eq!(panels[0].width, 40);
    assert_eq!(panels[1].width, 40);
}

#[test]
fn regions_with_no_panels_still_yield_a_keybar() {
    let (panels, keybar) = regions(Rect::new(0, 0, 80, 24), 0);
    assert!(panels.is_empty());
    assert_eq!(keybar.height, KEYBAR_H);
}

#[test]
fn visible_shows_everything_when_it_fits() {
    let lines: Vec<String> = (0..3).map(|i| i.to_string()).collect();
    assert_eq!(visible(&lines, 10, 0, 0, 0).len(), 3);
    assert_eq!(visible(&lines, 3, 0, 0, 0).len(), 3);
}

#[test]
fn visible_preserves_header_and_tail_logs() {
    let lines: Vec<String> = vec![
        "HEADER".into(),
        "CPU 50%".into(),
        "MEM 40%".into(),
        "DSK 30%".into(),
        "RULE".into(),
        "PROC HDR".into(),
        "p1".into(),
        "p2".into(),
        "p3".into(),
        "p4".into(),
        "p5".into(),
    ];
    let shown = visible(&lines, 6, 1, 0, 0);
    assert_eq!(shown.len(), 6);
    assert_eq!(shown[0], "HEADER");
    assert_eq!(shown[1], "p1");
    assert_eq!(shown[5], "p5");
}

#[test]
fn visible_handles_zero_height() {
    let lines: Vec<String> = (0..3).map(|i| i.to_string()).collect();
    assert!(visible(&lines, 0, 0, 0, 0).is_empty());
}

#[test]
fn visible_scrolls_backwards_into_history() {
    let lines: Vec<String> = vec![
        "HEADER".into(),
        "line 1".into(),
        "line 2".into(),
        "line 3".into(),
        "line 4".into(),
        "line 5".into(),
        "line 6".into(),
        "line 7".into(),
        "line 8".into(),
        "line 9".into(),
        "line 10".into(),
    ];
    let tail = visible(&lines, 4, 1, 0, 0);
    assert_eq!(tail[0], "HEADER");
    assert_eq!(tail[1], "line 8");
    assert_eq!(tail[3], "line 10");

    let scrolled = visible(&lines, 4, 1, 0, 3);
    assert_eq!(scrolled[0], "HEADER");
    assert_eq!(scrolled[1], "line 5");
    assert_eq!(scrolled[3], "line 7");
}

#[test]
fn keybar_lists_every_binding() {
    let theme = &multitop_agent::color::KARE;
    let text: String = keybar_line(
        multitop_agent::SortBy::Cpu,
        theme,
        120,
        multitop::app::Mode::Monitor,
    )
    .spans
    .iter()
    .map(|s| s.content.as_ref())
    .collect();
    for hint in [
        "ESC", "Quit", "F", "Fetch", "D", "Docker", "S", "Stats", "U", "Upgrade", "T", "Theme",
    ] {
        assert!(text.contains(hint), "missing {hint} in {text:?}");
    }
}

#[test]
fn keybar_shows_sort_by_cpu_and_theme() {
    let theme = &multitop_agent::color::KARE;
    let text: String = keybar_line(
        multitop_agent::SortBy::Cpu,
        theme,
        120,
        multitop::app::Mode::Monitor,
    )
    .spans
    .iter()
    .map(|s| s.content.as_ref())
    .collect();
    assert!(text.contains("heme: Kare"), "theme indicator missing");
    assert!(text.contains("[Sort:"), "sort indicator missing");
    assert!(text.contains("Cpu"), "CPU sort key missing from keybar");
}

#[test]
fn keybar_shows_sort_by_mem() {
    let theme = &multitop_agent::color::KARE;
    let text: String = keybar_line(
        multitop_agent::SortBy::Mem,
        theme,
        120,
        multitop::app::Mode::Monitor,
    )
    .spans
    .iter()
    .map(|s| s.content.as_ref())
    .collect();
    assert!(text.contains("[Sort:"), "sort indicator missing");
    assert!(text.contains("Mem"), "Memory sort key missing from keybar");
}
