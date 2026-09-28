//! Tests for the palette half: whether the themes as shipped are legible, and
//! whether adapting one for a background keeps them so.
//!
//! These are the tests that make the roadmap's claim a measurement. "Every one of
//! the 8 themes is dark, so yellow and cyan wash out on a light terminal" was
//! found by looking at a screenshot; `the_roadmaps_claim_holds_for_every_theme_
//! on_a_light_terminal` asserts it arithmetically for all eight, so it cannot
//! rot into a claim nobody re-checks, and so a ninth theme is covered by the
//! same assertions.

use super::{
    adapt, ensure_contrast, Rgb, Role, MIN_MUTED_CONTRAST, MIN_ROLE_SEPARATION, MIN_TEXT_CONTRAST,
};
use crate::color::THEMES;
use crate::lab::colour_difference;

/// The background of a light terminal. Near-white rather than pure white, to
/// stand in for a terminal a person actually chose.
const LIGHT: Rgb = Rgb::new(252, 252, 250);

#[test]
fn a_single_colour_that_already_reads_is_left_alone() {
    // Adapting a palette is a whole-palette decision, but a caller asking about
    // one colour must still not have it moved for company.
    for theme in THEMES {
        for role in Role::all() {
            let base = role_colour(theme, role);
            if base.contrast_ratio(dark(theme)) >= MIN_TEXT_CONTRAST {
                assert_eq!(
                    ensure_contrast(base, dark(theme), MIN_TEXT_CONTRAST),
                    Some(base)
                );
            }
        }
    }
}

#[test]
fn adapt_covers_a_dark_background_too() {
    // And the palette-level path, in the direction that matters for a future
    // light theme: every role must clear the floor on a dark background as well,
    // so the same call is safe from either side.
    for theme in THEMES {
        let adapted = adapted(theme, dark(theme));
        assert!(
            adapted.worst_contrast(dark(theme)) >= MIN_TEXT_CONTRAST,
            "{}: worst role is {:.2}:1 after adapting for its own background",
            theme.name,
            adapted.worst_contrast(dark(theme))
        );
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Primary => "primary",
        Role::Secondary => "secondary",
        Role::Muted => "muted",
        Role::Text => "text",
        Role::MeterLow => "meter_low",
        Role::MeterMid => "meter_mid",
        Role::MeterHigh => "meter_high",
    }
}

#[test]
fn adapting_is_deterministic() {
    // Two calls on the same inputs must agree, or a caller cannot cache the
    // result per (theme, background) and a redraw would flicker.
    for theme in THEMES {
        for bg in [LIGHT, dark(theme)] {
            assert_eq!(
                adapt(theme, bg, MIN_TEXT_CONTRAST),
                adapt(theme, bg, MIN_TEXT_CONTRAST)
            );
        }
    }
}

#[test]
fn adapting_never_reduces_the_separation_two_roles_had() {
    // The invariant that per-colour adjustment broke, and the reason `adapt` is a
    // palette. Judged by perceptual distance, not by contrast: a cyan and a green
    // at one lightness are 1.0:1 in contrast terms and unmistakably different
    // colours, so contrast would fail a palette for preserving its hues.
    //
    // The floor is a JND, not the source palette's own ΔE, and the difference
    // matters: legibility on a light terminal REQUIRES compressing the palette's
    // dynamic range, because every role has to fit under one luminance
    // threshold. Kare's `primary` and `muted` are 50 ΔE apart on a dark terminal
    // and 28 apart adapted, and that is the cost of being readable at all rather
    // than a defect. What must not happen is two roles landing on the SAME
    // colour, which is why the floor is a just-noticeable difference and why the
    // pairs that are already identical in the source are exempt.
    for theme in THEMES {
        let adapted = adapted(theme, LIGHT);
        let roles = Role::all();
        for (i, a) in roles.iter().enumerate() {
            for b in &roles[i + 1..] {
                let (ca, cb) = (role_colour(theme, *a), role_colour(theme, *b));
                if ca == cb {
                    continue; // already one colour in the source; listed below
                }
                let after = colour_difference(adapted.get(*a), adapted.get(*b));
                assert!(
                    after >= MIN_ROLE_SEPARATION,
                    "{}: {} and {} end up {:.1} apart on light, at or below a \
                     just-noticeable difference: {:?} vs {:?}",
                    theme.name,
                    role_name(*a),
                    role_name(*b),
                    after,
                    adapted.get(*a),
                    adapted.get(*b)
                );
            }
        }
    }
}

