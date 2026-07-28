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
    let rows = Layout::vertical(vec![Constraint::Ratio(1, panels as u32); panels]).split(body);
    (rows.to_vec(), keybar)
}

/// The panel size to tell the agent about, so its frames arrive pre-fitted.
pub fn agent_dims(size: Size, panels: usize) -> (u16, u16) {
    if panels == 0 {
        return (MIN_AGENT_COLS, MIN_AGENT_ROWS);
    }
    let body_h = size.height.saturating_sub(KEYBAR_H);
    let cols = size
        .width
        .saturating_sub(SIDE_MARGIN * 2)
        .max(MIN_AGENT_COLS);
    let rows = (body_h / panels as u16).max(MIN_AGENT_ROWS);
    (cols, rows)
}

/// Show the tail when there is more content than room — for streamed command
/// output the newest lines are the interesting ones.
fn visible(lines: &[String], height: usize) -> &[String] {
    if lines.len() <= height {
        lines
    } else {
        &lines[lines.len() - height..]
    }
}

fn keybar_line() -> Line<'static> {
    let key = Style::default().fg(Color::White);
    let label = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(" ESC", key),
        Span::styled(" Quit  ", label),
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
        let lines = visible(&panel.view, inner.height as usize);
        // No wrapping: frames are pre-formatted to the width we asked for,
        // and wrapping a bar chart turns one row into two and breaks the
        // whole panel's alignment.
        f.render_widget(Paragraph::new(ansi::to_text(lines)), inner);
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
        assert_eq!(cols, 98, "one column of margin each side");
        assert_eq!(rows, 10, "30 body rows over 3 panels");
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
    fn regions_do_not_overlap() {
        let (panels, keybar) = regions(Rect::new(0, 0, 80, 30), 3);
        for pair in panels.windows(2) {
            assert_eq!(pair[0].y + pair[0].height, pair[1].y);
        }
        let last = panels.last().unwrap();
        assert!(last.y + last.height <= keybar.y);
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
        assert_eq!(visible(&lines, 10).len(), 3);
        assert_eq!(visible(&lines, 3).len(), 3);
    }

    /// Overflowing output must keep its tail: for a running command, the last
    /// lines are the result.
    #[test]
    fn visible_keeps_the_tail() {
        let lines: Vec<String> = (0..100).map(|i| i.to_string()).collect();
        let shown = visible(&lines, 5);
        assert_eq!(shown.len(), 5);
        assert_eq!(shown[0], "95");
        assert_eq!(shown[4], "99");
    }

    #[test]
    fn visible_handles_zero_height() {
        let lines: Vec<String> = (0..3).map(|i| i.to_string()).collect();
        assert!(visible(&lines, 0).is_empty());
    }

    #[test]
    fn keybar_lists_every_binding() {
        let text: String = keybar_line()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        for hint in ["ESC", "Quit", "D", "Docker", "S", "Stats", "U", "Upgrade"] {
            assert!(text.contains(hint), "missing {hint} in {text:?}");
        }
    }
}
