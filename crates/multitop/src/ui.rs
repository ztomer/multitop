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
/// the header (line 0) so the server name stays visible. Supports scrolling via `scroll_offset`.
fn visible(
    lines: &[String],
    height: usize,
    pin_header: bool,
    target_cols: usize,
    scroll_offset: usize,
) -> Vec<String> {
    if lines.is_empty() || height == 0 {
        return Vec::new();
    }
    if lines.len() <= height {
        let mut out = lines.to_vec();
        if !out.is_empty() && target_cols > 0 {
            for line in out.iter_mut() {
                *line = refit_line(line, target_cols);
            }
        }
        return out;
    }

    let body_budget = height.saturating_sub(1);
    let mut out = Vec::with_capacity(height);
    let mut badge_offset = 0;

    if pin_header && lines.len() > 1 {
        let body_lines = &lines[1..];
        let max_offset = body_lines.len().saturating_sub(body_budget);
        let eff_offset = scroll_offset.min(max_offset);
        badge_offset = eff_offset;

        let end = body_lines.len().saturating_sub(eff_offset);
        let start = end.saturating_sub(body_budget);

        out.push(lines[0].clone());
        out.extend_from_slice(&body_lines[start..end]);
    } else {
        let max_offset = lines.len().saturating_sub(height);
        let eff_offset = scroll_offset.min(max_offset);
        let end = lines.len().saturating_sub(eff_offset);
        let start = end.saturating_sub(height);
        out.extend_from_slice(&lines[start..end]);
    }

    if !out.is_empty() && target_cols > 0 {
        for (i, line) in out.iter_mut().enumerate() {
            if i == 0 && badge_offset > 0 && pin_header {
                let badge = format!(" [\u{2191} -{badge_offset} lines] ");
                let target_w = target_cols.saturating_sub(badge.chars().count());
                let refitted = refit_line(line, target_w);
                *line = format!("{refitted}\x1b[33;1m{badge}\x1b[0m");
            } else {
                *line = refit_line(line, target_cols);
            }
        }
    }

    out
}

fn keybar_line(sort: multitop_agent::SortBy, theme: &multitop_agent::color::Palette, keybar_width: u16) -> Line<'static> {
    let key = Style::default().fg(Color::White);
    let label = Style::default().fg(Color::DarkGray);
    let active = Style::default().fg(Color::White);
    let inactive = Style::default().fg(Color::DarkGray);
    let border_color = Color::Rgb(theme.ratatui_border.0, theme.ratatui_border.1, theme.ratatui_border.2);
    let accent_color = Color::Rgb(theme.ratatui_accent.0, theme.ratatui_accent.1, theme.ratatui_accent.2);
    let sort_label = Style::default().fg(border_color);
    let theme_val_style = Style::default().fg(accent_color);

    let left_spans = [
        Span::styled(" ESC / Q", key),
        Span::styled(" Quit  ", label),
        Span::styled("F", key),
        Span::styled(" Fetch  ", label),
        Span::styled("D", key),
        Span::styled(" Docker  ", label),
        Span::styled("S", key),
        Span::styled(" Stats  ", label),
        Span::styled("U", key),
        Span::styled(" Upgrade  ", label),
        Span::styled("T", key),
        Span::styled(" Theme", label),
    ];
    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();

    let (mem_style, cpu_style) = match sort {
        multitop_agent::SortBy::Mem => (active, inactive),
        multitop_agent::SortBy::Cpu => (inactive, active),
    };

    let theme_name_padded = format!("{:<11}", theme.name);
    let badge_spans = [
        Span::styled("[", sort_label),
        Span::styled("T", key),
        Span::styled("heme: ", sort_label),
        Span::styled(theme_name_padded, theme_val_style),
        Span::styled("]  ", sort_label),
        Span::styled("[Sort by: ", sort_label),
        Span::styled("Mem", mem_style),
        Span::styled("/ ", sort_label),
        Span::styled("Cpu", cpu_style),
        Span::styled("]", sort_label),
    ];
    let badge_width: usize = badge_spans.iter().map(|s| s.content.len()).sum();

    let pad = (keybar_width as usize).saturating_sub(left_width + badge_width);
    const SPACES: &str = "                                                                                                                                                                                                                                                                ";
    let pad_str = if pad <= SPACES.len() {
        &SPACES[..pad]
    } else {
        SPACES
    };

    let mut spans = Vec::with_capacity(left_spans.len() + 1 + badge_spans.len());
    spans.extend(left_spans);
    spans.push(Span::styled(pad_str, label));
    spans.extend(badge_spans);
    Line::from(spans)
}

pub fn draw(f: &mut Frame, app: &App) {
    let (panel_areas, keybar) = regions(f.area(), app.panels.len());
    let theme = app.current_theme();
    let bg_color = Color::Rgb(theme.ratatui_keybar_bg.0, theme.ratatui_keybar_bg.1, theme.ratatui_keybar_bg.2);

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
        let lines = visible(&panel.view, inner.height as usize, true, inner.width as usize, panel.scroll_offset);
        // No wrapping: frames are pre-formatted to the width we asked for,
        // and wrapping a bar chart turns one row into two and breaks the
        // whole panel's alignment.
        f.render_widget(Paragraph::new(ansi::to_text(&lines)), inner);
    }

    f.render_widget(
        Paragraph::new(keybar_line(app.sort, theme, keybar.width)).style(Style::default().bg(bg_color)),
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
        assert_eq!(visible(&lines, 10, false, 0, 0).len(), 3);
        assert_eq!(visible(&lines, 3, false, 0, 0).len(), 3);
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
        let shown = visible(&lines, 6, true, 0, 0);
        assert_eq!(shown.len(), 6);
        assert_eq!(shown[0], "HEADER");
        assert_eq!(shown[1], "p1");
        assert_eq!(shown[5], "p5");
    }

    #[test]
    fn visible_handles_zero_height() {
        let lines: Vec<String> = (0..3).map(|i| i.to_string()).collect();
        assert!(visible(&lines, 0, false, 0, 0).is_empty());
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
        let tail = visible(&lines, 4, true, 0, 0);
        assert_eq!(tail[0], "HEADER");
        assert_eq!(tail[1], "line 8");
        assert_eq!(tail[3], "line 10");

        let scrolled = visible(&lines, 4, true, 0, 3);
        assert_eq!(scrolled[0], "HEADER");
        assert_eq!(scrolled[1], "line 5");
        assert_eq!(scrolled[3], "line 7");
    }

    #[test]
    fn keybar_lists_every_binding() {
        let theme = &multitop_agent::color::KARE;
        let text: String = keybar_line(multitop_agent::SortBy::Cpu, theme, 120)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        for hint in ["ESC", "Quit", "F", "Fetch", "D", "Docker", "S", "Stats", "U", "Upgrade", "T", "Theme"] {
            assert!(text.contains(hint), "missing {hint} in {text:?}");
        }
    }

    #[test]
    fn keybar_shows_sort_by_cpu_and_theme() {
        let theme = &multitop_agent::color::KARE;
        let text: String = keybar_line(multitop_agent::SortBy::Cpu, theme, 120)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("heme: Kare"), "theme indicator missing");
        assert!(text.contains("[Sort by:"), "sort indicator missing");
        assert!(text.contains("Cpu"), "CPU sort key missing from keybar");
    }

    #[test]
    fn keybar_shows_sort_by_mem() {
        let theme = &multitop_agent::color::KARE;
        let text: String = keybar_line(multitop_agent::SortBy::Mem, theme, 120)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("[Sort by:"), "sort indicator missing");
        assert!(text.contains("Mem"), "Memory sort key missing from keybar");
    }
}
