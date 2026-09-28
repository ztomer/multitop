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
    let rows = panels.div_ceil(2);
    let grid_rows: u32 = u32::try_from(rows).unwrap_or(u32::MAX);
    let v_chunks = Layout::vertical(vec![Constraint::Ratio(1, grid_rows); rows]).split(body);
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
///
/// Measured from `regions` rather than re-derived. This used to be a second
/// copy of the grid arithmetic -- its own `1 => (1, 1), 2 => (1, 2), n => (2,
/// n/2)` table beside the one above -- and two copies of a layout are two
/// layouts that agree until one of them is edited.
///
/// The *smallest* pane, not the first: an uneven split gives one pane a row or
/// a column more than another, and a frame rendered for the larger one is
/// clipped in the smaller. Rendering to the smallest is the size every pane can
/// hold.
#[must_use]
pub fn agent_dims(size: Size, panels: usize) -> (u16, u16) {
    let (grid, _) = regions(Rect::new(0, 0, size.width, size.height), panels);
    let cols = grid
        .iter()
        .map(|p| p.width.saturating_sub(SIDE_MARGIN * 2))
        .min()
        .unwrap_or(0)
        .max(MIN_AGENT_COLS);
    let rows = grid
        .iter()
        .map(|p| p.height)
        .min()
        .unwrap_or(0)
        .max(MIN_AGENT_ROWS);
    (cols, rows)
}

#[cfg(test)]
mod agent_dims_tests {
    use super::*;
    use ratatui::layout::Size;

    fn size(w: u16, h: u16) -> Size {
        Size {
            width: w,
            height: h,
        }
    }

    /// The property a run depends on, and the reason it is a test rather than a
    /// comment: the size a remote child is told its pty has must be the size of
    /// the pane its output is drawn into, for **every** panel count.
    ///
    /// This is the multi-panel answer, pinned. It was a live question -- with
    /// four hosts in a 2x2 grid on a 200-column terminal, is the upgrade told
    /// 198 columns (the screen) or 98 (its own pane)? -- and the answer is the
    /// pane, because `agent_dims` and `ui::draw` both measure `regions()` and
    /// there is no second copy of the grid arithmetic to disagree with.
    #[test]
    fn the_published_size_is_the_pane_a_run_is_drawn_into() {
        for panels in 1..=9usize {
            let term = size(200, 40);
            let (cols, rows) = agent_dims(term, panels);
            let (grid, _) = regions(Rect::new(0, 0, term.width, term.height), panels);
            for pane in &grid {
                let drawable = pane.width.saturating_sub(SIDE_MARGIN * 2);
                assert!(
                    cols <= drawable,
                    "{panels} panels: told {cols} columns for a pane {drawable} wide"
                );
                assert!(
                    rows <= pane.height,
                    "{panels} panels: told {rows} rows for a pane {} tall",
                    pane.height
                );
            }
        }
    }

    /// And for a single panel -- the case almost every upgrade runs in -- it is
    /// exactly the pane, not a rounded-down or padded approximation of it.
    #[test]
    fn a_single_panel_is_told_its_own_width() {
        assert_eq!(agent_dims(size(200, 40), 1), (198, 39));
        assert_eq!(agent_dims(size(80, 24), 1), (78, 23));
    }

    /// The floor is the render floor, and it is why a very narrow terminal
    /// cannot shrink the geometry further: below it the agent's own column
    /// arithmetic degenerates, so the pane clips rather than the child being
    /// asked for a window nothing can honour.
    #[test]
    fn a_terminal_too_narrow_to_honour_still_gets_the_render_floor() {
        // 20 columns wide: 18 drawable, floored to 40. The height is the pane's
        // own, because a 9-row pane is above the floor.
        assert_eq!(agent_dims(size(20, 10), 1), (MIN_AGENT_COLS, 9));
    }

    /// Columns are always equal across a row; heights can differ by one when the
    /// split lands oddly. `agent_dims` takes the minimum on purpose -- a frame
    /// rendered for the taller pane is clipped in the shorter one.
    #[test]
    fn the_smaller_pane_governs() {
        for h in 12..40u16 {
            let (grid, _) = regions(Rect::new(0, 0, 200, h), 2);
            let shortest = grid.iter().map(|p| p.height).min().unwrap_or(0);
            assert!(
                agent_dims(size(200, h), 2).1 <= shortest,
                "height {h}: published more rows than the shortest pane has"
            );
        }
    }

    /// The floor is a floor, and past it the published size stops describing
    /// the pane. Stated because the test above would otherwise read as a
    /// universal claim, and it is not one.
    ///
    /// Twelve rows of terminal, two panels: six and five rows each, and the
    /// minimum is floored to four -- which happens to fit. Eight rows is where
    /// it stops: three rows a pane, four published, and the agent renders a
    /// row the pane cannot show. That is the intended trade (a geometry below
    /// the floor is worse than a clipped row) and the only case where the
    /// published size is larger than the space, so it is named rather than left
    /// for the next reader to trip over.
    #[test]
    fn the_floor_can_exceed_a_pane_that_is_tiny() {
        let (grid, _) = regions(Rect::new(0, 0, 200, 8), 2);
        let shortest = grid.iter().map(|p| p.height).min().unwrap_or(0);
        assert_eq!(
            shortest, 3,
            "the premise: 8 rows cannot give two panels four each"
        );
        assert_eq!(
            agent_dims(size(200, 8), 2).1,
            MIN_AGENT_ROWS,
            "and the published height is the floor, not the pane"
        );
    }
}
