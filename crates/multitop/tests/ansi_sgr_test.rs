//! Every SGR code the converter claims to know.
//!
//! The agent emits a small subset, so the rest of the table only ever runs
//! against output from an `upgrade_cmd` — apt, dpkg, a user's own script — and
//! a code silently falling through to `_ => {}` shows up as text that lost its
//! colour, which nobody reports as a bug.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::ansi::line_to_spans;
use ratatui::style::{Color, Modifier};

/// The style the first span of `input` carries.
fn style_of(input: &str) -> ratatui::style::Style {
    let line = line_to_spans(input);
    line.spans.first().expect("a span must be produced").style
}

fn sgr(code: &str) -> ratatui::style::Style {
    style_of(&format!("\x1b[{code}mtext"))
}

#[test]
fn every_foreground_colour_maps_to_its_own_colour() {
    for (code, want) in [
        (30, Color::Black),
        (31, Color::Red),
        (32, Color::Green),
        (33, Color::Yellow),
        (34, Color::Blue),
        (35, Color::Magenta),
        (36, Color::Cyan),
        (37, Color::Gray),
        (90, Color::DarkGray),
        (91, Color::LightRed),
        (92, Color::LightGreen),
        (93, Color::LightYellow),
        (94, Color::LightBlue),
        (95, Color::LightMagenta),
        (96, Color::LightCyan),
        (97, Color::White),
    ] {
        assert_eq!(sgr(&code.to_string()).fg, Some(want), "SGR {code}");
    }
}

#[test]
fn every_background_colour_maps_to_its_own_colour() {
    for (code, want) in [
        (40, Color::Black),
        (41, Color::Red),
        (42, Color::Green),
        (43, Color::Yellow),
        (44, Color::Blue),
        (45, Color::Magenta),
        (46, Color::Cyan),
        (47, Color::Gray),
        (100, Color::DarkGray),
        (101, Color::LightRed),
        (102, Color::LightGreen),
        (103, Color::LightYellow),
        (104, Color::LightBlue),
        (105, Color::LightMagenta),
        (106, Color::LightCyan),
        (107, Color::White),
    ] {
        assert_eq!(sgr(&code.to_string()).bg, Some(want), "SGR {code}");
    }
}

#[test]
fn truecolour_and_indexed_colour_are_both_understood() {
    assert_eq!(sgr("38;2;10;20;30").fg, Some(Color::Rgb(10, 20, 30)));
    assert_eq!(sgr("48;2;40;50;60").bg, Some(Color::Rgb(40, 50, 60)));
    assert_eq!(sgr("38;5;123").fg, Some(Color::Indexed(123)));
    assert_eq!(sgr("48;5;200").bg, Some(Color::Indexed(200)));
}

#[test]
fn a_colour_component_out_of_range_clamps_rather_than_wrapping() {
    // 300 does not fit a byte. Wrapping it would silently produce a different
    // colour; falling back to zero at least does not lie about which channel.
    assert_eq!(sgr("38;2;300;0;0").fg, Some(Color::Rgb(0, 0, 0)));
    assert_eq!(sgr("38;5;999").fg, Some(Color::Indexed(0)));
}

#[test]
fn a_truncated_extended_colour_is_ignored_rather_than_half_applied() {
    // Not enough parameters to name a colour: the sequence is dropped, not
    // applied with whatever happened to follow it.
    assert_eq!(sgr("38;2;10").fg, None);
    assert_eq!(sgr("38;5").fg, None);
    assert_eq!(sgr("48;2").bg, None);
}

#[test]
fn bold_and_dim_go_on_and_come_back_off() {
    assert!(sgr("1").add_modifier.contains(Modifier::BOLD));
    assert!(sgr("2").add_modifier.contains(Modifier::DIM));

    // 22 removes both, whichever was set.
    let both_off = sgr("1;2;22");
    assert!(!both_off.add_modifier.contains(Modifier::BOLD));
    assert!(!both_off.add_modifier.contains(Modifier::DIM));
}

#[test]
fn the_default_colour_codes_clear_only_their_own_channel() {
    let fg_cleared = sgr("31;44;39");
    assert_eq!(fg_cleared.fg, None, "39 did not clear the foreground");
    assert_eq!(
        fg_cleared.bg,
        Some(Color::Blue),
        "39 cleared the background too"
    );

    let bg_cleared = sgr("31;44;49");
    assert_eq!(bg_cleared.bg, None, "49 did not clear the background");
    assert_eq!(
        bg_cleared.fg,
        Some(Color::Red),
        "49 cleared the foreground too"
    );
}

#[test]
fn reset_clears_everything_that_came_before_it() {
    let reset = sgr("1;31;44;0");
    assert_eq!(reset.fg, None);
    assert_eq!(reset.bg, None);
    assert!(!reset.add_modifier.contains(Modifier::BOLD));

    // A bare `\x1b[m` is a reset too, with no parameters at all.
    let bare = style_of("\x1b[31m\x1b[mtext");
    assert_eq!(bare.fg, None, "a bare reset left the colour on");
}

#[test]
fn an_empty_parameter_reads_as_zero_rather_than_shifting_the_rest() {
    // `\x1b[;31m` has an empty first parameter. Dropping it instead of
    // recording a zero would make 31 the *first* parameter and change what a
    // multi-parameter sequence means.
    assert_eq!(sgr(";31").fg, Some(Color::Red));
    assert_eq!(sgr("31;;44").bg, Some(Color::Blue));
}

#[test]
fn a_code_this_converter_does_not_know_is_skipped_not_printed() {
    // Unknown SGR codes leave the style alone and never appear as text.
    let line = line_to_spans("\x1b[31m\x1b[7;53;73mred");
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "red", "an unhandled code leaked into the text");
    assert_eq!(line.spans[0].style.fg, Some(Color::Red));
}

#[test]
fn escapes_that_are_not_sgr_are_dropped_whole() {
    // Cursor moves and erases are meaningless inside a widget that redraws its
    // whole area — but they must not print as literals either.
    for seq in ["\x1b[2J", "\x1b[1;1H", "\x1b[K", "\x1b[?25l", "\x1b[3A"] {
        let line = line_to_spans(&format!("{seq}visible"));
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "visible", "{seq:?} was printed as text");
    }
}

#[test]
fn an_unterminated_escape_does_not_leak_into_the_text() {
    let line = line_to_spans("before\x1b[31");
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "before");
}

#[test]
fn more_parameters_than_the_buffer_holds_do_not_overrun_it() {
    // Eight is the cap; a sequence with more must stop recording rather than
    // writing past the array.
    let s = sgr("1;1;1;1;1;1;1;1;1;1;1;31");
    assert!(s.add_modifier.contains(Modifier::BOLD));
    // The 31 came after the cap, so it was never recorded.
    assert_eq!(s.fg, None);
}

#[test]
fn a_sequence_with_no_parameters_at_all_resets_the_style() {
    // `\x1b[m` is a reset with an empty parameter list, which is not the same
    // as a list containing zero — the empty case has its own arm.
    let line = line_to_spans("\x1b[1;31mred\x1b[mplain");
    let plain = line
        .spans
        .iter()
        .find(|s| s.content.as_ref() == "plain")
        .expect("the text after the reset must be a span");
    assert_eq!(plain.style.fg, None, "the colour survived a bare reset");
    assert!(!plain.style.add_modifier.contains(Modifier::BOLD));
}
