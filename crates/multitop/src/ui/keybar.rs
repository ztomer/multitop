use crate::app::App;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::keybar_confirm::{kill_confirm_row, quit_confirm_row, upgrade_confirm_row};

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
#[must_use]
pub fn mode_pair(
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
#[must_use]
pub fn keybar_badges(
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
        vec![
            Span::styled("[", sort_label),
            Span::styled("H", key_hi),
            Span::styled("] Alerts", label),
        ],
        vec![
            Span::styled("[", sort_label),
            Span::styled("y", key_hi),
            Span::styled("] Yank", label),
        ],
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

pub fn badge_span(badge: &str) -> String {
    if badge.is_empty() {
        String::new()
    } else {
        format!("\x1b[33;1m{badge}\x1b[0m")
    }
}

pub(super) fn span_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// The narrow keybar's shed order, views before doors, by NAME: an index
/// list coupled to the row's order broke the day Ops was added.
const SHED: [&str; 9] = ["F", "G", "D", "P", "U", "S", "/", "E", "?"];

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
    let shed: Vec<usize> = SHED
        .iter()
        .filter_map(|k| keys.iter().position(|(t, _)| t == k))
        .collect();
    let kept = crate::layout::fit_row(&widths, 2, keybar_width as usize, &shed);
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
    let upgrade_word = if active_mode == crate::app::Mode::Upgrade {
        // In the Upgrade view the same key starts the run, so say which of the
        // two it will do rather than leaving the second press undiscoverable.
        "pgrade: run"
    } else {
        "pgrade"
    };
    // The view keys, left to right, from ONE table - the words and the
    // initials fallback below read the same rows, so a view cannot be in one
    // and missing from the other: the letters before the key, the key, the
    // rest of the word, the key's initial, and the view it selects.
    let views = [
        ("", "S", "tats", "S", crate::app::Mode::Monitor),
        ("", "D", "ocker", "D", crate::app::Mode::Docker),
        ("", "F", "etch", "F", crate::app::Mode::Fetch),
        ("", "G", "raphs", "G", crate::app::Mode::Graphs),
        ("O", "p", "s", "P", crate::app::Mode::Ops),
        ("", "U", upgrade_word, "U", crate::app::Mode::Upgrade),
    ];
    let mut left_spans = vec![
        Span::styled("ESC / ", label),
        Span::styled("Q", key_hi),
        Span::styled("uit", label),
    ];
    for (before, key, rest, _, m) in views {
        let (hi, lbl) = pair(m);
        left_spans.push(Span::styled("  ", label));
        if !before.is_empty() {
            left_spans.push(Span::styled(before, lbl));
        }
        left_spans.extend([Span::styled(key, hi), Span::styled(rest, lbl)]);
    }
    left_spans.extend([
        Span::styled("  ", label),
        Span::styled("/", key_hi),
        Span::styled(" Filter", label),
        Span::styled("  ", label),
        Span::styled("?", key_hi),
        Span::styled(" Help", label),
    ]);
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
        let mut keys = vec![("Q", key_hi)];
        keys.extend(
            views
                .iter()
                .map(|&(_, _, _, initial, m)| (initial, pair(m).0)),
        );
        keys.extend([("/", key_hi), ("E", key_hi), ("?", key_hi)]);
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
    let kept = crate::layout::fit_row(&widths, gap, budget.saturating_sub(gap), &[1, 0, 4, 3, 2]);

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

/// What the keybar row should be right now: a confirm row for a quit or an
/// upgrade when one is armed, the filter prompt while typing, and the ordinary
/// keybar otherwise.
#[must_use]
pub fn keybar_content(
    app: &App,
    theme: &multitop_agent::color::Palette,
    keybar_width: u16,
    active_mode: crate::app::Mode,
) -> Line<'static> {
    let label = Style::default().fg(Color::DarkGray);
    let accent_color = Color::Rgb(
        theme.ratatui_accent.0,
        theme.ratatui_accent.1,
        theme.ratatui_accent.2,
    );
    let key_hi = Style::default()
        .fg(Color::White)
        .add_modifier(ratatui::style::Modifier::BOLD);
    if app.overlay.is_help() {
        return Line::from(vec![
            Span::styled("[", label),
            Span::styled("Esc", key_hi),
            Span::styled("] close help  ", label),
            Span::styled("?", key_hi),
            Span::styled(" toggle  ", label),
            Span::styled("[q] quit", label),
        ]);
    }
    if app.overlay.is_palette() {
        return Line::from(vec![
            Span::styled("[", label),
            Span::styled("Esc", key_hi),
            Span::styled("] close  ", label),
            Span::styled("Enter", key_hi),
            Span::styled(" run", label),
        ]);
    }
    if app.is_focused() {
        return Line::from(vec![
            Span::styled("[", label),
            Span::styled("Esc", key_hi),
            Span::styled("] unzoom  ", label),
            Span::styled("z", key_hi),
            Span::styled(" focus", label),
        ]);
    }
    // The same answer `run::handle_key` acts on, so the row cannot name one set
    // of keys while another set is live.
    match app.active_confirm() {
        Some(crate::app::Confirm::Quit) => quit_confirm_row(app, key_hi, label, keybar_width),
        Some(crate::app::Confirm::Kill) => {
            kill_confirm_row(app, accent_color, key_hi, label, keybar_width)
        }
        Some(crate::app::Confirm::Upgrade) => {
            upgrade_confirm_row(app, accent_color, key_hi, label, keybar_width)
        }
        None => keybar_line(app.sort, theme, keybar_width, active_mode, filter_hint(app)),
    }
}
/// What the keybar should say about the current filter.
pub fn filter_hint(app: &App) -> FilterHint<'_> {
    if app.is_filtering() {
        FilterHint::Editing(&app.filter_query)
    } else if app.filter_query.trim().is_empty() {
        FilterHint::Off
    } else {
        FilterHint::Active(&app.filter_query)
    }
}
