use crate::app::App;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

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
pub fn badge_span(badge: &str) -> String {
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
#[allow(clippy::needless_pass_by_value)]
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
    // Shed the views before the doors: Fetch, Graphs, Docker, Upgrade, Stats,
    // then Filter and Settings. Graphs goes early because it is a second
    // reading of the numbers Stats already shows.
    //
    // These are indices into the row above, so the order of the two lists is
    // coupled -- moving a key in the row means moving its index here. The row
    // is Q S D F G U / E.
    let kept = crate::layout::fit_row(&widths, 2, keybar_width as usize, &[3, 4, 2, 5, 1, 6, 7]);
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
    let (g_hi, g_lbl) = pair(crate::app::Mode::Graphs);
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
        Span::styled("G", g_hi),
        Span::styled("raphs", g_lbl),
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
            ("G", g_hi),
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

/// Assemble a keybar row from whole chunks, shedding in a declared order.
///
/// The same rule as every other row: a chunk is drawn whole or not at all, and
/// the shed order is a priority list, never "drop from the right". The way out
/// is never in the shed list.
///
/// Each chunk's width is measured from the spans that will be drawn, never
/// declared alongside them. A hand-written number is a second copy of the
/// string's length that drifts the moment the string is edited -- `[Esc] stay`
/// was declared as 11 cells and is 10 -- and the whole point of the budget is
/// that it is describing what actually goes on screen.
fn chunk_row(
    chunks: &[Vec<Span<'static>>],
    keybar_width: u16,
    shed: &[usize],
    sep_style: Style,
) -> Line<'static> {
    let widths: Vec<usize> = chunks
        .iter()
        .map(|spans| spans.iter().map(Span::width).sum())
        .collect();
    let kept = crate::layout::fit_row(&widths, 2, keybar_width as usize, shed);
    let mut out = Vec::new();
    for (n, index) in kept.iter().enumerate() {
        if n > 0 {
            out.push(Span::styled("  ", sep_style));
        }
        out.extend(chunks[*index].clone());
    }
    Line::from(out)
}

/// The confirmation that replaces the keybar while an upgrade is armed.
///
/// Kare's ruling, review round B: a keybar row rather than a box -- the box
/// was 38 cells wide at 40 columns and clipped its own cancel line to `Esc t`,
/// while the filter prompt renders every word whole at the same size. State
/// left, keys right, two spaces between. Shed order: the `· M skipped` tail
/// first (the ⚠ is already in those panes), then the count itself before the
/// keys; `[Esc] cancel` is last and in practice never -- it is the only thing
/// on the line the operator cannot guess.
///
/// The count is the alarm: with the run scoped to the filter, a grid showing
/// one host says "Upgrade 1 host", never a sentence long enough to hide the
/// others.
///
/// # The interrupted-run warning
///
/// The box this replaced also said "Previous upgrade was interrupted! Check
/// server state." when a run started and no completion followed. Rams
/// condemned the box's aggregate `Last update` *timestamp* and the ruling
/// dropped it; the warning is a different thing and dropping it with the box
/// was an accident. It is back, and it sheds **after** the count: how many
/// machines are about to be touched is a number the operator can recover by
/// looking at the grid, whereas "one of these has a half-finished dpkg
/// transaction on it" appears nowhere else on the screen.
fn upgrade_confirm_row(
    app: &App,
    accent: Color,
    key_hi: Style,
    label: Style,
    keybar_width: u16,
) -> Line<'static> {
    let scope = app.filtered_indices();
    let skipped = app.upgrade_skip_hosts();
    let runnable = scope.len().saturating_sub(skipped.len());

    // The count is styled as the alarm: a grid showing one host that says
    // "Upgrade 8 hosts" must be louder than any sentence that would fit.
    let count = format!(
        "Upgrade {runnable} host{}",
        if runnable == 1 { "" } else { "s" }
    );
    let mut chunks: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
        count,
        Style::default()
            .fg(accent)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )]];
    let count_at = 0;
    // Shed order is built by identity, not by position: which index holds what
    // depends on whether the optional chunks are present at all.
    let mut skipped_at = None;
    let mut interrupted_at = None;
    if !skipped.is_empty() {
        skipped_at = Some(chunks.len());
        chunks.push(vec![Span::styled(
            format!("\u{b7} {} skipped", skipped.len()),
            label,
        )]);
    }
    if app.previous_upgrade_interrupted() {
        interrupted_at = Some(chunks.len());
        chunks.push(vec![Span::styled(
            "\u{26a0} previous run interrupted",
            Style::default().fg(Color::Yellow),
        )]);
    }
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("U", key_hi),
        Span::styled("] go", label),
    ]);
    // Neither key ever sheds. Together they are 20 cells; a terminal too narrow
    // for that is too narrow for the grid underneath, and a confirmation with
    // no stated way out is the defect this row exists to remove.
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Esc", key_hi),
        Span::styled("] cancel", label),
    ]);
    let shed: Vec<usize> = skipped_at
        .into_iter()
        .chain(std::iter::once(count_at))
        .chain(interrupted_at)
        .collect();
    chunk_row(&chunks, keybar_width, &shed, label)
}

/// The confirmation that replaces the keybar once Esc/q/Ctrl-C asked to quit
/// while upgrades were in flight. Names the hosts, states the cost, and gives
/// the two ways out.
fn quit_confirm_row(app: &App, key_hi: Style, label: Style, keybar_width: u16) -> Line<'static> {
    let hosts = app.running_upgrade_hosts();
    let n = hosts.len();
    let mut chunks: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
        format!("{n} upgrade{} running", if n == 1 { "" } else { "s" }),
        Style::default().fg(Color::Yellow),
    )]];
    // The host list is the first thing to go: it is long, and every one of
    // those names is already on the grid behind this row. The count is not --
    // it is what says the quit has a cost at all.
    let mut shed = Vec::new();
    let host_list = hosts.join(", ");
    if !host_list.is_empty() {
        shed.push(chunks.len());
        chunks.push(vec![Span::styled(format!("\u{b7} {host_list}"), label)]);
    }
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Q", key_hi),
        Span::styled("] quit anyway", label),
    ]);
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Esc", key_hi),
        Span::styled("] stay", label),
    ]);
    chunk_row(&chunks, keybar_width, &shed, label)
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
    if app.help_visible {
        return Line::from(vec![
            Span::styled("[", label),
            Span::styled("Esc", key_hi),
            Span::styled("] close help  ", label),
            Span::styled("?", key_hi),
            Span::styled(" toggle  ", label),
            Span::styled("[q] quit", label),
        ]);
    }
    if app.command_palette_visible {
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
