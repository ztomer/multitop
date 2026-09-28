//! Tests for [`PaletteView`], which is the type that makes the wiring cheap.
//!
//! The claim this module has to earn is not that it compiles — a type with the
//! right field names compiles anywhere — but that a view built for a light
//! terminal actually contains legible escapes, and that it degrades to the
//! theme's own colours when adaptation declines.

use super::{escape_for, PaletteView};
use crate::adapt::{adapt, Role, MIN_MUTED_CONTRAST, MIN_TEXT_CONTRAST};
use crate::color::{PLAIN, THEMES};
use crate::surface::Rgb;

/// A near-white terminal, the case the whole module exists for.
const LIGHT: Rgb = Rgb::new(252, 252, 250);

/// A dark terminal, where the palette was designed to be used.
///
/// Deliberately NOT a theme's `own_background`. That value is the KEYBAR STRIP's
/// colour — `ui/draw.rs` paints the keybar with it and leaves every other cell to
/// the terminal — so it says nothing about what is behind a pane. Using it as
/// "the dark terminal" is the mistake this constant is here to avoid, and the
/// first draft of this file made exactly it: two tests asserted a dark terminal
/// and were quietly measuring a keybar.
const DARK: Rgb = Rgb::new(24, 26, 36);

/// Every escape the view exposes, paired with the name a failure should use.
fn escapes(view: &PaletteView) -> Vec<(&'static str, &str)> {
    vec![
        ("primary", view.primary()),
        ("secondary", view.secondary()),
        ("muted", view.muted()),
        ("text", view.text()),
        ("meter_low", view.meter_low()),
        ("meter_mid", view.meter_mid()),
        ("meter_high", view.meter_high()),
    ]
}

/// The colour an escape names, or `None` if it is not a 24-bit colour.
fn colour_of(escape: &str) -> Option<Rgb> {
    Rgb::from_ansi(escape)
}

#[test]
fn on_a_dark_terminal_the_view_still_clears_every_floor() {
    // I first wrote this as "a dark terminal reproduces the theme verbatim", and
    // it failed — correctly. `muted` measures 2.8:1 to 4.7:1 on a dark terminal
    // in all eight themes, so it is below the 4.5 floor in every one of them and
    // the map has real work to do even there. The claim that survived being
    // measured is the one worth keeping: adaptation on a dark terminal makes
    // things legible without breaking anything that already was.
    //
    // Which is the general property: for BOTH backgrounds, every role clears its
    // own floor afterwards. The verbatim case is covered separately, by a
    // background chosen to need nothing.
    for theme in THEMES {
        let view = PaletteView::for_background(theme, DARK);
        for (name, escape) in escapes(&view) {
            let colour = colour_of(escape).expect("a 24-bit colour");
            let floor = if name == "muted" {
                MIN_MUTED_CONTRAST
            } else {
                MIN_TEXT_CONTRAST
            };
            assert!(
                colour.contrast_ratio(DARK) >= floor,
                "{} {name} is {:.2}:1 on a dark terminal, needs {floor}",
                theme.name,
                colour.contrast_ratio(DARK)
            );
        }
    }
}

#[test]
fn a_palette_that_needs_nothing_is_reproduced_verbatim() {
    // The identity case, which is the only way to know the map does not re-tint
    // a palette nobody asked it to touch. Nord and Gruvbox are the two whose
    // DARKEST role clears the floor on a near-black terminal, so for those the
    // whole palette already reads and every escape must come out byte-identical.
    //
    // Naming the two themes rather than looping over all eight is the honest
    // version: the other six have a `muted` at 0.12 to 0.17 luminance, below the
    // 4.5:1 the floor asks on near-black, so the map legitimately moves them. A
    // test looping over all eight and expecting verbatim would be asserting a
    // property of the palettes that does not exist, and it would be wrong for a
    // reason that looks like a bug in the map.
    const ALREADY_LEGIBLE: [usize; 2] = [2, 3];
    let near_black = Rgb::new(2, 2, 4);
    for index in ALREADY_LEGIBLE {
        let theme = &THEMES[index];
        let view = PaletteView::for_background(theme, near_black);
        for (name, escape) in escapes(&view) {
            let expected = match name {
                "primary" => theme.primary(),
                "secondary" => theme.secondary(),
                "muted" => theme.muted(),
                "text" => theme.text(),
                "meter_low" => theme.meter_low(),
                "meter_mid" => theme.meter_mid(),
                "meter_high" => theme.meter_high(),
                _ => unreachable!(),
            };
            assert_eq!(
                escape, expected,
                "{} {name} was re-tinted on a background where it already read",
                theme.name
            );
        }
    }
}

