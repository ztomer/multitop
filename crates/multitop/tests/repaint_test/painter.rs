//! The screen model alone: bytes in, paint operations out.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::tasks::{Paint, Painter};

/// Feed one complete line, terminated the way a pty terminates lines.
///
/// `\r\n`, not `\n`. Every line from the exec channel arrives that way now,
/// on every host and for the local panel, so a test that fed a bare `\n` would
/// be testing a stream this program no longer receives.
fn line(p: &mut Painter, text: &str) -> Vec<Paint> {
    p.feed_bytes(format!("{text}\r\n").as_bytes())
}

/// The single paint one line produced, or a failure naming what came instead.
fn one(p: &mut Painter, text: &str) -> Paint {
    let mut paints = line(p, text);
    assert_eq!(
        paints.len(),
        1,
        "expected one paint for {text:?}: {paints:?}"
    );
    paints.remove(0)
}

#[test]
fn ordinary_lines_simply_follow_one_another() {
    let mut p = Painter::new();
    for text in ["one", "two", "three"] {
        let paint = one(&mut p, text);
        assert_eq!(paint.text, text);
        assert_eq!(paint.back, 0, "nothing moved the cursor, so it appends");
    }
}

/// The regression that sent me looking. `painted_states("a\r")` is
/// `["a", ""]`, whose last element is empty -- so the reader at HEAD collapsed
/// every CRLF-terminated line to nothing. A pty terminates *every* line that
/// way, so on the transport a stale `ControlPath` file selects, the whole
/// upgrade log went blank. The line-based reader before it was safe by
/// accident: `tokio`'s `Lines` strips the `\r` for you.
#[test]
fn a_crlf_terminated_line_keeps_its_text() {
    let mut p = Painter::new();
    assert_eq!(p.feed_bytes(b"hello\r\n")[0].text, "hello");
    assert_eq!(p.feed_bytes(b"plain\n")[0].text, "plain");
}

/// One line in, one paint out. The reader at HEAD split on `\r` *and* `\n`,
/// so a CRLF line fed the painter twice and the second, empty feed moved the
/// cursor a second time -- which is how `ESC[nA` blocks drifted and appended
/// copies instead of overwriting.
#[test]
fn a_crlf_line_paints_once_and_moves_the_cursor_once() {
    let mut p = Painter::new();
    assert_eq!(
        p.feed_bytes(b"a\r\nb\r\n").len(),
        2,
        "two lines, two paints"
    );
    // If the cursor had moved twice per line, this would not land on the line
    // two rows up.
    let paint = one(&mut p, "\u{1b}[2Aover-a");
    assert_eq!(paint.back, 2);
}

#[test]
fn a_cursor_up_places_the_next_line_over_one_already_shown() {
    let mut p = Painter::new();
    one(&mut p, "layer one: pulling");
    one(&mut p, "layer two: pulling");

    let first = one(&mut p, "\u{1b}[2Alayer one: extracting");
    assert_eq!(first.text, "layer one: extracting");
    assert_eq!(first.back, 2, "two rows above the append point");

    let second = one(&mut p, "layer two: extracting");
    assert_eq!(second.back, 1, "the row below the one just written");

    assert_eq!(one(&mut p, "\u{1b}[2Alayer one: done").back, 2);
    assert_eq!(one(&mut p, "layer two: done").back, 1);
}

#[test]
fn a_bare_cursor_up_carries_to_the_next_line_that_has_text() {
    let mut p = Painter::new();
    one(&mut p, "one");
    one(&mut p, "two");
    assert!(
        line(&mut p, "\u{1b}[2A").is_empty(),
        "movement alone draws nothing"
    );
    assert_eq!(
        one(&mut p, "over one").back,
        2,
        "the movement was remembered across the write that carried no text"
    );
}

#[test]
fn a_missing_count_means_one_row() {
    let mut p = Painter::new();
    one(&mut p, "one");
    assert_eq!(one(&mut p, "\u{1b}[Aover one").back, 1);
}

#[test]
fn moving_back_down_returns_to_the_end() {
    let mut p = Painter::new();
    one(&mut p, "one");
    one(&mut p, "two");
    assert!(line(&mut p, "\u{1b}[2A").is_empty());
    assert!(line(&mut p, "\u{1b}[2B").is_empty());
    assert_eq!(one(&mut p, "three").back, 0, "back at the append point");
}

#[test]
fn moving_up_past_the_top_stops_at_the_top() {
    let mut p = Painter::new();
    one(&mut p, "one");
    assert!(line(&mut p, "\u{1b}[9B").is_empty());
    assert_eq!(one(&mut p, "two").back, 0);
}

#[test]
fn erasing_downward_is_reported_so_the_stale_rows_can_be_blanked() {
    let mut p = Painter::new();
    one(&mut p, "one");
    one(&mut p, "two");
    one(&mut p, "three");
    let paint = one(&mut p, "\u{1b}[3A\u{1b}[Jshorter");
    assert_eq!(paint.back, 3);
    assert_eq!(paint.erase_below, 1);
}

#[test]
fn erasing_a_line_needs_no_reporting_because_the_write_replaces_it() {
    let mut p = Painter::new();
    one(&mut p, "one");
    let paint = one(&mut p, "\u{1b}[A\u{1b}[Kover one");
    assert_eq!(paint.erase_below, 0);
    assert_eq!(paint.back, 1);
}

