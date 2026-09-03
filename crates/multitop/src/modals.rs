use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;

/// Which slow vault operation the "please wait" modal is reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// A Touch ID / fingerprint prompt is outstanding.
    Biometric,
    /// Argon2id is checking a typed master password.
    Verifying,
    /// Argon2id is deriving the key for a brand new vault.
    Creating,
}

impl Waiting {
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Biometric => " Vault Locked ",
            Self::Verifying => " Unlocking Vault ",
            Self::Creating => " Creating Vault ",
        }
    }

    pub(crate) const fn headline(self) -> &'static str {
        match self {
            Self::Biometric => "  Unlocking with Touch ID / fingerprint\u{2026}",
            Self::Verifying => "  Checking the master password\u{2026}",
            Self::Creating => "  Encrypting the new vault\u{2026}",
        }
    }

    pub(crate) const fn hint(self) -> &'static str {
        match self {
            Self::Biometric => "  Esc to cancel and use the vault password instead.",
            Self::Verifying => "  This takes a moment by design. Esc to cancel.",
            // Says not to retype, because retyping is exactly what the empty
            // field invited before this modal existed -- and every extra Enter
            // was another vault initialisation.
            Self::Creating => "  This takes a moment by design \u{2014} no need to type it again.",
        }
    }
}

/// The "please wait" modal, shown while a slow vault operation is outstanding.
pub fn draw_vault_awaiting_biometric(f: &mut Frame, waiting: Waiting) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));
    let popup_height = (8u16).min(area.height.saturating_sub(2));

    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_rect = Rect::new(x, y, popup_width, popup_height);

    let theme = &multitop_agent::color::THEMES[0];
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

    let block = ratatui::widgets::Block::default()
        .title(waiting.title())
        .title_style(
            Style::default()
                .fg(accent_color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(border_color));

    f.render_widget(ratatui::widgets::Clear, popup_rect);

    let lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            waiting.headline(),
            Style::default().fg(Color::White),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            waiting.hint(),
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup_rect);
}

/// Horizontal padding inside a modal's border, in cells.
///
/// Named because the height calculation must subtract exactly what the block
/// adds, and because the indent belongs to the block rather than to each line:
/// carried per line, a wrapped continuation starts at the border.
const PAD: u16 = 2;

/// Keys table from `README.md:82`, rendered as the `?` overlay.
///
/// The table is the one place the keys are named in English rather than as
/// `tui/lib.sh` `→` glyphs, so a key that does nothing is visible immediately.
/// Drawn with `Rams` dim + `NO_COLOR` aware via `Palette`.
pub fn draw_command_palette(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (72u16).min(area.width.saturating_sub(4));
    let popup_height = (12u16).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let rect = Rect::new(x, y, popup_width, popup_height);

    let theme = &multitop_agent::color::THEMES[0];
    let border = Color::Rgb(
        theme.ratatui_border.0,
        theme.ratatui_border.1,
        theme.ratatui_border.2,
    );
    let accent = Color::Rgb(
        theme.ratatui_accent.0,
        theme.ratatui_accent.1,
        theme.ratatui_accent.2,
    );

    let block = ratatui::widgets::Block::default()
        .title(" Command Palette  : to open ")
        .title_style(
            Style::default()
                .fg(accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .borders(ratatui::widgets::Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(PAD))
        .border_style(Style::default().fg(border));

    let input = app.command_input.as_str();
    let filter = input.to_lowercase();
    let all = [
        "filter <query>",
        "clear filter",
        "upgrade",
        "docker",
        "fetch",
        "graphs",
        "stats",
        "sort cpu",
        "sort mem",
        "theme",
        "add server",
        "vault unlock",
        "yank",
    ];
    let shown: Vec<&str> = all
        .iter()
        .copied()
        .filter(|c| filter.is_empty() || c.contains(&filter))
        .take(8)
        .collect();

    let mut rows: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(": ", Style::default().fg(Color::Yellow)),
            Span::styled(input.to_string(), Style::default().fg(Color::White)),
            Span::styled("\u{2588}", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
    ];
    for cmd in &shown {
        rows.push(Line::from(vec![Span::styled(
            format!("  {cmd}"),
            Style::default().fg(Color::DarkGray),
        )]));
    }
    if shown.is_empty() {
        rows.push(Line::from(vec![Span::styled(
            "  No matches",
            Style::default().fg(Color::DarkGray),
        )]));
    }
    rows.push(Line::from(""));
    rows.push(Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::White)),
        Span::styled(" run  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::White)),
        Span::styled(" close", Style::default().fg(Color::DarkGray)),
    ]));

    f.render_widget(ratatui::widgets::Clear, rect);
    let para = Paragraph::new(rows).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

