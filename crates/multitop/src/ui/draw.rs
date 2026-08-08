use crate::ansi;
use crate::app::App;
use crate::ui::keybar::{badge_span, keybar_content};
use crate::ui::layout::{regions, SIDE_MARGIN};
use crate::ui::windowing::pane_lines;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

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
        Paragraph::new(keybar_content(
            app,
            theme,
            keybar.width,
            crate::app::Mode::Monitor,
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
    }
}

#[allow(clippy::too_many_lines)]
/// Draw one frame.
///
/// Takes `&mut App` for one reason: the scroll offset is bounded here and
/// nowhere else. `App::scroll_up` cannot know a pane's height *or* its composed
/// length, and its own clamp -- `pane_len - 1` -- was wrong in both directions:
/// looser than the real limit by the height of the pane, so scrolling to the top
/// stored an offset far past anything the view could use and the next dozen
/// presses the other way moved nothing; and tighter than it by every wrapped
/// notice, so `Home` stopped short of the very lines a notice is written to
/// deliver. That clamp is gone. The effective offset computed for the frame is
/// written back, so what is stored is always what is shown.
pub fn draw(f: &mut Frame, app: &mut App) {
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

    // Collected rather than written in the loop, which holds `app` immutably.
    let mut effective_offsets: Vec<(usize, usize)> = Vec::with_capacity(shown.len());
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
        let (mut lines, badge_offset) = pane_lines(
            app,
            idx,
            inner.height as usize,
            inner.width as usize,
            panel.scroll_offset,
        );
        effective_offsets.push((idx, badge_offset));
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
            // the banner and the scroll badge. `visible` used to write the badge
            // here too and lose.
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
            // The fitter reports the cells it drew. Measuring the returned
            // string here would be a second width calculation beside the one
            // that produced it, and the two disagree by a factor of two the
            // moment the style changes -- which is how the rule either side of
            // the name ends up computed against a width the name does not have.
            let (server_target, disp_w) = crate::layout::fit_banner_styled(
                &panel.server.user,
                &host_name,
                total_w.saturating_sub(2),
                app.banner_style,
            );
            let space_needed = disp_w + 2;

            if total_w >= space_needed {
                let rem = total_w - space_needed;
                let left_rule_len = rem / 2;
                let right_rule_len = rem - left_rule_len;

                let fw = &server_target;

                // A space either side of the name. `space_needed` has always
                // budgeted two and only one was emitted, so the rule was a
                // character longer on the left than the right -- invisible while
                // the name was a wall of fullwidth glyphs, obvious once it is
                // ordinary text.
                lines[0] = format!(
                    "{}{}{} {}{}{}{} {}{}{}{}",
                    theme.secondary(),
                    "\u{2500}".repeat(left_rule_len),
                    theme.reset,
                    theme.primary(),
                    theme.bold,
                    fw,
                    theme.reset,
                    theme.secondary(),
                    "\u{2500}".repeat(right_rule_len),
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
    // What is stored becomes what was shown. Without this the offset kept
    // whatever `App::scroll_up` allowed, which is the pane's height further
    // back than the view can go, and every press in the other direction was
    // spent walking back through that gap with nothing moving on screen.
    for (idx, offset) in effective_offsets {
        if let Some(p) = app.panels.get_mut(idx) {
            p.scroll_offset = offset;
        }
    }
    let active_mode = app
        .panels
        .get(app.selected_panel)
        .or_else(|| app.panels.first())
        .map_or(crate::app::Mode::Monitor, |p| p.mode);

    f.render_widget(
        Paragraph::new(keybar_content(app, theme, keybar.width, active_mode))
            .style(Style::default().bg(bg_color)),
        keybar,
    );

    draw_modals(f, app);
}
