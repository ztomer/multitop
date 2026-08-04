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
///
/// Returns the rows to draw and **how far back the view is scrolled**, for the
/// caller to render as a badge.
///
/// # Row 0 has one owner
///
/// This used to compose the scroll badge onto `out[0]` itself. `draw` then
/// overwrote `lines[0]` with the host banner, unconditionally, on every frame --
/// so the badge was built and destroyed within one frame and the scroll-position
/// indicator has never once been on screen. Two pieces of code believed they
/// owned that row and the later one silently won.
///
/// Handing the offset back as a value, rather than as text already baked into a
/// row somebody else is about to overwrite, is what makes that unrepresentable.
#[must_use]
pub fn visible(
    lines: &[String],
    height: usize,
    pinned: usize,
    target_cols: usize,
    scroll_offset: usize,
) -> (Vec<String>, usize) {
    if lines.is_empty() || height == 0 {
        return (Vec::new(), 0);
    }
    if lines.len() <= height {
        let mut out = lines.to_vec();
        if target_cols > 0 {
            for line in &mut out {
                *line = refit_line(line, target_cols);
            }
        }
        return (out, 0);
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
        for line in &mut out {
            *line = refit_line(line, target_cols);
        }
    }

    (out, if pinned > 0 { badge_offset } else { 0 })
}

#[must_use]
#[allow(clippy::too_many_lines)]
/// What the keybar should say about filtering.
///
/// An enum rather than a `&str` plus an `is_editing` flag: the two states need
/// different lines, and a pair of arguments that must agree is how they end up
/// disagreeing.
#[derive(Clone, Copy, Debug)]
pub enum FilterHint<'a> {
    /// No filter, nothing being typed.
    Off,
    /// The user is typing a query right now.
    Editing(&'a str),
    /// A filter is in force but not being edited. This must be visible: panels
    /// are hidden, and a monitor that silently stops showing a host is worse
    /// than one that shows it failing.
    Active(&'a str),
}

/// The key letter and its label for one view, highlighted when that view is on.
///
/// Six copies of this if/else pair inline are what made `keybar_line` too long
/// to read, and the sixth was the one that had to be edited to add a view.
fn mode_pair(
    active_mode: crate::app::Mode,
    this: crate::app::Mode,
    on: Style,
    key_off: Style,
    label_off: Style,
) -> (Style, Style) {
    if active_mode == this {
        (on, on)
    } else {
        (key_off, label_off)
    }
}

/// The prompt shown in place of the keybar while a query is being typed.
fn filter_prompt(query: &str, label: Style, accent: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled("Filter: ", label),
        Span::styled(query.to_string(), Style::default().fg(accent)),
        Span::styled("\u{2588}", Style::default().fg(accent)),
        Span::styled("   [Enter] keep  [Esc] clear", label),
    ])
}