#[test]
fn carriage_returns_still_collapse_within_the_line_being_painted() {
    let mut p = Painter::new();
    let paint = one(&mut p, "10%\r50%\r100%");
    assert_eq!(paint.text, "100%");
    assert_eq!(paint.back, 0);
}

/// A bar that has just moved its cursor to column 0 and written nothing yet
/// still *shows* its last value. Blanking it would make every progress display
/// flicker between its number and nothing.
#[test]
fn a_line_ending_in_a_bare_carriage_return_keeps_what_is_on_screen() {
    let mut p = Painter::new();
    let paints = p.feed_bytes(b"[###   ] 42%\r");
    assert_eq!(paints.len(), 1);
    assert_eq!(paints[0].text, "[###   ] 42%");
}

/// A prompt has no newline, and the operator cannot answer one they cannot see.
/// It is painted as soon as it arrives and overwritten in place as the rest of
/// it comes -- never appended twice, which is what the old 100 ms flush did
/// because it never cleared the buffer it had just sent.
#[test]
fn a_prompt_arriving_in_pieces_is_one_line_that_grows() {
    let mut p = Painter::new();
    let first = p.feed_bytes(b"Continue? ");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].text, "Continue? ");
    assert_eq!(first[0].back, 0, "nothing there yet, so it appends");

    let second = p.feed_bytes(b"[Y/n] ");
    assert_eq!(second.len(), 1);
    assert_eq!(
        second[0].text, "Continue? [Y/n] ",
        "the line grew; it was not replaced by its own tail"
    );
    assert_eq!(second[0].back, 1, "overwrites the line it already painted");

    // And answering it closes the line rather than starting a third copy.
    let third = p.feed_bytes(b"y\r\n");
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].text, "Continue? [Y/n] y");
    assert_eq!(third[0].back, 1);
    assert_eq!(one(&mut p, "next").back, 0, "the next line appends");
}

/// The property the reported defect is about, stated directly.
#[test]
fn no_line_is_ever_painted_at_the_append_point_twice() {
    let mut p = Painter::new();
    // One line delivered in five pieces, the way a slow remote actually sends.
    let mut appends = 0;
    for chunk in [
        &b"Reading "[..],
        b"package ",
        b"lists",
        b"... Done",
        b"\r\n",
    ] {
        for paint in p.feed_bytes(chunk) {
            if paint.back == 0 {
                appends += 1;
            }
        }
    }
    assert_eq!(
        appends, 1,
        "one line of output must claim the append point exactly once"
    );
}

/// A chunk boundary can fall anywhere, including inside an escape sequence.
#[test]
fn an_escape_sequence_split_across_chunks_is_still_understood() {
    let mut p = Painter::new();
    one(&mut p, "one");
    one(&mut p, "two");
    p.feed_bytes(b"\x1b[");
    p.feed_bytes(b"2");
    let paints = p.feed_bytes(b"Aover one\r\n");
    assert_eq!(paints.len(), 1);
    assert_eq!(paints[0].text, "over one");
    assert_eq!(
        paints[0].back, 2,
        "the sequence was reassembled, not dropped"
    );
}

#[test]
fn a_blank_line_is_output_and_is_kept() {
    let mut p = Painter::new();
    one(&mut p, "one");
    let paints = p.feed_bytes(b"\r\n");
    assert_eq!(paints.len(), 1, "a blank line between paragraphs is output");
    assert_eq!(paints[0].text, "");
    assert_eq!(paints[0].back, 0);
}

#[test]
fn a_sequence_this_model_does_not_know_is_left_in_the_text() {
    let mut p = Painter::new();
    let paint = one(&mut p, "\u{1b}[31mred\u{1b}[0m");
    assert!(
        paint.text.contains("\u{1b}[31m"),
        "colour survives: {paint:?}"
    );
    assert_eq!(paint.back, 0);
}

#[test]
fn cursor_previous_line_moves_cursor_up() {
    let mut p = Painter::new();
    one(&mut p, "one");
    one(&mut p, "two");
    assert_eq!(one(&mut p, "\u{1b}[2Fover one").back, 2);
}

#[test]
fn sgr_and_cr_preceding_cursor_up_does_not_block_parsing() {
    let mut p = Painter::new();
    one(&mut p, "one");
    let paint = one(&mut p, "\r\u{1b}[2A\u{1b}[32mgreen");
    assert_eq!(paint.back, 2);
    assert!(paint.text.contains("green"));
}

#[test]
fn private_modes_like_cursor_hide_are_consumed() {
    let mut p = Painter::new();
    let paint = one(&mut p, "\u{1b}[?25lworking");
    assert_eq!(paint.text, "working");
    assert_eq!(paint.back, 0);
}

/// Nothing more is coming and the last line never got its newline. It is
/// already on screen; `finish` exists so a caller can close the painter without
/// wondering whether anything is owed.
#[test]
fn finishing_repaints_an_unterminated_last_line_rather_than_appending_it() {
    let mut p = Painter::new();
    p.feed_bytes(b"partial output");
    let last = p.finish().expect("an unterminated line is still output");
    assert_eq!(last.text, "partial output");
    assert_eq!(
        last.back, 1,
        "it overwrites itself; it is not a second copy"
    );
    assert!(p.finish().is_none(), "and there is nothing owed after that");
}
