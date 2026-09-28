//! Tests for the colour arithmetic: what a colour IS, and the arithmetic that
//! measures one. Everything that involves a palette or a role is in
//! `adapt_tests.rs`, because the two halves answer different questions -- "is
//! this number right" and "should this palette be legible".

use super::Rgb;

/// A near-white background, the friendliest light terminal there is. A palette
/// that fails against this fails against every light background.
const LIGHT: Rgb = Rgb::new(252, 252, 250);

#[test]
fn an_escape_that_is_not_a_24_bit_colour_is_refused_rather_than_guessed() {
    // The shapes that must all be `None`. This is the check that replaced a
    // panic: `clippy::panic` is on in this repository, and a total function
    // that says "not a colour" is a better contract than one that aborts a
    // release build over a typo in a palette table.
    for bad in [
        "\x1b[31m",             // 8-bit, not 24-bit
        "\x1b[38;5;208m",       // the 256-colour form
        "",                     // PLAIN's entries are exactly this
        "\x1b[38;2;255;85m",    // two components
        "\x1b[38;2;;85;85m",    // an empty component
        "\x1b[38;2;abc;85;85m", // not digits
        "\x1b[38;2;999;85;85m", // does not fit a byte
        "\x1b[38;2;1;2;3;4m",   // a fourth component: not this escape
    ] {
        assert_eq!(Rgb::from_ansi(bad), None, "should have refused {bad:?}");
    }
    // And the one that works, so the list above is a list of refusals and not a
    // list of everything.
    assert_eq!(
        Rgb::from_ansi("\x1b[38;2;255;85;85m"),
        Some(Rgb::new(255, 85, 85))
    );
}

// ---------------------------------------------------------------------------
// The claim the roadmap makes, measured.
// ---------------------------------------------------------------------------

#[test]
fn contrast_is_symmetric() {
    let a = Rgb::new(241, 250, 140);
    assert!((a.contrast_ratio(LIGHT) - LIGHT.contrast_ratio(a)).abs() < 1e-6);
}

#[test]
fn contrast_ratio_spans_one_to_twenty_one() {
    const BLACK: Rgb = Rgb::new(0, 0, 0);
    const WHITE: Rgb = Rgb::new(255, 255, 255);
    assert!((BLACK.contrast_ratio(WHITE) - 21.0).abs() < 0.01);
    assert!((WHITE.contrast_ratio(BLACK) - 21.0).abs() < 0.01);
    assert!((Rgb::new(7, 7, 7).contrast_ratio(Rgb::new(7, 7, 7)) - 1.0).abs() < 0.001);
    // The other end of the published table: #767676 on white is the canonical
    // "just passes AA" grey, at 4.54.
    assert!((Rgb::new(0x76, 0x76, 0x76).contrast_ratio(WHITE) - 4.54).abs() < 0.02);
}

#[test]
fn hsl_round_trips() {
    for rgb in [
        Rgb::new(241, 250, 140),
        Rgb::new(139, 233, 253),
        Rgb::new(255, 85, 85),
        Rgb::new(255, 255, 255),
        Rgb::new(0, 0, 0),
        Rgb::new(128, 128, 128),
    ] {
        let (h, s, l) = rgb.hsl();
        let back = Rgb::from_hsl(h, s, l);
        assert!(
            (i16::from(back.r) - i16::from(rgb.r)).abs() <= 1
                && (i16::from(back.g) - i16::from(rgb.g)).abs() <= 1
                && (i16::from(back.b) - i16::from(rgb.b)).abs() <= 1,
            "{rgb:?} -> ({h:.1},{s:.3},{l:.3}) -> {back:?}"
        );
    }
}