/// The right-hand badges, as three whole units with their widths.
///
/// Three units, not seventeen loose spans, because they are shed whole or not
/// at all -- and the previous flat `Vec<Span>` gave the caller no way to know
/// where one badge ended and the next began, so it could only be guillotined.
///
/// The `{:<11}` pad on the theme name is gone with it: it spent seven dead
/// columns on a four-letter word at exactly the width where the bar overflows,
/// which both Rams and Kare called out independently.
fn keybar_badges(
    sort: multitop_agent::SortBy,
    theme: &multitop_agent::color::Palette,
    label: Style,
    key_hi: Style,
    sort_label: Style,
    accent_color: Color,
) -> Vec<(usize, Vec<Span<'static>>)> {
    let active = Style::default().fg(Color::White);
    let inactive = Style::default().fg(Color::DarkGray);
    let theme_val_style = Style::default().fg(accent_color);
    let (mem_style, cpu_style) = match sort {
        multitop_agent::SortBy::Mem => (active, inactive),
        multitop_agent::SortBy::Cpu => (inactive, active),
    };
    let badges = vec![
        // `[E] Settings`, not `[SEttings]`: every other key in the bar
        // highlights the first letter, so highlighting the second here made the
        // one mnemonic that has to be explained rather than seen.
        vec![
            Span::styled("[", sort_label),
            Span::styled("E", key_hi),
            Span::styled("] Settings", label),
        ],
        vec![
            Span::styled("[", sort_label),
            Span::styled(
                "T",
                Style::default()
                    .fg(accent_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled("heme: ", sort_label),
            Span::styled(theme.name.to_string(), theme_val_style),
            Span::styled("]", sort_label),
        ],
        vec![
            Span::styled("[Sort: ", sort_label),
            Span::styled("C", key_hi),
            Span::styled("pu", cpu_style),
            // Was "/ ", which rendered as `Cpu/ Mem`.
            Span::styled("/", sort_label),
            Span::styled("M", key_hi),
            Span::styled("em", mem_style),
            Span::styled("]", sort_label),
        ],
    ];
    badges
        .into_iter()
        .map(|spans| (span_width(&spans), spans))
        .collect()
}

/// The scroll badge, coloured, or nothing at all when not scrolled back.
fn badge_span(badge: &str) -> String {
    if badge.is_empty() {
        String::new()
    } else {
        format!("\x1b[33;1m{badge}\x1b[0m")
    }
}

/// Display width of a run of spans.
fn span_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// The keybar for a terminal too narrow for words: one letter per key.
///
/// One chunk per key, so the narrow end sheds by the same rule as the wide end
/// rather than being clipped. `Q` is deliberately absent from the shed order --
/// quit is the one thing a user stuck in a twelve-column terminal most needs to
/// find, and it is the only binding here that cannot be discovered by trying.
fn keybar_initials(
    keys: &[(&'static str, Style)],
    keybar_width: u16,
    label: Style,
    filter: FilterHint<'_>,
    accent: Color,
) -> Line<'static> {
    let mut chunks: Vec<Vec<Span<'static>>> = keys
        .iter()
        .map(|(text, style)| vec![Span::styled(*text, *style)])
        .collect();
    if let FilterHint::Active(query) = filter {
        chunks.push(vec![Span::styled(
            format!("[{query}]"),
            Style::default().fg(accent),
        )]);
    }
    let widths: Vec<usize> = chunks.iter().map(|c| span_width(c)).collect();
    // Shed the views before the doors: Fetch, Docker, Upgrade, Stats, then
    // Filter and Settings.
    let kept = crate::layout::fit_row(&widths, 2, keybar_width as usize, &[3, 2, 4, 1, 5, 6]);
    let mut out = Vec::new();
    for (n, index) in kept.iter().enumerate() {
        if n > 0 {
            out.push(Span::styled("  ", label));
        }
        out.extend(chunks[*index].clone());
    }
    Line::from(out)
}

#[must_use]
pub fn keybar_line(
    sort: multitop_agent::SortBy,
    theme: &multitop_agent::color::Palette,
    keybar_width: u16,
    active_mode: crate::app::Mode,
    filter: FilterHint<'_>,
) -> Line<'static> {
    const SPACES: &str = "                                                                                                                                                                                                                                                                ";
    let label = Style::default().fg(Color::DarkGray);
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

    let key_hi = Style::default()
        .fg(Color::White)
        .add_modifier(ratatui::style::Modifier::BOLD);

    // While typing, the keybar becomes the prompt. Reusing the row avoids
    // moving the panels underneath, which would reflow the whole grid on the
    // first keystroke.
    if let FilterHint::Editing(query) = filter {
        return filter_prompt(query, label, accent_color);
    }

    let pair = |m| mode_pair(active_mode, m, active_mode_style, key_hi, label);
    let (f_hi, f_lbl) = pair(crate::app::Mode::Fetch);
    let (d_hi, d_lbl) = pair(crate::app::Mode::Docker);
    let (s_hi, s_lbl) = pair(crate::app::Mode::Monitor);
    let (u_hi, u_lbl) = pair(crate::app::Mode::Upgrade);
    let upgrade_word = if active_mode == crate::app::Mode::Upgrade {
        // In the Upgrade view the same key starts the run, so say which of the
        // two it will do rather than leaving the second press undiscoverable.
        "pgrade: run"
    } else {
        "pgrade"
    };
    let mut left_spans = vec![
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
        Span::styled(upgrade_word, u_lbl),
        Span::styled("  ", label),
        Span::styled("/", key_hi),
        Span::styled(" Filter", label),
    ];
    // A filter in force is never abbreviated away: panels are hidden, and a
    // monitor that silently stops showing a host is worse than one showing it
    // failing.
    if let FilterHint::Active(query) = filter {
        left_spans.push(Span::styled(
            format!("  [filter: {query}]"),
            Style::default().fg(accent_color),
        ));
    }

    // Kare's ruling for the narrow end: below the width where the words fit,
    // the mode row becomes initials and the accent highlight carries the
    // meaning. `Paragraph` used to guillotine this instead -- at 40 columns the
    // bar read `Upgrad`, a word cut in half, and Filter, Settings, Theme and
    // Sort were simply gone with nothing to say they existed.
    if span_width(&left_spans) > keybar_width as usize {
        let keys = [
            ("Q", key_hi),
            ("S", s_hi),
            ("D", d_hi),
            ("F", f_hi),
            ("U", u_hi),
            ("/", key_hi),
            ("E", key_hi),
        ];
        return keybar_initials(&keys, keybar_width, label, filter, accent_color);
    }
    let left_width = span_width(&left_spans);

    // Shed whole badges rather than letting `Paragraph` slice the last one.
    //
    // The order is Kare's ruling as third expert: Sort goes first, then Theme,
    // and Settings survives longest -- Settings is the door to configuration,
    // while a sort order is recoverable by pressing `c` or `m` and watching what
    // happens. Rams wanted the theme badge deleted outright instead; that is
    // recorded as rejected in the roadmap, because a badge that fits at the
    // width in front of you should be drawn.
    let badges = keybar_badges(sort, theme, label, key_hi, sort_label, accent_color);
    let gap = 2;
    let budget = (keybar_width as usize).saturating_sub(left_width);
    let widths: Vec<usize> = badges.iter().map(|(w, _)| *w).collect();
    // The budget already excludes the left group, so the leading gap between it
    // and the first badge has to come out too.
    let kept = crate::layout::fit_row(&widths, gap, budget.saturating_sub(gap), &[2, 1, 0]);

    let kept_width: usize =
        kept.iter().map(|i| widths[*i]).sum::<usize>() + gap * kept.len().saturating_sub(1);
    let pad = (keybar_width as usize)
        .saturating_sub(left_width + kept_width)
        .min(SPACES.len());

    let mut spans = left_spans;
    spans.push(Span::styled(&SPACES[..pad], label));
    for (n, index) in kept.iter().enumerate() {
        if n > 0 {
            spans.push(Span::styled("  ", label));
        }
        spans.extend(badges[*index].1.clone());
    }
    Line::from(spans)
}
/// What the keybar should say about the current filter.
fn filter_hint(app: &App) -> FilterHint<'_> {
    if app.is_filtering() {
        FilterHint::Editing(&app.filter_query)
    } else if app.filter_query.trim().is_empty() {
        FilterHint::Off
    } else {
        FilterHint::Active(&app.filter_query)
    }
}

/// Say that a filter is hiding everything, rather than showing a blank screen.
///
/// An empty result and a dead app look identical otherwise, and the way out --
/// `Esc` -- is not guessable from a blank terminal.
fn draw_no_matches(f: &mut Frame, app: &App, theme: &multitop_agent::color::Palette) {
    let bg_color = Color::Rgb(
        theme.ratatui_keybar_bg.0,
        theme.ratatui_keybar_bg.1,
        theme.ratatui_keybar_bg.2,
    );
    let area = f.area();
    let (body, keybar) = (
        Rect {
            height: area.height.saturating_sub(1),
            ..area
        },
        Rect {
            y: area.y + area.height.saturating_sub(1),
            height: 1.min(area.height),
            ..area
        },
    );
    let hosts = app.panels.len();
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  No host matches \"{}\".", app.filter_query),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                format!("  {hosts} configured; Esc clears the filter."),
                Style::default().fg(Color::DarkGray),
            )),
        ]),
        body,
    );
    f.render_widget(
        Paragraph::new(keybar_line(
            app.sort,
            theme,
            keybar.width,
            crate::app::Mode::Monitor,
            filter_hint(app),
        ))
        .style(Style::default().bg(bg_color)),
        keybar,
    );
}