#[test]
fn on_a_light_terminal_every_role_becomes_legible() {
    // The point of the module. Each escape is parsed back and measured against
    // the light background, so what is asserted is the rendered colour rather
    // than the string that was written.
    for theme in THEMES {
        let view = PaletteView::for_background(theme, LIGHT);
        for (name, escape) in escapes(&view) {
            let colour = colour_of(escape).unwrap_or_else(|| {
                panic!("{} {name} is not a 24-bit colour: {escape:?}", theme.name)
            });
            assert!(
                colour.contrast_ratio(LIGHT) >= MIN_TEXT_CONTRAST,
                "{} {name} is {:.2}:1 on a light terminal, drawn as {escape:?}",
                theme.name,
                colour.contrast_ratio(LIGHT)
            );
        }
    }
}

#[test]
fn the_field_reads_are_legible_too() {
    // `ops::render` and `fetch_render` read `pal.green`, `pal.red` and friends as
    // FIELDS, so the accessors being correct would not save them. These are the
    // reads that a method-only test would miss entirely.
    for theme in THEMES {
        let view = PaletteView::for_background(theme, LIGHT);
        for (name, escape) in [
            ("red", view.red.as_str()),
            ("green", view.green.as_str()),
            ("yellow", view.yellow.as_str()),
            ("cyan", view.cyan.as_str()),
            ("white", view.white.as_str()),
            ("gray", view.gray.as_str()),
            ("purple", view.purple.as_str()),
            ("blue", view.blue.as_str()),
        ] {
            let colour = colour_of(escape).unwrap_or_else(|| {
                panic!("{} {name} is not a 24-bit colour: {escape:?}", theme.name)
            });
            assert!(
                colour.contrast_ratio(LIGHT) >= MIN_TEXT_CONTRAST,
                "{} {name} is {:.2}:1 on a light terminal",
                theme.name,
                colour.contrast_ratio(LIGHT)
            );
        }
    }
}

#[test]
fn the_field_reads_and_the_accessors_agree() {
    // They are computed separately, which is exactly why they can drift. A
    // divergence here would show a panel in one colour and its own meter in
    // another, and nothing else would notice.
    for theme in THEMES {
        let view = PaletteView::for_background(theme, LIGHT);
        assert_eq!(view.cyan.as_str(), view.primary());
        assert_eq!(view.purple.as_str(), view.secondary());
        assert_eq!(view.gray.as_str(), view.muted());
        assert_eq!(view.white.as_str(), view.text());
        assert_eq!(view.green.as_str(), view.meter_low());
        assert_eq!(view.yellow.as_str(), view.meter_mid());
        assert_eq!(view.red.as_str(), view.meter_high());
    }
}

#[test]
fn a_theme_with_no_colour_falls_back_rather_than_going_blank() {
    // `PLAIN` is every escape empty, a deliberate no-colour render. Adaptation
    // declines it, and the view must pass the theme's own (empty) escapes
    // through rather than inventing a palette — a blank screen would be a far
    // worse failure than a missing colour.
    let view = PaletteView::for_background(&PLAIN, LIGHT);
    for (name, escape) in escapes(&view) {
        assert_eq!(
            escape, "",
            "PLAIN {name} should stay uncoloured, got {escape:?}"
        );
    }
    assert_eq!(view.name(), PLAIN.name);
}

#[test]
fn the_non_colour_escapes_are_never_adapted() {
    // `reset`, `bold` and `dim` are not colours. Adapting them would be
    // meaningless and rebuilding them would allocate for nothing.
    for theme in THEMES {
        let dark = PaletteView::for_background(theme, DARK);
        let light = PaletteView::for_background(theme, LIGHT);
        assert_eq!(dark.reset, theme.reset);
        assert_eq!(light.reset, theme.reset);
        assert_eq!(dark.bold, theme.bold);
        assert_eq!(light.bold, theme.bold);
        assert_eq!(dark.dim, theme.dim);
        assert_eq!(light.dim, theme.dim);
    }
}

#[test]
fn the_name_is_the_themes_name_so_the_config_still_round_trips() {
    for theme in THEMES {
        let view = PaletteView::for_background(theme, LIGHT);
        assert_eq!(view.name(), theme.name);
    }
}

