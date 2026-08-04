use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
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
    const fn title(self) -> &'static str {
        match self {
            Self::Biometric => " Vault Locked ",
            Self::Verifying => " Unlocking Vault ",
            Self::Creating => " Creating Vault ",
        }
    }

    const fn headline(self) -> &'static str {
        match self {
            Self::Biometric => "  Unlocking with Touch ID / fingerprint\u{2026}",
            Self::Verifying => "  Checking the master password\u{2026}",
            Self::Creating => "  Encrypting the new vault\u{2026}",
        }
    }

    const fn hint(self) -> &'static str {
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

pub fn draw_vault_password_prompt(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));
    // Creating explains what the password is for and carries three more lines
    // for it. Sized to the content rather than left at the unlocking height,
    // which clipped the "Press Enter to create" footer off the bottom.
    let wanted = if app.vault_creating() { 12 } else { 10 };
    let popup_height = wanted.min(area.height.saturating_sub(2));

    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_rect = Rect::new(x, y, popup_width, popup_height);

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
        .border_style(Style::default().fg(border_color));

    f.render_widget(ratatui::widgets::Clear, popup_rect);

    let password_dots = crate::fmt::mask_secret(app.vault_password_input());
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            if creating {
                "  Choose a master password for your new vault:"
            } else {
                "  Enter vault master password to unlock:"
            },
            Style::default().fg(Color::White),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  > ", Style::default().fg(Color::Cyan)),
            Span::styled(password_dots, Style::default().fg(Color::White)),
        ]),
    ];
    if creating {
        // Kept inside the box: it is 64 columns wide at most, so 62 is the
        // budget and the first of these used to be 68 -- the sentence about
        // what the password is *for* lost its verb to the border.
        for row in [
            "  Sudo passwords are encrypted with it.",
            "  Touch ID unlocks it day to day; this password is the",
            "  recovery path if that key is ever lost.",
        ] {
            lines.push(Line::from(vec![Span::styled(
                row,
                Style::default().fg(Color::DarkGray),
            )]));
        }
    }
    if let Some(error) = app
        .vault_create_error()
        .or_else(|| app.vault_password_error())
    {
        lines.push(Line::from(vec![Span::styled(
            format!("  {error}"),
            Style::default().fg(Color::Red),
        )]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
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

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup_rect);
}