/// Whatever modal is on top of everything else, if any.
///
/// Split out because Server Settings needs it too. It used to be reachable only
/// from the main view, so saving a password there had to *close* the panel to
/// show the vault-creation prompt -- the user pressed Enter on a row and landed
/// back on the stats screen.
fn draw_modals(f: &mut Frame, app: &App) {
    use crate::modals::Waiting;
    let waiting = if app.vault_create_in_flight() {
        Some(Waiting::Creating)
    } else if app.vault_verifying() {
        Some(Waiting::Verifying)
    } else if app.vault_awaiting_biometric() {
        Some(Waiting::Biometric)
    } else {
        None
    };
    if let Some(waiting) = waiting {
        crate::modals::draw_vault_awaiting_biometric(f, waiting);
    } else if app.show_vault_password_prompt() || app.vault_creating() {
        crate::modals::draw_vault_password_prompt(f, app);
    } else if app.show_upgrade_modal() {
        crate::modals::draw_upgrade_modal(f, app);
    }
}

#[allow(clippy::too_many_lines)]
pub fn draw(f: &mut Frame, app: &App) {
    if app.password_manager.is_some() {
        crate::config_ui::draw(f, app);
        draw_modals(f, app);
        return;
    }
    // Filtering hides panels rather than dimming them: the point of narrowing to
    // one host is to give that host the whole screen. `shown` maps a layout slot
    // back to the real panel index -- everything downstream (sparklines, the
    // selected panel, task generations) is keyed by the real index, and mixing
    // the two up is how a panel ends up wearing another host's data.
    let shown = app.filtered_indices();
    let theme = app.current_theme();
    if shown.is_empty() {
        draw_no_matches(f, app, theme);
        draw_modals(f, app);
        return;
    }
    let (panel_areas, keybar) = regions(f.area(), shown.len());
    let bg_color = Color::Rgb(
        theme.ratatui_keybar_bg.0,
        theme.ratatui_keybar_bg.1,
        theme.ratatui_keybar_bg.2,
    );

    for (&idx, area) in shown.iter().zip(&panel_areas) {
        let panel = &app.panels[idx];
        let inner = Rect {
            x: area.x + SIDE_MARGIN,
            y: area.y,
            width: area.width.saturating_sub(SIDE_MARGIN * 2),
            height: area.height,
        };
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        let (mut lines, badge_offset) = visible(
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

            // Row 0 is composed here, once, from everything that belongs on it:
            // the banner, the sparklines either side of it, and the scroll
            // badge. `visible` used to write the badge here too and lose.
            let badge = if badge_offset > 0 {
                format!(" [\u{2191} -{badge_offset} lines] ")
            } else {
                String::new()
            };
            let badge_w = badge.chars().count();
            let total_w = (inner.width as usize).saturating_sub(badge_w);
            // Plain ASCII, fitted to the room there is. This was mapped into
            // fullwidth codepoints, which doubled the cell cost and clipped the
            // digits -- the only part that differs between hosts -- off the
            // right, and drew the one label that must never be wrong in a
            // fallback CJK font.
            let server_target = crate::layout::fit_banner(
                &panel.server.user,
                &host_name,
                total_w.saturating_sub(2),
            );
            let disp_w = server_target.chars().count();
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
                let fw = &server_target;

                // A space either side of the name. `space_needed` has always
                // budgeted two and only one was emitted, so the rule was a
                // character longer on the left than the right -- invisible while
                // the name was a wall of fullwidth glyphs, obvious once it is
                // ordinary text.
                lines[0] = format!(
                    "{left_str}{}{}{} {}{}{} {}{}{}{}{right_str}{}",
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
                    badge_span(&badge),
                );
            } else {
                // Not `center_header`: that is the agent's own helper and maps
                // the name into fullwidth glyphs, which is the defect this
                // branch also had.
                lines[0] = format!(
                    "{}{}{}{}{}",
                    theme.primary(),
                    theme.bold,
                    server_target,
                    theme.reset,
                    badge_span(&badge)
                );
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
        Paragraph::new(keybar_line(
            app.sort,
            theme,
            keybar.width,
            active_mode,
            filter_hint(app),
        ))
        .style(Style::default().bg(bg_color)),
        keybar,
    );

    draw_modals(f, app);
}
