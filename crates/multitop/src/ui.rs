//! Layout and drawing.

use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ansi;
use crate::app::App;

/// Rows reserved at the bottom for the key hints.
pub const KEYBAR_H: u16 = 1;
/// One blank column either side of a panel's contents.
const SIDE_MARGIN: u16 = 1;

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

pub use crate::refit::{refit_header, refit_line};

/// Show the tail when there is more content than room, optionally pinning
/// the header (line 0) so the server name stays visible. Supports scrolling via `scroll_offset`.
#[must_use]
pub fn visible(
    lines: &[String],
    height: usize,
    pinned: usize,
    target_cols: usize,
    scroll_offset: usize,
) -> Vec<String> {
    if lines.is_empty() || height == 0 {
        return Vec::new();
    }
    if lines.len() <= height {
        let mut out = lines.to_vec();
        if !out.is_empty() && target_cols > 0 {
            for line in &mut out {
                *line = refit_line(line, target_cols);
            }
        }
        return out;
    }

    // The pinned block must never crowd the body out, or a tall header on a
    // short panel would leave a single row for the output it is describing.
    // Half the panel is the most it may take; the banner always survives.
    let pinned = pinned
        .min(lines.len())
        .min((height / 2).max(1))
        .min(height.saturating_sub(1));
    let body_budget = height.saturating_sub(pinned.max(1));
    let mut out = Vec::with_capacity(height);
    let mut badge_offset = 0;

    if pinned > 0 && lines.len() > pinned {
        let body_lines = &lines[pinned..];
        let max_offset = body_lines.len().saturating_sub(body_budget);
        let eff_offset = scroll_offset.min(max_offset);
        badge_offset = eff_offset;

        let end = body_lines.len().saturating_sub(eff_offset);
        let start = end.saturating_sub(body_budget);

        out.extend_from_slice(&lines[..pinned]);
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
            if i == 0 && badge_offset > 0 && pinned > 0 {
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

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn keybar_line(
    sort: multitop_agent::SortBy,
    theme: &multitop_agent::color::Palette,
    keybar_width: u16,
    active_mode: crate::app::Mode,
) -> Line<'static> {
    const SPACES: &str = "                                                                                                                                                                                                                                                                ";
    let label = Style::default().fg(Color::DarkGray);
    let active = Style::default().fg(Color::White);
    let inactive = Style::default().fg(Color::DarkGray);
    let border_color = Color::Rgb(
        theme.ratatui_border.0,
        theme.ratatui_border.1,
        theme.ratatui_border.2,
    );
    let accent_color = Color::Rgb(
        theme.ratatui_accent.0,
        theme.ratatui_accent.1,
        theme.ratatui_accent.2,
    );
    let active_mode_style = Style::default()
        .bg(accent_color)
        .fg(Color::Black)
        .add_modifier(ratatui::style::Modifier::BOLD);
    let sort_label = Style::default().fg(border_color);
    let theme_val_style = Style::default().fg(accent_color);

    let key_hi = Style::default()
        .fg(Color::White)
        .add_modifier(ratatui::style::Modifier::BOLD);

    let f_hi = if active_mode == crate::app::Mode::Fetch {
        active_mode_style
    } else {
        key_hi
    };
    let f_lbl = if active_mode == crate::app::Mode::Fetch {
        active_mode_style
    } else {
        label
    };
    let d_hi = if active_mode == crate::app::Mode::Docker {
        active_mode_style
    } else {
        key_hi
    };
    let d_lbl = if active_mode == crate::app::Mode::Docker {
        active_mode_style
    } else {
        label
    };
    let s_hi = if active_mode == crate::app::Mode::Monitor {
        active_mode_style
    } else {
        key_hi
    };
    let s_lbl = if active_mode == crate::app::Mode::Monitor {
        active_mode_style
    } else {
        label
    };
    let u_hi = if active_mode == crate::app::Mode::Upgrade {
        active_mode_style
    } else {
        key_hi
    };
    let u_lbl = if active_mode == crate::app::Mode::Upgrade {
        active_mode_style
    } else {
        label
    };
    let left_spans = [
        Span::styled("ESC / ", label),
        Span::styled("Q", key_hi),
        Span::styled("uit  ", label),
        Span::styled("S", s_hi),
        Span::styled("tats", s_lbl),
        Span::styled("  ", label),
        Span::styled("D", d_hi),
        Span::styled("ocker", d_lbl),
        Span::styled("  ", label),
        Span::styled("F", f_hi),
        Span::styled("etch", f_lbl),
        Span::styled("  ", label),
        Span::styled("U", u_hi),
        // In the Upgrade view the same key starts the run, so say which of the
        // two it will do rather than leaving the second press undiscoverable.
        Span::styled(
            if active_mode == crate::app::Mode::Upgrade {
                "pgrade: run"
            } else {
                "pgrade"
            },
            u_lbl,
        ),
        Span::styled("  ", label),
    ];
    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();

    let (mem_style, cpu_style) = match sort {
        multitop_agent::SortBy::Mem => (active, inactive),
        multitop_agent::SortBy::Cpu => (inactive, active),
    };
    let theme_name_padded = format!("{:<11}", theme.name);
    let badge_spans = [
        Span::styled("[", sort_label),
        Span::styled("S", label),
        Span::styled("E", key_hi),
        Span::styled("ttings", label),
        Span::styled("]  ", sort_label),
        Span::styled("[", sort_label),
        Span::styled(
            "T",
            Style::default()
                .fg(accent_color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled("heme: ", sort_label),
        Span::styled(theme_name_padded, theme_val_style),
        Span::styled("]  ", sort_label),
        Span::styled("[Sort: ", sort_label),
        Span::styled("C", key_hi),
        Span::styled("pu", cpu_style),
        Span::styled("/ ", sort_label),
        Span::styled("M", key_hi),
        Span::styled("em", mem_style),
        Span::styled("]", sort_label),
    ];
    let badge_width: usize = badge_spans.iter().map(|s| s.content.len()).sum();
    let pad = (keybar_width as usize).saturating_sub(left_width + badge_width);
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
#[allow(clippy::too_many_lines)]
pub fn draw(f: &mut Frame, app: &App) {
    if app.password_manager.is_some() {
        crate::config_ui::draw(f, app);
        return;
    }
    let (panel_areas, keybar) = regions(f.area(), app.panels.len());
    let theme = app.current_theme();
    let bg_color = Color::Rgb(
        theme.ratatui_keybar_bg.0,
        theme.ratatui_keybar_bg.1,
        theme.ratatui_keybar_bg.2,
    );

    for ((idx, panel), area) in app.panels.iter().enumerate().zip(&panel_areas) {
        let inner = Rect {
            x: area.x + SIDE_MARGIN,
            y: area.y,
            width: area.width.saturating_sub(SIDE_MARGIN * 2),
            height: area.height,
        };
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        let mut lines = visible(
            &panel.view,
            inner.height as usize,
            panel.pinned_lines.max(1),
            inner.width as usize,
            panel.scroll_offset,
        );
        if !lines.is_empty() {
            let host_name = panel
                .last_monitor
                .as_ref()
                .and_then(|p| match p {
                    multitop_agent::proto::Payload::Monitor(snap) => Some(snap.host.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panel.server.host.clone());

            let server_target = if !panel.server.user.is_empty()
                && !panel.server.user.eq_ignore_ascii_case("default")
            {
                format!("{}@{}", panel.server.user, host_name)
            } else {
                host_name
            };
            let total_w = inner.width as usize;
            let disp_w = multitop_agent::fmt::fullwidth_display_width(&server_target);
            let space_needed = disp_w + 2;

            if total_w >= space_needed {
                let rem = total_w - space_needed;
                let left_rule_len = rem / 2;
                let right_rule_len = rem - left_rule_len;

                let mem_bar = app
                    .sparklines_mem
                    .get(idx)
                    .map(|s| s.render_bar_limited(left_rule_len.saturating_sub(2)))
                    .unwrap_or_default();
                let cpu_bar = app
                    .sparklines_cpu
                    .get(idx)
                    .map(|s| s.render_bar_limited(right_rule_len.saturating_sub(2)))
                    .unwrap_or_default();

                let (left_str, mem_used_len) =
                    if app.show_sparklines() && !mem_bar.is_empty() && left_rule_len >= 3 {
                        let text = format!("M:{mem_bar}");
                        let len = text.chars().count();
                        (format!("\x1b[36;1m{text}\x1b[0m"), len)
                    } else {
                        (String::new(), 0)
                    };

                let (right_str, cpu_used_len) =
                    if app.show_sparklines() && !cpu_bar.is_empty() && right_rule_len >= 3 {
                        let text = format!("C:{cpu_bar}");
                        let len = text.chars().count();
                        (format!("\x1b[33;1m{text}\x1b[0m"), len)
                    } else {
                        (String::new(), 0)
                    };

                let left_rule_rem = left_rule_len.saturating_sub(mem_used_len);
                let right_rule_rem = right_rule_len.saturating_sub(cpu_used_len);
                let fw = multitop_agent::fmt::fullwidth(&server_target);

                lines[0] = format!(
                    "{left_str}{}{}{}{}{}{} {}{}{}{}{right_str}",
                    theme.secondary(),
                    "\u{2500}".repeat(left_rule_rem),
                    theme.reset,
                    theme.primary(),
                    theme.bold,
                    fw,
                    theme.reset,
                    theme.secondary(),
                    "\u{2500}".repeat(right_rule_rem),
                    theme.reset,
                );
            } else {
                lines[0] = multitop_agent::fmt::center_header(&server_target, total_w, theme);
            }
        }
        f.render_widget(Paragraph::new(ansi::to_text(&lines)), inner);
    }
    let active_mode = app
        .panels
        .get(app.selected_panel)
        .or_else(|| app.panels.first())
        .map_or(crate::app::Mode::Monitor, |p| p.mode);

    f.render_widget(
        Paragraph::new(keybar_line(app.sort, theme, keybar.width, active_mode))
            .style(Style::default().bg(bg_color)),
        keybar,
    );

    if app.vault_awaiting_biometric() || app.vault_verifying() {
        crate::modals::draw_vault_awaiting_biometric(f, app.vault_verifying());
    } else if app.show_vault_password_prompt() || app.vault_creating() {
        crate::modals::draw_vault_password_prompt(f, app);
    } else if app.show_upgrade_modal() {
        crate::modals::draw_upgrade_modal(f, app);
    }
}
