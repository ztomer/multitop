//! Rendering for the full-screen configuration panel.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

#[allow(
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::expect_used
)]
pub fn draw(f: &mut Frame, app: &App) {
    let manager = app
        .password_manager
        .as_ref()
        .expect("configuration is open");
    let theme = app.current_theme();
    let accent = Color::Rgb(
        theme.ratatui_accent.0,
        theme.ratatui_accent.1,
        theme.ratatui_accent.2,
    );
    let border = Color::Rgb(
        theme.ratatui_border.0,
        theme.ratatui_border.1,
        theme.ratatui_border.2,
    );
    let bg = Color::Rgb(
        theme.ratatui_keybar_bg.0,
        theme.ratatui_keybar_bg.1,
        theme.ratatui_keybar_bg.2,
    );
    let mut lines = vec![Line::from(Span::styled(
        "  Server                    User          Port  Upgrade command            Password",
        Style::default().fg(Color::DarkGray),
    ))];
    for (index, panel) in app.panels.iter().enumerate() {
        let marker = if index == manager.selected { ">" } else { " " };
        let user = if panel.server.user.is_empty() {
            "default"
        } else {
            &panel.server.user
        };
        let command = panel.server.upgrade_cmd.as_deref().unwrap_or("-");
        let style = if index == manager.selected {
            Style::default().fg(accent)
        } else {
            Style::default()
        };
        // Kare set only. These were a padlock and a white circle written as
        // unicode escapes, which is how they sat in a repo with a no-emoji
        // gate: the escape is plain ASCII in the source, so a character scan
        // saw nothing while the UI drew emoji.
        //
        // Two states, and they mean one thing each: this host has its own
        // sudo password, or it does not. There used to be a third, for a host
        // borrowing a shared password -- that shared password was a conflation
        // of the vault master password with a per-host sudo password, and it is
        // gone.
        let (state, state_color) = if panel.password_saved || panel.sudo_password.is_some() {
            ("\u{2713} Stored", Color::Green)
        } else {
            ("\u{b7} Unset", Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{marker} {:<25} {:<13} {:<5} {:<26} ",
                    panel.server.host, user, panel.server.port, command
                ),
                style,
            ),
            Span::styled(state, Style::default().fg(state_color)),
        ]));
    }
    lines.push(Line::from(""));

    if let Some(draft) = &manager.draft {
        lines.push(Line::from(Span::styled(
            "Editing server",
            Style::default().fg(accent),
        )));
        let masked_pass = crate::fmt::mask_secret(&draft.password);
        for (index, (label, value)) in [
            ("Host", &draft.host),
            ("User", &draft.user),
            ("Port", &draft.port),
            ("Upgrade command", &draft.upgrade_cmd),
            ("Password", &masked_pass),
        ]
        .iter()
        .enumerate()
        {
            lines.push(Line::from(format!(
                "{} {label:<16}: {value}",
                if index == draft.field { ">" } else { " " }
            )));
        }
        lines.push(Line::from(Span::styled(
            "  Leave Password empty to remove this host's own password.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "[Tab/Up/Down] Field  [Enter] Save  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )));
    } else if manager.editing() {
        lines.push(Line::from("Changing the vault master password".to_string()));
        lines.push(Line::from(vec![
            Span::raw("Password: "),
            Span::styled(
                crate::fmt::mask_secret(&manager.input),
                Style::default().fg(accent),
            ),
        ]));
        // A rotation does not touch the OS credential store, so saying it does
        // would be a plain lie about where the secret goes.
        lines.push(Line::from(Span::styled(
            "[Enter] Continue  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "[Enter] Edit  [A] Add  [D] Delete  [I] Import ~/.ssh/config  [R] Change vault master password  [Esc/E] Return",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Experimental",
            Style::default().fg(accent),
        )));
        let spark_on = app.show_sparklines();
        lines.push(Line::from(vec![
            Span::raw("  Sparklines  "),
            Span::styled(
                if spark_on { "[On]" } else { "[Off]" },
                Style::default().fg(if spark_on {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            "  [S] Toggle sparklines",
            Style::default().fg(Color::DarkGray),
        )));
    }
    if let Some(notice) = &manager.notice {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            notice.clone(),
            Style::default().fg(Color::Yellow),
        )));
    }
    let block = Block::default()
        .title(" Server Settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(bg));
    f.render_widget(Paragraph::new(lines).block(block), f.area());
}
