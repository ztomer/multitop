use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::fmt::unixtime_to_str;

pub fn draw_upgrade_modal(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));
    let popup_height = (10u16).min(area.height.saturating_sub(2));

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

    let block = ratatui::widgets::Block::default()
        .title(" Confirm System Update ")
        .title_style(
            Style::default()
                .fg(accent_color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(border_color));

    f.render_widget(ratatui::widgets::Clear, popup_rect);

    let last_up = app.last_update.map(unixtime_to_str).unwrap_or_else(|| "Never".to_string());
    let interrupted = app.upgrade_started_at.is_some_and(|started| app.last_update.is_none_or(|lu| started > lu));

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Updates can be slow and potentially destructive.",
            Style::default().fg(Color::Yellow),
        )]),
    ];
    if interrupted {
        lines.push(Line::from(vec![Span::styled(
            "  Previous upgrade was interrupted! Check server state.",
            Style::default().fg(Color::Red),
        )]));
    }
    lines.push(Line::from(vec![Span::styled(
        "  Are you sure you want to run updates on all servers?",
        Style::default().fg(Color::White),
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Last update: ", Style::default().fg(Color::DarkGray)),
        Span::styled(last_up, Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "U",
            Style::default()
                .fg(Color::White)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(Color::White)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" to confirm, ", Style::default().fg(Color::DarkGray)),
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

pub fn draw_vault_password_prompt(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));
    let popup_height = (10u16).min(area.height.saturating_sub(2));

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

    let block = ratatui::widgets::Block::default()
        .title(" Vault Password ")
        .title_style(
            Style::default()
                .fg(accent_color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(border_color));

    f.render_widget(ratatui::widgets::Clear, popup_rect);

    let password_dots: String = (0..app.vault_password_input.len()).map(|_| '*').collect();
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Enter vault master password to unlock:",
            Style::default().fg(Color::White),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  > ", Style::default().fg(Color::Cyan)),
            Span::styled(password_dots, Style::default().fg(Color::White)),
        ]),
    ];
    if let Some(ref error) = app.vault_password_error {
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
        Span::styled(" to unlock, ", Style::default().fg(Color::DarkGray)),
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