pub fn draw_help(f: &mut Frame) {
    let area = f.area();
    let popup_width = (72u16).min(area.width.saturating_sub(4));
    let popup_height = (20u16).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let rect = Rect::new(x, y, popup_width, popup_height);

    let theme = &multitop_agent::color::THEMES[0];
    let border = Color::Rgb(
        theme.ratatui_border.0,
        theme.ratatui_border.1,
        theme.ratatui_border.2,
    );
    let accent = Color::Rgb(
        theme.ratatui_accent.0,
        theme.ratatui_accent.1,
        theme.ratatui_accent.2,
    );

    let block = ratatui::widgets::Block::default()
        .title(" Help  ? to close ")
        .title_style(
            Style::default()
                .fg(accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .borders(ratatui::widgets::Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(PAD))
        .border_style(Style::default().fg(border));

    let rows: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Esc/q", Style::default().fg(Color::Yellow)),
            Span::raw(" quit  "),
            Span::styled("c", Style::default().fg(Color::Yellow)),
            Span::raw("/"),
            Span::styled("m", Style::default().fg(Color::Yellow)),
            Span::raw(" sort  "),
            Span::styled("d", Style::default().fg(Color::Yellow)),
            Span::raw(" docker  "),
            Span::styled("s", Style::default().fg(Color::Yellow)),
            Span::raw(" stats"),
        ]),
        Line::from(vec![
            Span::styled("G", Style::default().fg(Color::Yellow)),
            Span::raw(" graphs  "),
            Span::styled("u", Style::default().fg(Color::Yellow)),
            Span::raw(" update  "),
            Span::styled("f", Style::default().fg(Color::Yellow)),
            Span::raw(" fetch  "),
            Span::styled("e", Style::default().fg(Color::Yellow)),
            Span::raw(" settings"),
        ]),
        Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(" filter  "),
            Span::styled("1-9", Style::default().fg(Color::Yellow)),
            Span::raw(" select  "),
            Span::styled("t", Style::default().fg(Color::Yellow)),
            Span::raw(" theme  "),
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" help"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Upgrade is two presses: u shows what would run, u again confirms.",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(vec![Span::styled(
            "Filter searches host/user + whatever the view shows (process, container, OS).",
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    f.render_widget(ratatui::widgets::Clear, rect);
    let para = Paragraph::new(rows).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

/// The prompt for unlocking an existing vault, or choosing a new master
/// password.
///
/// # Why the content is assembled before the box is sized
///
/// The height was a literal -- 12 when creating, 10 when unlocking -- and the
/// comment beside it recorded that it had *already* been bumped once because it
/// "clipped the `Press Enter to create` footer off the bottom". A guessed line
/// count is only ever right at the width it was guessed for. At 40 columns the
/// box is 36 cells wide and every line of prose in it was cut mid-word:
///
/// ```text
/// |  Enter vault master password to unl|
/// |  Press Enter to unlock, Esc to canc|
/// ```
///
/// A modal that amputates its own way out is the defect Kare's ruling removed
/// from the upgrade confirmation by making it a keybar row. A password prompt
/// cannot become a keybar row -- it needs a field -- so it does the other half
/// of that ruling instead: **it sheds.**
///
/// # The shed order
///
/// Wrapping alone is not enough. A twelve-row terminal leaves eight rows inside
/// the borders, and the create prompt's full content needs nine. So the parts
/// are ranked, exactly as `fit_row` ranks a keybar's chunks:
///
/// 1. the explanation of what the password is for -- useful, never essential
/// 2. the blank lines that space the block out -- decoration
///
/// The headline, the field and the footer naming both keys are never shed. An
/// operator who cannot read what the password protects can still act; one who
/// cannot see `Esc` is stuck.
#[allow(clippy::too_many_lines)]
pub fn draw_vault_password_prompt(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));

    let theme = app.current_theme();
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

    // The same prompt serves unlocking an existing vault and choosing the
    // master password for a new one; only the wording differs.
    let creating = app.vault_creating();
    let block = ratatui::widgets::Block::default()
        .title(if creating {
            " Create Vault "
        } else {
            " Vault Password "
        })
        .title_style(
            Style::default()
                .fg(accent_color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .borders(ratatui::widgets::Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(PAD))
        .border_style(Style::default().fg(border_color));

    let password_dots = crate::fmt::mask_secret(app.vault_password_input());
    let error: Option<String> = app
        .vault_create_error()
        .or_else(|| app.vault_password_error())
        .map(ToString::to_string);

    let compose = |explain: bool, blanks: bool| -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        if blanks {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            if creating {
                "Choose a master password for your new vault:"
            } else {
                "Enter vault master password to unlock:"
            },
            Style::default().fg(Color::White),
        )]));
        if blanks {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::styled(password_dots.clone(), Style::default().fg(Color::White)),
        ]));
        if creating && explain {
            lines.push(Line::from(vec![Span::styled(
                "Encrypts sudo passwords. Touch ID unlocks it day to day; \
                 this password is the recovery path if that key is lost.",
                Style::default().fg(Color::DarkGray),
            )]));
        }
        if let Some(error) = &error {
            lines.push(Line::from(vec![Span::styled(
                error.clone(),
                Style::default().fg(Color::Red),
            )]));
        }
        if blanks {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                if creating {
                    " to create, "
                } else {
                    " to unlock, "
                },
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(" to cancel", Style::default().fg(Color::DarkGray)),
        ]));
        lines
    };

    // The rows the box can hold inside its border, at the width it will be
    // drawn at.
    let inner_w = popup_width.saturating_sub(2 + PAD * 2) as usize;
    let budget = usize::from(area.height.saturating_sub(4));
    let wrapped_len = |lines: &[Line<'static>]| -> usize {
        lines
            .iter()
            .map(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                crate::layout::wrap_words(&text, inner_w).len().max(1)
            })
            .sum()
    };

    // First arrangement that fits, in shed order. The last is the floor: when
    // even that overflows the terminal is too short for a prompt at all, and
    // the box is clamped to the screen rather than the content.
    let mut lines = compose(false, false);
    for candidate in [compose(true, true), compose(false, true)] {
        if wrapped_len(&candidate) <= budget {
            lines = candidate;
            break;
        }
    }
    let needed = wrapped_len(&lines);

    let popup_height = u16::try_from(needed + 2)
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_rect = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(ratatui::widgets::Clear, popup_rect);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, popup_rect);
}
