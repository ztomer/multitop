use ratatui::layout::{Constraint, Layout, Rect, Size};

//  Layout and drawing.

/// Rows reserved at the bottom for the key hints.
pub const KEYBAR_H: u16 = 1;
/// One blank column either side of a panel's contents.
pub const SIDE_MARGIN: u16 = 1;

/// Minimum panel width the agent is asked to render into. Below this the
/// layout stops being readable anyway, and a too-small width makes the
/// agent's own column arithmetic degenerate.
pub const MIN_AGENT_COLS: u16 = 40;
pub const MIN_AGENT_ROWS: u16 = 4;

/// Split the screen into one region per panel plus the key bar.
#[must_use]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc, clippy::expect_used)]
pub fn regions(area: Rect, panels: usize) -> (Vec<Rect>, Rect) {
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
    // One row per PAIR of panels, not one row per panel.
    let grid_rows: u32 = u32::try_from(panels.div_ceil(2)).expect("too many panels");
    let v_chunks =
        Layout::vertical(vec![Constraint::Ratio(1, grid_rows); grid_rows as usize]).split(body);
    let mut rects = Vec::with_capacity(panels);
    for (r_idx, row_rect) in v_chunks.iter().enumerate() {
        let h_chunks = Layout::horizontal([
            Constraint::Ratio(1, grid_cols),
            Constraint::Ratio(1, grid_cols),
        ])
        .split(*row_rect);
        for (c_idx, col_rect) in h_chunks.iter().enumerate() {
            if r_idx * 2 + c_idx < panels {
                rects.push(*col_rect);
            }
        }
    }
    (rects, keybar)
}

/// The panel size to tell the agent about, so its frames arrive pre-fitted.
#[must_use]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc, clippy::expect_used)]
pub fn agent_dims(size: Size, panels: usize) -> (u16, u16) {
    if panels == 0 {
        return (MIN_AGENT_COLS, MIN_AGENT_ROWS);
    }
    let body_h = size.height.saturating_sub(KEYBAR_H);
    let (grid_cols, grid_rows) = match panels {
        1 => (1u16, 1u16),
        2 => (1u16, 2u16),
        n => (2u16, u16::try_from(n).expect("too many panels").div_ceil(2)),
    };
    let cols = (size.width / grid_cols)
        .saturating_sub(SIDE_MARGIN * 2)
        .max(MIN_AGENT_COLS);
    let rows = (body_h / grid_rows).max(MIN_AGENT_ROWS);
    (cols, rows)
}
