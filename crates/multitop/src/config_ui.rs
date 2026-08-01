//! Rendering for the full-screen configuration panel.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::passwords::ConfigSection;

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
                "Editing Server Configuration & Password",
                Style::default().fg(accent),
            )));
            let masked_pass = "*".repeat(draft.password.chars().count());
            for (index, (label, value)) in [
                ("Host", &draft.host),
                ("User", &draft.user),
                ("Port", &draft.port),
                ("Upgrade command", &draft.upgrade_cmd),
                ("Sudo password", &masked_pass),
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
                "[Tab/Up/Down] Field  [Enter] Save All Settings  [Esc] Cancel",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "[A] Add Server  [Enter/E] Edit Server & Password  [D] Delete Server  [Esc/E] Return",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  Server                         User              Password Status",
            Style::default().fg(Color::DarkGray),
        )));
        for (index, panel) in app.panels.iter().enumerate() {
            let marker = if index == manager.selected { ">" } else { " " };
            let user = if panel.server.user.is_empty() {
                "default"
            } else {
                &panel.server.user
            };
            let (state, state_color) = if panel.password_saved || panel.sudo_password.is_some() {
                ("\u{1f512} Stored", Color::Green)
            } else {
                ("\u{26aa} Unset", Color::DarkGray)
            };
            let style = if index == manager.selected {
                Style::default().fg(accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "{marker} {:<30} {:<17} ",
                        format!("{}:{}", panel.server.host, panel.server.port),
                        user
                    ),
                    style,
                ),
                Span::styled(state, Style::default().fg(state_color)),
            ]));
        }
        lines.push(Line::from(""));
        let spark_status = if app.show_sparklines() {
            "Enabled"
        } else {
            "Disabled"
        };
        let spark_color = if app.show_sparklines() {
            Color::Green
        } else {
            Color::DarkGray
        };
        lines.push(Line::from(vec![
            Span::raw("Sparklines (Experimental): "),
            Span::styled(spark_status, Style::default().fg(spark_color)),
        ]));
        lines.push(Line::from(""));
        if manager.editing {
            let target_desc = if manager.is_sso {
                "Editing Single Sign-On (SSO) password (applies to all servers)".to_string()
            } else {
                format!(
                    "Editing password override for {}",
                    app.panels[manager.selected].server.target()
                )
            };
            lines.push(Line::from(target_desc));
            lines.push(Line::from(vec![
                Span::raw("Password: "),
                Span::styled(
                    "*".repeat(manager.input.chars().count()),
                    Style::default().fg(accent),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                "[Enter] Save to OS credential store  [Esc] Cancel",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "[P] Toggle Sparklines (Experimental)  [Enter/S] SSO Password  [O] Override  [A] Add  [D] Delete",
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
