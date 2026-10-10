//! The colours for chrome drawn as ratatui cells -- the command bar, the
//! Settings panel, modals -- each adapted against the surface it sits ON.
//!
//! # The defect this shape exists to prevent
//!
//! From v0.50.0 to v0.53.0 the command bar was near-white on a dark terminal in
//! every theme. Its background was built as "the adapted TEXT colour" -- a
//! foreground role used as a fill -- and its text was ratatui NAMED colours
//! (`White`, `DarkGray`) that no adaptation ever saw: white on near-white,
//! 1.02:1 to 1.73:1. Every instrument measured foregrounds against the
//! terminal's background; the bar's own background was in no measurement.
//!
//! So a surface and the ink drawn on it are one value here. [`Chrome`] carries
//! the fill, and every role in it is adapted against THAT fill, not against
//! the terminal behind it. A caller cannot draw bar text measured against the
//! page, because the bar's text does not exist apart from the bar.

use num_traits::ToPrimitive;

use crate::adapt::{adapt, Role, MIN_TEXT_CONTRAST};
use crate::color::Palette;
use crate::surface::{bg_is_light, Rgb};

/// How far a derived bar background moves from the terminal's own toward the
/// opposite end: enough to read as a strip, little enough that text adapted
/// against it still clears the floor with room to spare.
pub const BAR_LIFT: f32 = 0.08;

/// Ink for one surface. `None` is the terminal's own default -- which is all a
/// palette that declines colour ([`crate::color::PLAIN`]) can honestly offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    /// The surface's fill. `None` for chrome that sits on the terminal's own
    /// background (a modal, the empty-filter screen) rather than painting one.
    pub bg: Option<Rgb>,
    /// Body text and the highlighted key of a binding.
    pub text: Option<Rgb>,
    /// Labels and hints: de-emphasised, still above the muted floor.
    pub muted: Option<Rgb>,
    /// The active mode, a selected row, a title.
    pub accent: Option<Rgb>,
    /// Box borders and the bar's punctuation.
    pub border: Option<Rgb>,
    /// A warning or a prompt waiting for an answer.
    pub warn: Option<Rgb>,
    /// A failure.
    pub alert: Option<Rgb>,
    /// A success, like a stored password.
    pub ok: Option<Rgb>,
    /// Text on an accent-filled pill: black or white, whichever reads.
    pub on_accent: Option<Rgb>,
}

impl Chrome {
    /// The terminal's defaults throughout: no fill, no ink.
    pub const PLAIN: Self = Self {
        bg: None,
        text: None,
        muted: None,
        accent: None,
        border: None,
        warn: None,
        alert: None,
        ok: None,
        on_accent: None,
    };

    /// Chrome that paints its own strip -- the command bar, Settings -- on a
    /// terminal painting `page`.
    #[must_use]
    pub fn bar(theme: &Palette, page: Rgb) -> Self {
        let fill = bar_background(theme, page);
        Self {
            bg: Some(fill),
            ..Self::ink(theme, fill)
        }
        .declined_if_plain(theme, page)
    }

    /// Chrome drawn straight onto the terminal's background `page`.
    #[must_use]
    pub fn page(theme: &Palette, page: Rgb) -> Self {
        Self::ink(theme, page)
    }

    /// Every role adapted against `surface`, with no fill of its own.
    fn ink(theme: &Palette, surface: Rgb) -> Self {
        let Some(adapted) = adapt(theme, surface, MIN_TEXT_CONTRAST) else {
            return Self::PLAIN;
        };
        let role = |r: Role| Some(adapted.get(r));
        let accent = adapted.get(Role::MeterLow);
        Self {
            bg: None,
            text: role(Role::Text),
            muted: role(Role::Muted),
            accent: Some(accent),
            border: role(Role::Primary),
            warn: role(Role::MeterMid),
            alert: role(Role::MeterHigh),
            ok: role(Role::MeterLow),
            on_accent: Some(readable_on(accent)),
        }
    }

    /// A palette with no colour gets no fill either: a coloured strip with
    /// terminal-default ink on it is exactly the unmeasured pairing this type
    /// exists to rule out.
    fn declined_if_plain(self, theme: &Palette, page: Rgb) -> Self {
        if adapt(theme, page, MIN_TEXT_CONTRAST).is_none() {
            Self::PLAIN
        } else {
            self
        }
    }
}

/// The command bar's fill on a terminal painting `page`.
///
/// The theme's own bar colour when it sits on the same side of light/dark as
/// the terminal -- that is the theme's design, and it reads as a strip. On the
/// other side it would be a black bar across a white screen (or the reverse),
/// so the fill is derived from the terminal's own background instead, lifted a
/// little toward the opposite end.
#[must_use]
pub fn bar_background(theme: &Palette, page: Rgb) -> Rgb {
    let own = theme.own_background();
    if bg_is_light(own) == bg_is_light(page) {
        own
    } else {
        lift(page)
    }
}

fn lift(page: Rgb) -> Rgb {
    let toward = if bg_is_light(page) { 0.0 } else { 255.0 };
    let mix = |c: u8| -> u8 {
        let c = f32::from(c);
        (toward - c)
            .mul_add(BAR_LIFT, c)
            .round()
            .to_u8()
            .unwrap_or(u8::MAX)
    };
    Rgb::new(mix(page.r), mix(page.g), mix(page.b))
}

/// Black or white, whichever has more contrast against `fill`.
#[must_use]
pub fn readable_on(fill: Rgb) -> Rgb {
    let black = Rgb::new(0, 0, 0);
    let white = Rgb::new(u8::MAX, u8::MAX, u8::MAX);
    if black.contrast_ratio(fill) >= white.contrast_ratio(fill) {
        black
    } else {
        white
    }
}

#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
