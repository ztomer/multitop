//! Chrome, measured against the surface it is drawn on.

use super::*;
use crate::adapt::{MIN_MUTED_CONTRAST, MIN_TEXT_CONTRAST};
use crate::color::{PLAIN, THEMES};

/// A spread of real terminal backgrounds, both sides of light/dark.
const PAGES: &[Rgb] = &[
    Rgb::new(0, 0, 0),
    Rgb::new(30, 30, 30),
    Rgb::new(40, 42, 54),
    Rgb::new(253, 246, 227),
    Rgb::new(238, 238, 238),
    Rgb::new(255, 255, 255),
];

fn inks(c: &Chrome) -> [(&'static str, Option<Rgb>, f32); 7] {
    [
        ("text", c.text, MIN_TEXT_CONTRAST),
        ("muted", c.muted, MIN_MUTED_CONTRAST),
        ("accent", c.accent, MIN_TEXT_CONTRAST),
        ("border", c.border, MIN_TEXT_CONTRAST),
        ("warn", c.warn, MIN_TEXT_CONTRAST),
        ("alert", c.alert, MIN_TEXT_CONTRAST),
        ("ok", c.ok, MIN_TEXT_CONTRAST),
    ]
}

#[test]
fn the_bar_sits_on_the_terminals_side_of_light_and_dark() {
    // The defect: the fill was the adapted TEXT colour, so on a dark terminal
    // the bar was near-white. A fill on the wrong side of the page is that
    // defect whatever the ink on it measures.
    for theme in THEMES {
        for &page in PAGES {
            let fill = Chrome::bar(theme, page)
                .bg
                .expect("a themed bar has a fill");
            assert_eq!(
                bg_is_light(fill),
                bg_is_light(page),
                "{}: bar {fill:?} on page {page:?}",
                theme.name
            );
        }
    }
}

#[test]
fn every_ink_reads_against_its_own_surface() {
    for theme in THEMES {
        for &page in PAGES {
            for (surface, chrome) in [
                ("bar", Chrome::bar(theme, page)),
                ("page", Chrome::page(theme, page)),
            ] {
                let behind = chrome.bg.unwrap_or(page);
                for (name, ink, floor) in inks(&chrome) {
                    let ink = ink.expect("a themed chrome has every ink");
                    let ratio = ink.contrast_ratio(behind);
                    assert!(
                        ratio >= floor,
                        "{} {surface} {name}: {ink:?} on {behind:?} is {ratio:.2}:1, under {floor}",
                        theme.name
                    );
                }
                let accent = chrome.accent.expect("accent");
                let on = chrome.on_accent.expect("on_accent");
                assert!(
                    on.contrast_ratio(accent) >= MIN_TEXT_CONTRAST,
                    "{} {surface}: the active-mode pill reads {:.2}:1",
                    theme.name,
                    on.contrast_ratio(accent)
                );
            }
        }
    }
}

#[test]
fn the_themes_own_bar_is_kept_where_it_already_belongs() {
    // A dark theme on a dark terminal keeps its designed strip; only the
    // wrong side gets a derived one.
    for theme in THEMES {
        let own = theme.own_background();
        let same_side = if bg_is_light(own) {
            Rgb::new(255, 255, 255)
        } else {
            Rgb::new(0, 0, 0)
        };
        assert_eq!(
            Chrome::bar(theme, same_side).bg,
            Some(own),
            "{}",
            theme.name
        );
    }
}

#[test]
fn a_palette_without_colour_draws_terminal_defaults_and_no_fill() {
    for &page in PAGES {
        assert_eq!(Chrome::bar(&PLAIN, page), Chrome::PLAIN);
        assert_eq!(Chrome::page(&PLAIN, page), Chrome::PLAIN);
    }
}
