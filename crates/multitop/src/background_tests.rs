//! Tests for the background probe.
//!
//! The cases that matter are the ones a real terminal produces: a well-formed
//! reply, a terminal that stays silent, and a terminal that answers with
//! something that is not a colour. A probe whose happy path is the only tested
//! path has tested the path that was never in doubt.

use super::{
    detect_with, parse_colorfgbg, parse_osc11_reply, Background, Rgb, Tty, OSC11_QUERY,
    PROBE_TIMEOUT,
};
use std::time::Duration;

/// A `Tty` that answers from a script, so every case below is reproducible.
struct Scripted {
    reply: Option<&'static str>,
    /// Whether the query was actually sent, so a test can prove the probe asked.
    asked: bool,
}

impl Tty for Scripted {
    fn query(&mut self, query: &str, _timeout: Duration) -> Option<String> {
        self.asked = true;
        assert_eq!(query, OSC11_QUERY, "the probe must ask exactly this");
        self.reply.map(str::to_string)
    }
}

fn no_env(_key: &str) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// Parsing the OSC 11 reply.
// ---------------------------------------------------------------------------

#[test]
fn it_parses_a_standard_reply() {
    // The exact shape iTerm2 answers with.
    assert_eq!(
        parse_osc11_reply("\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
        Some(Rgb::new(255, 255, 255))
    );
    assert_eq!(
        parse_osc11_reply("\x1b]11;rgb:0000/0000/0000\x1b\\"),
        Some(Rgb::new(0, 0, 0))
    );
    // A mid grey, the case a real user hits.
    assert_eq!(
        parse_osc11_reply("\x1b]11;rgb:1e1e/1e1e/2e2e\x07"),
        Some(Rgb::new(30, 30, 46))
    );
}

#[test]
fn it_scales_by_the_width_that_was_actually_sent() {
    // The reply is 16-bit per component, so a 1-digit `f` is full white and a
    // 4-digit `ffff` is the same white. Reading either as a byte offset gives
    // 15 and 255 for the same colour, and a palette adapted to 15 is adapted
    // to the wrong background.
    let one = parse_osc11_reply("\x1b]11;rgb:f/f/f\x07");
    let four = parse_osc11_reply("\x1b]11;rgb:ffff/ffff/ffff\x07");
    assert_eq!(one, four);
    assert_eq!(one, Some(Rgb::new(255, 255, 255)));
    // And a mid value scales the same way at either width.
    let one = parse_osc11_reply("\x1b]11;rgb:8/8/8\x07");
    let four = parse_osc11_reply("\x1b]11;rgb:8888/8888/8888\x07");
    assert_eq!(one, four);
}

#[test]
fn it_parses_the_hash_form() {
    // Some terminals answer `#rrggbb` rather than `rgb:`.
    assert_eq!(
        parse_osc11_reply("\x1b]11;#ffffff\x1b\\"),
        Some(Rgb::new(255, 255, 255))
    );
    assert_eq!(
        parse_osc11_reply("\x1b]11;#1e1e2e\x07"),
        Some(Rgb::new(30, 30, 46))
    );
}

#[test]
fn it_tolerates_echoed_input_before_the_reply() {
    // The probe reads raw stdin, and a keypress that arrived first is in the
    // same stream. Refusing the reply because it is not at the start would make
    // the probe fail exactly when the user is typing, which is when a wrong
    // background matters most.
    assert_eq!(
        parse_osc11_reply("x\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
        Some(Rgb::new(255, 255, 255))
    );
}

#[test]
fn it_refuses_replies_that_are_not_colours() {
    for bad in [
        "\x1b]11;rgb:ffff/ffff\x1b\\",           // two components
        "\x1b]11;rgb:ffff/ffff/ffff/ffff\x1b\\", // four
        "\x1b]11;rgb:zzzz/ffff/ffff\x1b\\",      // not hex
        "\x1b]11;rgb:/ffff/ffff\x1b\\",          // empty component
        "\x1b]11;none\x1b\\",                    // a name, not a colour
        "\x1b]11;rgb:fffff/ffff/ffff\x1b\\",     // too many digits for a u16
        "\x1b]11;#fff\x1b\\",                    // short hash
        "no prefix at all",
    ] {
        assert_eq!(parse_osc11_reply(bad), None, "should have refused {bad:?}");
    }
}

// ---------------------------------------------------------------------------
// COLORFGBG.
// ---------------------------------------------------------------------------

#[test]
fn it_reads_the_background_field_of_colorfgbg() {
    assert_eq!(parse_colorfgbg("0;0"), Some(Rgb::new(0, 0, 0)));
    assert_eq!(parse_colorfgbg("15;7"), Some(Rgb::new(229, 229, 229)));
    assert_eq!(parse_colorfgbg("0;15"), Some(Rgb::new(255, 255, 255)));
    assert_eq!(parse_colorfgbg("  0;7  "), Some(Rgb::new(229, 229, 229)));
}

#[test]
fn it_refuses_the_unset_default_rather_than_trusting_it() {
    // The trap. An emulator that was never configured leaves COLORFGBG at "7;0",
    // and index 7 is a light grey -- so reading it as a background adapts a dark
    // terminal to a light palette. Falling back to the theme's own background is
    // the safe answer; the string is not a measurement.
    assert_eq!(parse_colorfgbg("7;0"), None);
    assert_eq!(parse_colorfgbg("0;7"), Some(Rgb::new(229, 229, 229)));
}

#[test]
fn it_refuses_colorfgbg_it_cannot_read() {
    for bad in ["", "nonsense", "0", "0;", "0;99", "0;abc", ";"] {
        assert_eq!(parse_colorfgbg(bad), None, "should have refused {bad:?}");
    }
}

// ---------------------------------------------------------------------------
// The probe as a whole: which source answers, and in what order.
// ---------------------------------------------------------------------------

#[test]
fn osc11_wins_over_colorfgbg_when_both_are_available() {
    // OSC 11 is the terminal reporting on itself now; COLORFGBG is a value from
    // whenever the terminal was launched. The fresher measurement is the right
    // one, and this is the ordering that makes detection mean anything.
    let mut tty = Scripted {
        reply: Some("\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
        asked: false,
    };
    let env = |key: &str| (key == "COLORFGBG").then(|| "0;0".to_string());
    let found = detect_with(&env, &mut tty);
    assert_eq!(found, Background::Reported(Rgb::new(255, 255, 255)));
    assert_eq!(found.source(), "OSC 11");
    assert!(tty.asked, "the terminal must actually be asked");
}

#[test]
fn colorfgbg_is_the_fallback_when_the_terminal_stays_silent() {
    // The common case: a terminal that never answers. This must not hang and
    // must not fail -- it falls through.
    let mut tty = Scripted {
        reply: None,
        asked: false,
    };
    let env = |key: &str| (key == "COLORFGBG").then(|| "0;15".to_string());
    let found = detect_with(&env, &mut tty);
    assert_eq!(found, Background::Environment(Rgb::new(255, 255, 255)));
    assert!(tty.asked);
}

#[test]
fn an_unparseable_reply_falls_through_rather_than_becoming_a_colour() {
    // A terminal that answers with something that is not a colour has not
    // answered. Treating the answer as a colour would be a guess wearing a
    // measurement's clothes.
    let mut tty = Scripted {
        reply: Some("\x1b]11;something-else\x1b\\"),
        asked: false,
    };
    let env = |key: &str| (key == "COLORFGBG").then(|| "0;0".to_string());
    assert_eq!(
        detect_with(&env, &mut tty),
        Background::Environment(Rgb::new(0, 0, 0))
    );
}

#[test]
fn a_silent_terminal_with_no_environment_is_honestly_unknown() {
    let mut tty = Scripted {
        reply: None,
        asked: false,
    };
    let found = detect_with(&no_env, &mut tty);
    assert_eq!(found, Background::Unknown);
    assert_eq!(found.source(), "unknown");
    // And Unknown defers to the theme, rather than guessing dark.
    assert_eq!(found.or_theme(Rgb::new(46, 52, 64)), Rgb::new(46, 52, 64));
    // While a real measurement overrides the theme.
    assert_eq!(
        Background::Reported(Rgb::new(255, 255, 255)).or_theme(Rgb::new(46, 52, 64)),
        Rgb::new(255, 255, 255)
    );
}

#[test]
fn the_default_colorfgbg_alone_is_not_treated_as_a_measurement() {
    // Both probes useless, and the environment string present but meaningless:
    // the answer is Unknown, not "light grey background".
    let mut tty = Scripted {
        reply: None,
        asked: false,
    };
    let env = |key: &str| (key == "COLORFGBG").then(|| "7;0".to_string());
    assert_eq!(detect_with(&env, &mut tty), Background::Unknown);
}

// ---------------------------------------------------------------------------
// The timeout, which is the property that makes the probe safe to ship.
// ---------------------------------------------------------------------------

#[test]
fn the_timeout_is_short_enough_to_be_paid_once_at_startup() {
    // Not a test of behaviour -- a test of the budget. 120 ms is under a frame
    // budget for most of a second; a probe that waited two seconds would be
    // felt as a slow launch on every terminal that cannot answer, and those are
    // the majority.
    assert!(PROBE_TIMEOUT <= Duration::from_millis(250));
    assert!(
        PROBE_TIMEOUT >= Duration::from_millis(50),
        "too tight for a slow terminal"
    );
}
