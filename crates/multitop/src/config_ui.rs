//! Rendering for the full-screen configuration panel.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

/// Cells that never shrink: the credential state, and the port.
///
/// The state is the whole reason this screen exists, and a port is five digits
/// or it is wrong.
const STATE_W: usize = 8; // "\u{2713} Stored"
const PORT_W: usize = 5;
/// The marker column plus one space between each of the five cells.
const GAPS: usize = 6;

/// Cut a cell's text to the width its column has.
///
/// `{:<26}` pads but never truncates, so a longer upgrade command simply pushed
/// the Password column off to the right and the row stopped lining up with the
/// header above it. Counted in characters, and the last one becomes an ellipsis
/// so a clipped value cannot be mistaken for the whole of a short one.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

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
    // The row used to be 75 fixed columns before the credential state, so at 80
    // the Password header read `Pas` and the rows read `✓ S`; at 40 the column
    // was gone. This is the one screen where the user deletes a host and edits
    // its stored password, and whether that host HAS one was the thing amputated
    // at the right margin. State must be visible where the decision is made.
    //
    // So: the state cell and the port never shrink, and Server / User / Upgrade
    // command share whatever the terminal actually gives. A short `user` hands
    // its surplus to a long command rather than hoarding it.
    let inner = usize::from(f.area().width.saturating_sub(2));
    let rows: Vec<(String, String)> = app
        .panels
        .iter()
        .map(|p| {
            let user = if p.server.user.is_empty() {
                "default".to_string()
            } else {
                p.server.user.clone()
            };
            (p.server.host.clone(), user)
        })
        .collect();
    let want_host = rows
        .iter()
        .map(|(h, _)| h.chars().count())
        .max()
        .unwrap_or(6);
    let want_user = rows
        .iter()
        .map(|(_, u)| u.chars().count())
        .max()
        .unwrap_or(4);
    let want_cmd = app
        .panels
        .iter()
        .map(|p| {
            p.server
                .upgrade_cmd
                .as_deref()
                .unwrap_or("-")
                .chars()
                .count()
        })
        .max()
        .unwrap_or(3);
    let cells = crate::layout::share_width(
        STATE_W + PORT_W + GAPS,
        &[6, 4, 3],
        &[want_host.max(6), want_user.max(4), want_cmd.max(3)],
        inner,
    );
    let (host_w, user_w, cmd_w) = (cells[0], cells[1], cells[2]);

    let mut lines = vec![Line::from(Span::styled(
        format!(
            "  {:<host_w$} {:<user_w$} {:<PORT_W$} {:<cmd_w$} {}",
            clip("Server", host_w),
            clip("User", user_w),
            "Port",
            clip("Upgrade command", cmd_w),
            "Password"
        ),
        Style::default().fg(Color::DarkGray),
    ))];
    for (index, panel) in app.panels.iter().enumerate() {
        let marker = if index == manager.selected { ">" } else { " " };
        let user = if panel.server.user.is_empty() {
            "default"
        } else {
            &panel.server.user
        };
        let command = clip(panel.server.upgrade_cmd.as_deref().unwrap_or("-"), cmd_w);
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
                    "{marker} {:<host_w$} {:<user_w$} {:<PORT_W$} {:<cmd_w$} ",
                    clip(&panel.server.host, host_w),
                    clip(user, user_w),
                    panel.server.port,
                    command
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
        // Wrapped by whole hints, never sliced.
        //
        // This was two hand-split lines sized for a wide terminal. At 40 columns
        // `Paragraph` cut them where it liked, which left an orphaned `[` -- it
        // reads as a rendering fault, not a hint -- and shed `[Esc/Q] Return`.
        // The settings panel paints over the keybar, so that was the only exit
        // signage on the screen.
        //
        // `[Esc/Q] Return` therefore goes FIRST (Kare): it is the only hint here
        // a user cannot guess, so it must never be the one that falls off the
        // end. The rest they will find by pressing things.
        //
        // Wrapping rather than shedding, because this panel has vertical room to
        // spare -- there is no reason to lose a hint when a second line is free.
        let mut row: Vec<&str> = Vec::new();
        let mut used = 0usize;
        for hint in [
            "[Esc/Q] Return",
            "[Enter/E] Edit",
            "[A] Add",
            "[D] Delete",
            "[I] Import ~/.ssh/config",
            "[R] Change vault master password",
        ] {
            let w = hint.chars().count();
            let extra = if row.is_empty() { w } else { w + 2 };
            if !row.is_empty() && used + extra > inner {
                lines.push(Line::from(Span::styled(
                    row.join("  "),
                    Style::default().fg(Color::DarkGray),
                )));
                row.clear();
                used = 0;
            }
            used += if row.is_empty() { w } else { w + 2 };
            row.push(hint);
        }
        if !row.is_empty() {
            lines.push(Line::from(Span::styled(
                row.join("  "),
                Style::default().fg(Color::DarkGray),
            )));
        }
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
