//! Convert the agent's ANSI output into ratatui spans.
//!
//! The agent emits a small, known set of SGR codes, so this handles exactly
//! those and skips anything else rather than pulling in a general terminal
//! emulator. Unrecognised escapes are dropped, never printed as literals.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

fn apply_sgr(style: Style, code: u16) -> Style {
    match code {
        0 => Style::default(),
        1 => style.add_modifier(Modifier::BOLD),
        2 => style.add_modifier(Modifier::DIM),
        22 => style.remove_modifier(Modifier::BOLD | Modifier::DIM),
        30 => style.fg(Color::Black),
        31 => style.fg(Color::Red),
        32 => style.fg(Color::Green),
        33 => style.fg(Color::Yellow),
        34 => style.fg(Color::Blue),
        35 => style.fg(Color::Magenta),
        36 => style.fg(Color::Cyan),
        37 => style.fg(Color::Gray),
        39 => Style { fg: None, ..style },
        90 => style.fg(Color::DarkGray),
        91 => style.fg(Color::LightRed),
        92 => style.fg(Color::LightGreen),
        93 => style.fg(Color::LightYellow),
        94 => style.fg(Color::LightBlue),
        95 => style.fg(Color::LightMagenta),
        96 => style.fg(Color::LightCyan),
        97 => style.fg(Color::White),
        _ => style,
    }
}

/// Parse one line of ANSI-coloured text into styled spans.
pub fn line_to_spans(input: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\x1b' {
            text.push(c);
            continue;
        }
        if chars.peek() != Some(&'[') {
            // Not a CSI sequence; drop the ESC rather than rendering it.
            continue;
        }
        chars.next();

        let mut params = String::new();
        let mut final_byte = None;
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                final_byte = Some(c);
                break;
            }
            params.push(c);
        }

        // Only SGR changes styling; cursor moves and erases are meaningless
        // inside a widget that redraws the whole area anyway.
        if final_byte != Some('m') {
            continue;
        }
        if !text.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut text), style));
        }
        if params.is_empty() {
            style = Style::default();
        }
        for part in params.split(';') {
            match part.parse::<u16>() {
                Ok(code) => style = apply_sgr(style, code),
                // "\x1b[m" is a bare reset; a junk parameter is ignored.
                Err(_) if part.is_empty() => style = apply_sgr(style, 0),
                Err(_) => {}
            }
        }
    }

    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// Convert a frame into renderable text.
pub fn to_text<S: AsRef<str>>(lines: &[S]) -> Text<'static> {
    Text::from(
        lines
            .iter()
            .map(|l| line_to_spans(l.as_ref()))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn plain_text_passes_through() {
        let l = line_to_spans("hello world");
        assert_eq!(plain(&l), "hello world");
        assert_eq!(l.spans.len(), 1);
    }

    #[test]
    fn empty_input_yields_empty_line() {
        assert!(line_to_spans("").spans.is_empty());
    }

    #[test]
    fn color_applied_to_following_text() {
        let l = line_to_spans("\x1b[0;31mred\x1b[0m");
        assert_eq!(plain(&l), "red");
        assert_eq!(l.spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn text_before_escape_keeps_prior_style() {
        let l = line_to_spans("plain\x1b[0;32mgreen");
        assert_eq!(l.spans.len(), 2);
        assert_eq!(l.spans[0].content, "plain");
        assert_eq!(l.spans[0].style.fg, None);
        assert_eq!(l.spans[1].content, "green");
        assert_eq!(l.spans[1].style.fg, Some(Color::Green));
    }

    #[test]
    fn bold_is_a_modifier_not_a_color() {
        let l = line_to_spans("\x1b[1mCPU\x1b[0m");
        assert!(l.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn reset_clears_everything() {
        let l = line_to_spans("\x1b[1m\x1b[0;31ma\x1b[0mb");
        assert_eq!(l.spans[1].content, "b");
        assert_eq!(l.spans[1].style.fg, None);
        assert!(l.spans[1].style.add_modifier.is_empty());
    }

    #[test]
    fn multiple_params_in_one_sequence() {
        let l = line_to_spans("\x1b[1;36mx");
        assert_eq!(l.spans[0].style.fg, Some(Color::Cyan));
        assert!(l.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn every_agent_color_maps() {
        for (code, want) in [
            (31, Color::Red),
            (32, Color::Green),
            (33, Color::Yellow),
            (34, Color::Blue),
            (36, Color::Cyan),
            (37, Color::Gray),
            (90, Color::DarkGray),
        ] {
            let l = line_to_spans(&format!("\x1b[0;{code}mx"));
            assert_eq!(l.spans[0].style.fg, Some(want), "code {code}");
        }
    }

    #[test]
    fn bare_reset_sequence() {
        let l = line_to_spans("\x1b[0;31ma\x1b[mb");
        assert_eq!(l.spans[1].style.fg, None);
    }

    /// Non-SGR escapes must vanish, not leak into the panel as literal text.
    #[test]
    fn non_sgr_escapes_are_dropped() {
        assert_eq!(plain(&line_to_spans("\x1b[H\x1b[Jclean")), "clean");
        assert_eq!(plain(&line_to_spans("\x1b[?25lhidden")), "hidden");
    }

    #[test]
    fn lone_escape_is_dropped() {
        assert_eq!(plain(&line_to_spans("a\x1bb")), "ab");
        assert_eq!(plain(&line_to_spans("trailing\x1b")), "trailing");
    }

    #[test]
    fn unterminated_sequence_does_not_emit_garbage() {
        assert_eq!(plain(&line_to_spans("a\x1b[0;31")), "a");
    }

    #[test]
    fn unicode_survives() {
        let l = line_to_spans("\x1b[0;36m\u{ff48}\u{2500}\u{2191}\x1b[0m");
        assert_eq!(plain(&l), "\u{ff48}\u{2500}\u{2191}");
    }

    #[test]
    fn text_keeps_one_line_per_input() {
        let t = to_text(&["a", "\x1b[0;31mb\x1b[0m", "c"]);
        assert_eq!(t.lines.len(), 3);
        assert_eq!(plain(&t.lines[1]), "b");
    }

    /// A real agent line must round-trip to exactly its visible characters.
    #[test]
    fn agent_line_round_trips() {
        let raw = " \x1b[1mMEM\x1b[0m \x1b[0;36m[####....]\x1b[0m \x1b[0;36m 50%\x1b[0m \x1b[0;90m1.0GiB/2.0GiB      \x1b[0m";
        assert_eq!(
            plain(&line_to_spans(raw)),
            " MEM [####....]  50% 1.0GiB/2.0GiB      "
        );
    }
}
