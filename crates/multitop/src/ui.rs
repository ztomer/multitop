//! Layout and drawing.

use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ansi;
use crate::app::App;

/// Rows reserved at the bottom for the key hints.
const KEYBAR_H: u16 = 1;
/// One blank column either side of a panel's contents.
const SIDE_MARGIN: u16 = 1;

/// Minimum panel width the agent is asked to render into. Below this the
/// layout stops being readable anyway, and a too-small width makes the
/// agent's own column arithmetic degenerate.
const MIN_AGENT_COLS: u16 = 40;
const MIN_AGENT_ROWS: u16 = 4;

/// Split the screen into one region per panel plus the key bar.
fn regions(area: Rect, panels: usize) -> (Vec<Rect>, Rect) {
    let [body, keybar] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(KEYBAR_H)]).areas(area);
    if panels == 0 {
        return (Vec::new(), keybar);
    }
    if panels == 1 {
        return (vec![body], keybar);
    }
    if panels == 2 {
        let rows = Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(body);
        return (rows.to_vec(), keybar);
    }

    // For panels >= 3, use a 2-column grid layout
    let grid_cols: u32 = 2;
    let grid_rows: u32 = (panels as u32).div_ceil(2);
    let v_chunks = Layout::vertical(vec![Constraint::Ratio(1, grid_rows); grid_rows as usize]).split(body);
    let mut rects = Vec::with_capacity(panels);
    for (r_idx, row_rect) in v_chunks.iter().enumerate() {
        let h_chunks = Layout::horizontal([Constraint::Ratio(1, grid_cols), Constraint::Ratio(1, grid_cols)]).split(*row_rect);
        for (c_idx, col_rect) in h_chunks.iter().enumerate() {
            if r_idx * 2 + c_idx < panels {
                rects.push(*col_rect);
            }
        }
    }
    (rects, keybar)
}

/// The panel size to tell the agent about, so its frames arrive pre-fitted.
pub fn agent_dims(size: Size, panels: usize) -> (u16, u16) {
    if panels == 0 {
        return (MIN_AGENT_COLS, MIN_AGENT_ROWS);
    }
    let body_h = size.height.saturating_sub(KEYBAR_H);
    let (grid_cols, grid_rows) = match panels {
        1 => (1u16, 1u16),
        2 => (1u16, 2u16),
        n => (2u16, (n as u16).div_ceil(2)),
    };
    let cols = (size.width / grid_cols)
        .saturating_sub(SIDE_MARGIN * 2)
        .max(MIN_AGENT_COLS);
    let rows = (body_h / grid_rows).max(MIN_AGENT_ROWS);
    (cols, rows)
}

/// Reflow line 0 header if target_cols differs from the line's pre-rendered width.
pub fn refit_header(line: &str, target_cols: usize) -> Option<String> {
    if !line.contains('\u{2500}') {
        return None;
    }
    let fw: String = line
        .chars()
        .filter(|c| (0xFF01..=0xFF5E).contains(&(*c as u32)) || *c == ' ')
        .collect();
    let fw_trimmed = fw.trim();
    if fw_trimmed.is_empty() {
        return None;
    }
    let disp_w: usize = fw_trimmed
        .chars()
        .map(|c| if (0xFF01..=0xFF5E).contains(&(c as u32)) { 2 } else { 1 })
        .sum();

    if target_cols <= disp_w {
        return Some(format!("\x1b[36;1m{fw_trimmed}\x1b[0m"));
    }
    let space_needed = disp_w + 2;
    if target_cols < space_needed {
        return Some(format!("\x1b[36;1m{fw_trimmed}\x1b[0m"));
    }
    let rem = target_cols - space_needed;
    let left_len = rem / 2;
    let right_len = rem - left_len;

    Some(format!(
        "\x1b[90m{}\x1b[0m\x1b[36;1m {} \x1b[0m\x1b[90m{}\x1b[0m",
        "\u{2500}".repeat(left_len),
        fw_trimmed,
        "\u{2500}".repeat(right_len)
    ))
}

pub fn refit_line(line: &str, target_cols: usize) -> String {
    if target_cols == 0 {
        return line.to_string();
    }
    if let Some(h) = refit_header(line, target_cols) {
        return h;
    }
    let plain = multitop_agent::color::strip_ansi(line);
    let trimmed = plain.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c == '\u{2500}') {
        let rule_w = target_cols.saturating_sub(2);
        return format!(" \x1b[90m{}\x1b[0m", "\u{2500}".repeat(rule_w));
    }
    line.to_string()
}

/// Show the tail when there is more content than room, optionally pinning
/// the header (line 0) so the server name stays visible.
fn visible(lines: &[String], height: usize, _pin_header: bool, target_cols: usize) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = if lines.len() <= height {
        lines.to_vec()
    } else {
        lines[..height].to_vec()
    };

    if !out.is_empty() && target_cols > 0 {
        for line in out.iter_mut() {
            *line = refit_line(line, target_cols);
        }
    }

    out
}

fn keybar_line() -> Line<'static> {
    let key = Style::default().fg(Color::White);
    let label = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(" ESC / Q", key),
        Span::styled(" Quit  ", label),
        Span::styled("F", key),
        Span::styled(" Fetch  ", label),
        Span::styled("D", key),
        Span::styled(" Docker  ", label),
        Span::styled("S", key),
        Span::styled(" Stats  ", label),
        Span::styled("U", key),
        Span::styled(" Upgrade", label),
    ])
}

pub fn draw(f: &mut Frame, app: &App) {
    let (panel_areas, keybar) = regions(f.area(), app.panels.len());

    for (panel, area) in app.panels.iter().zip(panel_areas) {
        let inner = Rect {
            x: area.x + SIDE_MARGIN,
            y: area.y,
            width: area.width.saturating_sub(SIDE_MARGIN * 2),
            height: area.height,
        };
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        let lines = visible(&panel.view, inner.height as usize, true, inner.width as usize);
        // No wrapping: frames are pre-formatted to the width we asked for,
        // and wrapping a bar chart turns one row into two and breaks the
        // whole panel's alignment.
        f.render_widget(Paragraph::new(ansi::to_text(&lines)), inner);
    }

    f.render_widget(
        Paragraph::new(keybar_line()).style(Style::default().bg(Color::Indexed(236))),
        keybar,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: u16, h: u16) -> Size {
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
        // Panels 0 and 1 are in top row, panel 2 is in bottom row
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
        assert_eq!(visible(&lines, 10, false, 0).len(), 3);
        assert_eq!(visible(&lines, 3, false, 0).len(), 3);
    }

    #[test]
    fn visible_preserves_header_and_top_metrics() {
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
        let shown = visible(&lines, 6, true, 0);
        assert_eq!(shown.len(), 6);
        assert_eq!(shown[0], "HEADER");
        assert_eq!(shown[1], "CPU 50%");
        assert_eq!(shown[2], "MEM 40%");
        assert_eq!(shown[3], "DSK 30%");
        assert_eq!(shown[4], "RULE");
        assert_eq!(shown[5], "PROC HDR");
    }

    #[test]
    fn visible_handles_zero_height() {
        let lines: Vec<String> = (0..3).map(|i| i.to_string()).collect();
        assert!(visible(&lines, 0, false, 0).is_empty());
    }

    #[test]
    fn keybar_lists_every_binding() {
        let text: String = keybar_line()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        for hint in ["ESC", "Quit", "F", "Fetch", "D", "Docker", "S", "Stats", "U", "Upgrade"] {
            assert!(text.contains(hint), "missing {hint} in {text:?}");
        }
    }
}
