//! Rendering for the full-screen configuration panel.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::passwords::ConfigSection;

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
    let mut lines =
        vec![Line::from(Span::styled(
        "[Tab] Sudo Passwords / Servers — passwords are saved securely or kept for session.",
        Style::default().fg(Color::DarkGray),
    )), Line::from("")];
    if manager.section == ConfigSection::Servers {
        lines.push(Line::from(Span::styled(
            "  Server                          User              Port  Upgrade command",
            Style::default().fg(Color::DarkGray),
        )));
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
            lines.push(Line::from(Span::styled(
                format!(
                    "{marker} {:<31} {:<17} {:<5} {command}",
                    panel.server.host, user, panel.server.port
                ),
                style,
            )));
        }
        lines.push(Line::from(""));
        if let Some(draft) = &manager.draft {
            lines.push(Line::from(Span::styled(
                "Editing server",
                Style::default().fg(accent),
            )));
            for (index, (label, value)) in [
                ("Host", &draft.host),
                ("User", &draft.user),
                ("Port", &draft.port),
                ("Upgrade command", &draft.upgrade_cmd),
            ]
            .iter()
            .enumerate()
            {
                lines.push(Line::from(format!(
                    "{} {label}: {value}",
                    if index == draft.field { ">" } else { " " }
                )));
            }
            lines.push(Line::from(Span::styled(
                "[Tab/Up/Down] Field  [Enter] Save  [Esc] Cancel",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "[A] Add  [Enter/E] Edit  [D] Delete  [Esc/E] Return",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  Server                         User              Password",
            Style::default().fg(Color::DarkGray),
        )));
        for (index, panel) in app.panels.iter().enumerate() {
            let marker = if index == manager.selected { ">" } else { " " };
            let user = if panel.server.user.is_empty() {
                "default"
            } else {
                &panel.server.user
            };
            let state = if panel.password_saved {
                "Stored securely"
            } else if panel.sudo_password.is_some() {
                "Session only"
            } else {
                "Not set"
            };
            let style = if index == manager.selected {
                Style::default().fg(accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "{marker} {:<30} {:<17} {state}",
                    format!("{}:{}", panel.server.host, panel.server.port),
                    user
                ),
                style,
            )));
        }
        lines.push(Line::from(""));
        if manager.editing {
            let server = &app.panels[manager.selected].server;
            let persistence = if manager.store_on_save {
                "yes (system credential store)"
            } else {
                "no (this session only)"
            };
            lines.push(Line::from(format!(
                "Editing {}@{}",
                server.user, server.host
            )));
            lines.push(Line::from(vec![
                Span::raw("Password: "),
                Span::styled(
                    "*".repeat(manager.input.chars().count()),
                    Style::default().fg(accent),
                ),
            ]));
            lines.push(Line::from(format!("Store on save: {persistence}")));
            lines.push(Line::from(Span::styled(
                "[Enter] Save  [Esc] Cancel",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "[A] Add Server  [Enter] Edit Password  [S] Toggle secure storage  [D] Delete Server  [Esc/E] Return",
                Style::default().fg(Color::DarkGray),
            )));
        }
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