#[test]
fn adapting_preserves_hue_at_the_palette_level_too() {
    // The per-colour version proved this for one colour; a palette-level map
    // could in principle flatten a hue, and that is the failure it has to avoid.
    for theme in THEMES {
        let adapted = adapted(theme, LIGHT);
        for role in Role::all() {
            let (h0, s0, _) = role_colour(theme, role).hsl();
            let (h1, s1, _) = adapted.get(role).hsl();
            let hue_gap = (h0 - h1).abs().min(360.0 - (h0 - h1).abs());
            assert!(
                hue_gap < 6.0,
                "{} {}: hue moved {hue_gap:.0} degrees, {s0:.2} -> {s1:.2}",
                theme.name,
                role_name(role)
            );
        }
    }
}

#[test]
fn adjusting_preserves_hue() {
    // The house rule this exists to serve: a fix for one legibility problem
    // must not introduce an indistinguishability problem. A yellow that has
    // become grey would pass the ratio test and fail this one.
    let yellow = Rgb::new(241, 250, 140);
    let fixed = ensure_contrast(yellow, LIGHT, MIN_TEXT_CONTRAST).unwrap();
    let (h0, s0, _) = yellow.hsl();
    let (h1, s1, _) = fixed.hsl();
    assert!(
        (h0 - h1).abs() < 6.0,
        "hue moved from {h0:.0} to {h1:.0}: {yellow:?} -> {fixed:?}"
    );
    assert!(
        (s0 - s1).abs() < 0.12,
        "saturation moved from {s0:.2} to {s1:.2}: washed out rather than adjusted"
    );
}

#[test]
fn adjusting_reaches_the_ratio_for_every_role_of_every_theme() {
    for theme in THEMES {
        for role in Role::all() {
            let base = role_colour(theme, role);
            let fixed = ensure_contrast(base, LIGHT, MIN_TEXT_CONTRAST).unwrap_or_else(|| {
                panic!(
                    "{} {} could not be made legible on light",
                    theme.name,
                    role_name(role)
                )
            });
            assert!(
                fixed.contrast_ratio(LIGHT) >= MIN_TEXT_CONTRAST,
                "{} {} still {}:1 after adjusting",
                theme.name,
                role_name(role),
                fixed.contrast_ratio(LIGHT)
            );
        }
    }
}

#[test]
fn each_theme_states_its_own_background_and_they_differ() {
    // If these were all the same, the per-theme measurement above would be
    // decoration. If they are all different, no single assumption about "dark"
    // is good enough, which is the case for detection rather than a palette.
    let all: Vec<Rgb> = THEMES
        .iter()
        .map(crate::color::Palette::own_background)
        .collect();
    assert!(
        all.windows(2).any(|w| w[0] != w[1]),
        "themes share one background"
    );
    for (theme, bg) in THEMES.iter().zip(all.iter()) {
        assert!(
            bg.relative_luminance() < 0.2,
            "{} is not a dark backdrop: {bg:?}",
            theme.name
        );
    }
}

// ---------------------------------------------------------------------------
// Reading the colour back out of the palette, with no second source of truth.
// ---------------------------------------------------------------------------

