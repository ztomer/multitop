//! Convert the agent's ANSI output into ratatui spans.
//!
//! The agent emits a small, known set of SGR codes, so this handles exactly
//! those and skips anything else rather than pulling in a general terminal
//! emulator. Unrecognised escapes are dropped, never printed as literals.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

fn apply_sgr_params(mut style: Style, params: &[u16]) -> Style {
    if params.is_empty() {
        return Style::default();
    }
    let mut i = 0;
    while i < params.len() {
        match params[i] {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            30 => style = style.fg(Color::Black),
            31 => style = style.fg(Color::Red),
            32 => style = style.fg(Color::Green),
            33 => style = style.fg(Color::Yellow),
            34 => style = style.fg(Color::Blue),
            35 => style = style.fg(Color::Magenta),
            36 => style = style.fg(Color::Cyan),
            37 => style = style.fg(Color::Gray),
            38 if i + 4 < params.len() && params[i + 1] == 2 => {
                let r = params[i + 2] as u8;
                let g = params[i + 3] as u8;
                let b = params[i + 4] as u8;
                style = style.fg(Color::Rgb(r, g, b));
                i += 4;
            }
            38 if i + 2 < params.len() && params[i + 1] == 5 => {
                let idx = params[i + 2] as u8;
                style = style.fg(Color::Indexed(idx));
                i += 2;
            }
            39 => style.fg = None,
            40 => style = style.bg(Color::Black),
            41 => style = style.bg(Color::Red),
            42 => style = style.bg(Color::Green),
            43 => style = style.bg(Color::Yellow),
            44 => style = style.bg(Color::Blue),
            45 => style = style.bg(Color::Magenta),
            46 => style = style.bg(Color::Cyan),
            47 => style = style.bg(Color::Gray),
            48 if i + 4 < params.len() && params[i + 1] == 2 => {
                let r = params[i + 2] as u8;
                let g = params[i + 3] as u8;
                let b = params[i + 4] as u8;
                style = style.bg(Color::Rgb(r, g, b));
                i += 4;
            }
            48 if i + 2 < params.len() && params[i + 1] == 5 => {
                let idx = params[i + 2] as u8;
                style = style.bg(Color::Indexed(idx));
                i += 2;
            }
            49 => style.bg = None,
            90 => style = style.fg(Color::DarkGray),
            91 => style = style.fg(Color::LightRed),
            92 => style = style.fg(Color::LightGreen),
            93 => style = style.fg(Color::LightYellow),
            94 => style = style.fg(Color::LightBlue),
            95 => style = style.fg(Color::LightMagenta),
            96 => style = style.fg(Color::LightCyan),
            97 => style = style.fg(Color::White),
            100 => style = style.bg(Color::DarkGray),
            101 => style = style.bg(Color::LightRed),
            102 => style = style.bg(Color::LightGreen),
            103 => style = style.bg(Color::LightYellow),
            104 => style = style.bg(Color::LightBlue),
            105 => style = style.bg(Color::LightMagenta),
            106 => style = style.bg(Color::LightCyan),
            107 => style = style.bg(Color::White),
            _ => {}
        }
        i += 1;
    }
    style
}

/// Parse one line of ANSI-coloured text into styled spans.
pub fn line_to_spans(input: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\r'
            || c == '\x07'
            || c == '\x08'
            || (c < ' ' && c != '\t' && c != '\n' && c != '\x1b')
        {
            // Drop control characters (carriage return, backspace, bell, etc.)
            // so they don't corrupt Ratatui's terminal cursor/layout.
            continue;
        }

        if c != '\x1b' {
            text.push(c);
            continue;
        }

        // Check for OSC escape sequences (e.g. \x1b]0;title\x07 or \x1b]...\x1b\)
        if chars.peek() == Some(&']') {
            chars.next();
            while let Some(nc) = chars.next() {
                if nc == '\x07' {
                    break;
                }
                if nc == '\x1b' && chars.peek() == Some(&'\\') {
                    chars.next();
                    break;
                }
            }
            continue;
        }

        if chars.peek() != Some(&'[') {
            // Not a CSI sequence; drop the ESC rather than rendering it.
            continue;
        }
        chars.next();

        let mut final_byte = None;
        let mut params = [0u16; 8];
        let mut num_params = 0;
        let mut sgr_code: u16 = 0;
        let mut has_digits = false;
        let mut reset_bare = true;

        for c in chars.by_ref() {
            if c.is_ascii_digit() {
                sgr_code = sgr_code
                    .saturating_mul(10)
                    .saturating_add((c as u8 - b'0') as u16);
                has_digits = true;
                reset_bare = false;
            } else if c == ';' {
                if has_digits && num_params < 8 {
                    params[num_params] = sgr_code;
                    num_params += 1;
                    sgr_code = 0;
                    has_digits = false;
                } else if num_params < 8 {
                    params[num_params] = 0;
                    num_params += 1;
                }
                reset_bare = false;
            } else if c.is_ascii_alphabetic() {
                if has_digits && num_params < 8 {
                    params[num_params] = sgr_code;
                    num_params += 1;
                }
                final_byte = Some(c);
                break;
            }
        }

        // Only SGR changes styling; cursor moves and erases are meaningless
        // inside a widget that redraws the whole area anyway.
        if final_byte != Some('m') {
            continue;
        }
        if !text.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut text), style));
        }
        if reset_bare {
            style = Style::default();
        } else {
            style = apply_sgr_params(style, &params[..num_params]);
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

    #[test]
    fn osc_escapes_and_control_chars_are_dropped() {
        let raw = "hello\x1b]0;title\x07world\r\x08!";
        assert_eq!(plain(&line_to_spans(raw)), "helloworld!");
    }

    #[test]
    fn test_256_and_background_colors() {
        let line = line_to_spans("\x1b[38;5;208m\x1b[48;5;21m256-color\x1b[0m");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, Some(Color::Indexed(208)));
        assert_eq!(line.spans[0].style.bg, Some(Color::Indexed(21)));

        let line_rgb = line_to_spans("\x1b[48;2;10;20;30mrgb-bg\x1b[0m");
        assert_eq!(line_rgb.spans[0].style.bg, Some(Color::Rgb(10, 20, 30)));
    }
}