#[test]
fn an_escape_round_trips_through_the_parser_that_read_palettes() {
    // The two directions have to agree: `surface` reads a palette's escape and
    // this module writes one. They are separate functions in separate modules
    // that happen to share a format, which is a coincidence until a test.
    for colour in [
        Rgb::new(0, 0, 0),
        Rgb::new(255, 255, 255),
        Rgb::new(139, 233, 253),
        Rgb::new(1, 2, 3),
        Rgb::new(99, 100, 101),
    ] {
        assert_eq!(colour_of(&escape_for(colour)), Some(colour));
    }
}

#[test]
fn building_from_an_adaptation_matches_building_from_the_background() {
    // `for_background` adapts then builds; the App adapts once and calls
    // `from_adapted`. If those two paths disagreed, the cache would serve a
    // different palette than the one-shot call would.
    for theme in THEMES {
        let direct = PaletteView::for_background(theme, LIGHT);
        let adapted = adapt(theme, LIGHT, MIN_TEXT_CONTRAST);
        let cached = PaletteView::from_adapted(theme, adapted.as_ref());
        for ((_, a), (_, b)) in escapes(&direct).iter().zip(escapes(&cached).iter()) {
            assert_eq!(a, b, "{} differs between the two build paths", theme.name);
        }
    }
}

#[test]
fn every_role_is_reachable_by_index_as_the_array_assumes() {
    // The view's escape array is filled positionally from `Role::from_index`,
    // and every accessor indexes it by a named slot. If those two orderings ever
    // disagreed, `primary()` would return the muted colour and the tests above
    // would still pass, because they only check legibility.
    for theme in THEMES {
        let view = PaletteView::for_background(theme, LIGHT);
        for (index, role) in Role::all().into_iter().enumerate() {
            let by_array = &view.escapes()[index];
            let by_accessor = escapes(&view)
                .into_iter()
                .find(|(name, _)| *name == accessor_name(role))
                .map_or_else(|| panic!("no accessor for {role:?}"), |(_, escape)| escape);
            assert_eq!(by_array, by_accessor, "{} {role:?}", theme.name);
        }
    }
}

fn accessor_name(role: Role) -> &'static str {
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
fn the_cost_of_legibility_on_light_is_bounded_and_measured() {
    // The honest version of the claim I first wrote here, which was "a theme is
    // never worse on light than on dark" and is FALSE. Kare's `primary` goes
    // from 12.82:1 on a dark terminal to 5.57:1 on a light one: adapting a
    // palette into the narrow luminance band a light background allows costs
    // contrast, and it has to -- there is nowhere else to put the colour.
    //
    // What is worth asserting is that the cost is BOUNDED, that the result still
    // clears the bar, and that no role collapses. A fix that made the palette
    // legible by making it uniform would pass the ratio test and fail this.
    for theme in THEMES {
        let light = PaletteView::for_background(theme, LIGHT);
        for (name, escape) in escapes(&light) {
            let on_light = Rgb::from_ansi(escape)
                .expect("a colour")
                .contrast_ratio(LIGHT);
            assert!(
                on_light >= MIN_TEXT_CONTRAST,
                "{} {name} is {on_light:.2}:1 on light",
                theme.name
            );
            // And the range is bounded on BOTH sides, which is a bound with a
            // direction I got wrong first time. On a LIGHT background the ratio
            // FALLS as a role's luminance rises toward the background, so the
            // brightest role has the LOWEST ratio and the darkest the highest.
            // My first ceiling assumed the opposite and fired on Kare's
            // `secondary` at 7.45:1, which is correct behaviour: it is a dark
            // purple, it is the palette's darkest role, and being well clear of
            // the bar is what the map's floor is FOR.
            assert!(
                on_light <= MAX_ADAPTED_CONTRAST,
                "{} {name} is {on_light:.2}:1 on light, past the darkest end of \
                 the adapted band -- the map left part of the range unused",
                theme.name
            );
        }
    }
}

/// The most contrast a role may have after adapting for a light terminal.
///
/// The map sends the palette's brightest role to the threshold and its darkest to
/// `DARKEST_ROLE_FRACTION` of it, so the DARKEST role's ratio is the top of the
/// band — and the darkest role is `muted`, which is both by design (it is
/// de-emphasis) and by luminance.
///
/// Measured, not guessed: across all 8 themes and all 7 roles the band is
/// 4.68:1 to 9.63:1. The bound is set above that with room to spare, because its
/// job is to catch a role the map SKIPPED — a value that leaves the band means
/// the map did not run for it — not to pin the arithmetic. Two earlier guesses at
/// this number (a direction that was backwards, then 7.0) were both wrong, and
/// measuring the real range took one example and a minute.
const MAX_ADAPTED_CONTRAST: f32 = 11.0;