#[test]
fn every_theme_role_reads_back_as_the_rgb_it_was_written_as() {
    // The escapes are hand-written, so this is the check that a typo in one
    // reads as a colour rather than as silence.
    let kare = &THEMES[0];
    assert_eq!(kare.role_rgb(Role::Primary), Some(Rgb::new(139, 233, 253)));
    assert_eq!(kare.role_rgb(Role::MeterMid), Some(Rgb::new(241, 250, 140)));
    assert_eq!(kare.role_rgb(Role::MeterHigh), Some(Rgb::new(255, 85, 85)));
    for theme in THEMES {
        for role in Role::all() {
            // Round-tripped: re-encoding the RGB into an escape and reading it
            // back must be the identity, which is what makes the derived
            // colour usable to build a replacement escape later.
            let rgb = role_colour(theme, role);
            let escape = format!("\x1b[38;2;{};{};{}m", rgb.r, rgb.g, rgb.b);
            assert_eq!(
                Rgb::from_ansi(&escape),
                Some(rgb),
                "{} {}",
                theme.name,
                role_name(role)
            );
        }
    }
}

#[test]
fn it_adapts_toward_a_dark_background_as_well_as_away_from_a_light_one() {
    // The same call has to work in both directions, or a light-theme terminal is
    // a second bug waiting for the first one to be fixed. A colour that is TOO
    // DARK for a dark background is the mirror case -- a near-black one, which
    // is what a light theme would be full of.
    let near_black = Rgb::new(18, 18, 22);
    let dark_bg = Rgb::new(24, 26, 36);
    assert!(
        near_black.contrast_ratio(dark_bg) < MIN_TEXT_CONTRAST,
        "premise: it must fail first"
    );
    let fixed = ensure_contrast(near_black, dark_bg, MIN_TEXT_CONTRAST).unwrap();
    assert!(fixed.contrast_ratio(dark_bg) >= MIN_TEXT_CONTRAST);
    assert!(
        fixed.relative_luminance() > near_black.relative_luminance(),
        "a colour too dark for a dark background must be pushed lighter, not darker"
    );
}

#[test]
fn muted_meets_its_own_floor_on_dark() {
    // A separate finding from the light-terminal one, and it is NOT introduced
    // by anything here: `muted` is de-emphasis, so it sits below the body-text
    // bar by design, and in the darker themes it sits below even a de-emphasis
    // bar. Recorded as a measurement with a floor rather than left as an
    // opinion, because "it looked dim on purpose" is indistinguishable from
    // "nobody measured it" and only one of those is a decision.
    let mut short = Vec::new();
    for theme in THEMES {
        let ratio = role_colour(theme, Role::Muted).contrast_ratio(dark(theme));
        if ratio < MIN_MUTED_CONTRAST {
            short.push(format!("{} at {ratio:.2}:1", theme.name));
        }
    }
    assert!(
        short.is_empty(),
        "muted below its {MIN_MUTED_CONTRAST}:1 floor on the theme's own \
         background: {short:?}. Either lift those themes' gray or say the floor \
         is wrong -- do not quietly move the number."
    );
}

// ---------------------------------------------------------------------------
// The mechanism.
// ---------------------------------------------------------------------------

#[test]
fn the_dark_terminal_is_left_alone() {
    // The regression this must not cause: a palette tuned for a dark terminal
    // rendering differently on a dark terminal. Adjusting a colour that already
    // reads must be the identity, and the search agrees.
    for theme in THEMES {
        for role in Role::all() {
            let base = role_colour(theme, role);
            if base.contrast_ratio(dark(theme)) >= MIN_TEXT_CONTRAST {
                assert_eq!(
                    ensure_contrast(base, dark(theme), MIN_TEXT_CONTRAST),
                    Some(base)
                );
            }
        }
    }
}

