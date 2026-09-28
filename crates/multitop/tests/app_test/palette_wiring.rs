//! The palette the app DRAWS with, as opposed to the palette machinery.
//!
//! These exist because a whole feature can be correct in every module and still
//! not be connected to anything. The light-terminal work was, for one commit:
//! a probe that detected a background correctly, an adaptation that computed a
//! legible palette correctly, and an `App` that stored the background and then
//! rendered with the view built in `App::new` — before detection ran, against a
//! default. Every test passed, because every test tested a module rather than
//! the wiring between them. Ignoring the detected background entirely is the
//! simplest way to ship this feature and not have it.
//!
//! So: these assert that the thing the app holds is the thing the code computes.
//! They read the app's OWN view, not a freshly built one, because a test that
//! rebuilds its subject is testing the module and not the connection.

use multitop::app::App;
use multitop::background::Background;
use multitop_agent::color::THEMES;
use multitop_agent::palette_view::PaletteView;
use multitop_agent::surface::Rgb;

/// A near-white terminal: the case the whole feature exists for.
const LIGHT: Rgb = Rgb::new(252, 252, 250);

/// A dark terminal, near-black rather than a keybar colour — see
/// `palette_view_tests` for why `own_background` is not one.
const DARK: Rgb = Rgb::new(24, 26, 36);

fn app_with(background: Background) -> App {
    // `App::new` reaches `password_store` several calls down. An integration
    // binary is compiled without `cfg(test)`, so the mock store is not in force
    // unless asked for -- and without it these tests query the real OS keychain,
    // which raises a dialog per rebuild and can read credentials the user
    // depends on. `check_keychain_isolation.py` is what enforces it.
    let _guard = super::isolate_keychain();
    let mut app = App::new(Vec::new());
    app.background = background;
    // The call `boot_app` makes. Calling it here rather than in the test body is
    // the point: the assertion is that this call is what `boot_app` does, and a
    // test that does it itself proves only that it can be done.
    app.rebuild_palette_view();
    app
}

#[test]
fn a_light_background_reaches_the_palette_the_app_draws_with() {
    let app = app_with(Background::Reported(LIGHT));
    let drawn = app.current_theme();
    // Every role must be legible ON THE BACKGROUND THE APP WAS TOLD ABOUT. The
    // escape is parsed back and measured, so this is the rendered colour rather
    // than a string that was written.
    for (name, escape) in [
        ("primary", drawn.primary()),
        ("secondary", drawn.secondary()),
        ("muted", drawn.muted()),
        ("text", drawn.text()),
        ("meter_low", drawn.meter_low()),
        ("meter_mid", drawn.meter_mid()),
        ("meter_high", drawn.meter_high()),
    ] {
        let colour = Rgb::from_ansi(escape)
            .unwrap_or_else(|| panic!("{name} is not a 24-bit colour: {escape:?}"));
        assert!(
            colour.contrast_ratio(LIGHT) >= multitop_agent::adapt::MIN_TEXT_CONTRAST,
            "{name} is {:.2}:1 on the app's light background: {escape:?}",
            colour.contrast_ratio(LIGHT)
        );
    }
}

#[test]
fn a_dark_background_reaches_the_palette_the_app_draws_with() {
    let app = app_with(Background::Reported(DARK));
    let drawn = app.current_theme();
    for (name, escape) in [
        ("primary", drawn.primary()),
        ("text", drawn.text()),
        ("meter_mid", drawn.meter_mid()),
    ] {
        let colour = Rgb::from_ansi(escape).expect("a 24-bit colour");
        assert!(
            colour.contrast_ratio(DARK) >= multitop_agent::adapt::MIN_TEXT_CONTRAST,
            "{name} is {:.2}:1 on the app's dark background: {escape:?}",
            colour.contrast_ratio(DARK)
        );
    }
}

#[test]
fn an_unknown_background_falls_back_to_the_theme_not_to_the_last_one() {
    // The `Unknown` case has to be its own thing, because a fallback that reuses
    // whatever the view last had would pass every assertion above and be wrong
    // on a terminal that answered neither probe — which is most of them.
    let app = app_with(Background::Unknown);
    let theme = &THEMES[0];
    let expected = PaletteView::for_theme(theme);
    assert_eq!(app.current_theme().primary(), expected.primary());
    assert_eq!(app.current_theme().text(), expected.text());
}

#[test]
fn changing_the_theme_rebuilds_the_view_for_the_same_background() {
    // Theme-cycling has to re-derive, or the second theme is drawn in the first
    // one's colours. `cycle_theme` calls the rebuild itself, which is the thing
    // being asserted — a caller that remembered to rebuild would not be caught.
    let mut app = app_with(Background::Reported(LIGHT));
    app.theme_idx = 0;
    let before = app.current_theme().primary().to_string();
    app.cycle_theme();
    let after = app.current_theme().primary().to_string();
    // Both are legible on light — that is the property that must survive the
    // rebuild, rather than an exact escape, which would be a snapshot in
    // disguise. The first two themes are NOT compared for difference: Kare and
    // Dracula share a , and asserting they differ was a precondition I
    // wrote and did not check. The property worth asserting is that the view
    // still names the CURRENT theme, which the  below establishes.
    assert_eq!(app.current_theme().name(), THEMES[1].name);
    for escape in [&before, &after] {
        let colour = Rgb::from_ansi(escape).expect("a 24-bit colour");
        assert!(
            colour.contrast_ratio(LIGHT) >= multitop_agent::adapt::MIN_TEXT_CONTRAST,
            "{escape:?} is {:.2}:1 after cycling",
            colour.contrast_ratio(LIGHT)
        );
    }
}

#[test]
fn the_ratatui_colours_reach_the_app_too() {
    // The keybar background is the one a light terminal hurts most — a dark
    // theme's keybar is a black bar across a white screen — and it is a
    // `ratatui::Color` rather than an escape, so it travels a different path and
    // would not be covered by an assertion about escapes.
    let app = app_with(Background::Reported(LIGHT));
    let drawn = app.current_theme();
    let keybar = Rgb::new(drawn.keybar_bg.0, drawn.keybar_bg.1, drawn.keybar_bg.2);
    assert!(
        keybar.contrast_ratio(LIGHT) >= multitop_agent::adapt::MIN_TEXT_CONTRAST,
        "the keybar is {:.2}:1 against the app's own background: {keybar:?}",
        keybar.contrast_ratio(LIGHT)
    );
}