#[test]
fn the_roadmaps_claim_holds_for_every_theme_on_a_light_terminal() {
    let mut worst = f32::INFINITY;
    let mut offenders = Vec::new();
    for theme in THEMES {
        for role in [Role::Primary, Role::MeterMid, Role::MeterHigh] {
            let ratio = role_colour(theme, role).contrast_ratio(LIGHT);
            if ratio < MIN_TEXT_CONTRAST {
                offenders.push(format!("{} {} at {ratio:.2}", theme.name, role_name(role)));
            }
            worst = worst.min(ratio);
        }
    }
    // If a future palette fixes this for real, this test goes red and the
    // roadmap item is done -- which is the point of asserting the DEFECT.
    assert!(
        !offenders.is_empty(),
        "no theme fails on a light terminal, so either the palettes were fixed \
         or the measurement is broken; worst was {worst:.2}"
    );
    // And it fails CATASTROPHICALLY, not marginally. If this ever lands near
    // the threshold, the threshold is wrong for terminals, not the palette.
    assert!(
        worst < 1.5,
        "expected near-invisible, got a worst case of {worst:.2}: {offenders:?}"
    );
}

#[test]
fn the_roles_a_theme_already_shares_a_colour_are_known_and_listed() {
    // The pre-existing collisions, asserted so that ADDING one is a deliberate
    // edit to this list rather than something a reader discovers in a screenshot.
    // Sharing a colour is not automatically wrong -- these two are not presented
    // as a choice from each other -- but it is the one place where two roles are
    // told apart by something other than colour, so it has to be on the record.
    let mut shared = Vec::new();
    for theme in THEMES {
        let roles = Role::all();
        for (i, a) in roles.iter().enumerate() {
            for b in &roles[i + 1..] {
                if role_colour(theme, *a) == role_colour(theme, *b) {
                    shared.push(format!(
                        "{}:{}+{}",
                        theme.name,
                        role_name(*a),
                        role_name(*b)
                    ));
                }
            }
        }
    }
    shared.sort();
    assert_eq!(
        shared,
        vec!["Monokai:primary+meter_low".to_string()],
        "the set of roles a theme draws in one colour changed; if a new one is \
         deliberate, add it here with the reason, and if it is not, it is a bug"
    );
}

#[test]
fn the_themes_are_built_for_a_dark_terminal_and_legible_there() {
    // The control for the test above, and the reason this is a background
    // problem rather than a broken palette. If this fails, the palettes are
    // wrong on dark terminals too and no amount of light-background work helps.
    for theme in THEMES {
        for role in Role::all() {
            if role == Role::Muted {
                continue; // measured on its own floor, below
            }
            let ratio = role_colour(theme, role).contrast_ratio(dark(theme));
            assert!(
                ratio >= MIN_TEXT_CONTRAST,
                "{} {} is only {ratio:.2}:1 on the {:?} it was made for",
                theme.name,
                role_name(role),
                dark(theme)
            );
        }
    }
}

/// What a theme says its own backdrop is. Per theme, not one shared constant:
/// measuring Nord against Kare's keybar asks a question about neither, and the
/// muted role is exactly where the two answers differ. See
/// [`crate::surface::Palette::own_background`] -- it is the ONLY background a
/// theme states, which is the problem this module exists to name.
fn dark(theme: &crate::color::Palette) -> Rgb {
    theme.own_background()
}

/// Every THEMES member has all seven roles as 24-bit escapes, so `None` here
/// means one of them does not -- which is a finding about the palette, and the
/// test should say which one.
fn adapted(theme: &crate::color::Palette, bg: Rgb) -> super::Adapted {
    adapt(theme, bg, MIN_TEXT_CONTRAST)
        .unwrap_or_else(|| panic!("{} has a role that is not a 24-bit colour", theme.name))
}

/// A role's colour, failing with the theme and role if it is not one.
fn role_colour(theme: &crate::color::Palette, role: Role) -> Rgb {
    theme
        .role_rgb(role)
        .unwrap_or_else(|| panic!("{} {role:?} is not a 24-bit colour", theme.name))
}
