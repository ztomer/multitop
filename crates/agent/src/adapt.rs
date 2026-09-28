//! Adapting a whole palette to the background it is painted on.
//!
//! The colour arithmetic lives in [`crate::surface`]; this is the half that has
//! an opinion about a palette, and the opinion is that the PALETTE is the unit.
//!
//! Adjusting each role independently until it reaches the contrast floor is the
//! obvious implementation and it is wrong, and the test
//! `adapting_never_reduces_the_separation_two_roles_had` is the proof. Every role
//! that was failing gets driven to the SAME luminance -- the threshold -- so
//! Kare's `primary` and `secondary` came out one colour apart on a light
//! terminal. That buys legibility by spending a distinction, which is the one
//! thing the house rule about single-channel distinctions exists to prevent.
//!
//! So [`adapt`] fits ONE affine map on relative luminance across the palette's
//! own range, with the brightest role landing exactly on the threshold. Affine
//! on luminance preserves order and relative spacing, so roles that were close
//! stay close and roles that were far stay far.

use crate::color::Palette;
use crate::surface::{
    at_luminance, bg_is_light, bisect_lightness, Rgb, CONTRAST_OFFSET, MAX_ADJUSTED_LIGHTNESS,
    MIN_ADJUSTED_LIGHTNESS,
};

/// The minimum contrast ratio a text colour must reach against its background.
///
/// WCAG AA for normal text is 4.5. Terminal text is small, thin and frequently
/// read at a glance or transcribed, so the bar is the AA figure rather than the
/// 3.0 that covers "large text" in the web guidance -- a 3:1 pass is a pass in
/// a browser and a squint here.
pub const MIN_TEXT_CONTRAST: f32 = 4.5;

/// The floor for the [`Role::Muted`] role alone.
///
/// `muted` is de-emphasis by definition -- PIDs, totals, indents, the things a
/// reader should find without being drawn to -- so it is deliberately dimmer
/// than body text and asking it for [`MIN_TEXT_CONTRAST`] asks the palette to
/// stop doing its job. It still has to be readable, and this is where the
/// measurement is kept honest: see `muted_meets_its_own_floor_on_dark`, which
/// records what each theme actually reaches and which of them do not clear this.
pub const MIN_MUTED_CONTRAST: f32 = 3.0;

/// The minimum perceptual distance two roles must keep on a shared background.
///
/// A whole JND is not enough: at exactly one JND two roles are reliably
/// distinguishable one at a time and confusable side by side, which is the case
/// that matters here. Two JNDs is the usual figure for "comfortably different".
pub const MIN_ROLE_SEPARATION: f32 = 4.6;

/// Move one colour away from `bg` until it reads, keeping hue and saturation.
///
/// This is the single-colour primitive, and it is NOT how a palette is adapted
/// -- see [`adapt`] for why, which is a correction rather than a preference.
///
/// Returns `None` when the ratio cannot be reached without driving the colour to
/// [`MIN_ADJUSTED_LIGHTNESS`], which is the honest answer: a caller handed a
/// colour that still failed would be told a fix it did not get.
#[must_use]
// reachability: not yet called from production. `adapt` is the production path
// once a background is detected; this single-colour primitive exists for a caller
// that has one colour to fix and no palette to fit it to, which background
// detection will be. Kept, and kept tested, because returning `None` when a
// ratio cannot be reached is the honest answer and that branch has to stay
// exercised or the next person deletes it.
pub fn ensure_contrast(fg: Rgb, bg: Rgb, min_ratio: f32) -> Option<Rgb> {
    if fg.contrast_ratio(bg) >= min_ratio {
        return Some(fg);
    }
    let (hue, saturation, lightness) = fg.hsl();
    // The search runs AWAY from the original, because the original has already
    // been shown to fail: down in lightness on a light background, up on a dark
    // one.
    let (lo, hi) = if bg_is_light(bg) {
        (MIN_ADJUSTED_LIGHTNESS, lightness)
    } else {
        (lightness, MAX_ADJUSTED_LIGHTNESS)
    };
    let lightness = bisect_lightness(hue, saturation, lo, hi, bg, min_ratio)?;
    Some(Rgb::from_hsl(hue, saturation, lightness))
}

/// A palette's roles resolved for one background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Adapted {
    roles: [Rgb; ROLE_COUNT],
}

impl Adapted {
    /// The colour this palette draws `role` in on the background it was adapted
    /// for.
    ///
    /// Not an `Option`, and that is the improvement over
    /// [`ensure_contrast`]: an affine map over a luminance range has no failure
    /// mode. Every role gets a colour, and the guarantee is not "it might be
    /// legible" but "the palette as a whole meets the ratio" -- which is the
    /// claim [`Adapted`] can actually support, and which a per-colour `None`
    /// could only ever have weakened.
    #[must_use]
    pub const fn get(&self, role: Role) -> Rgb {
        self.roles[role.index()]
    }

    /// The worst contrast ratio of any role against `bg`.
    ///
    /// The single number a caller can check, because "every role passes" is a
    /// property of the palette and not of any one colour.
    #[must_use]
    // reachability: not yet called from production. This is the single number a
    // caller checks after adapting, to assert the palette as a whole is legible
    // rather than asserting each role in turn; the caller is the same wiring
    // step.
    pub fn worst_contrast(&self, bg: Rgb) -> f32 {
        let mut worst = f32::INFINITY;
        for c in self.roles {
            worst = worst.min(c.contrast_ratio(bg));
        }
        worst
    }

    /// Every role, in declaration order.
    #[must_use]
    pub const fn roles(&self) -> &[Rgb; ROLE_COUNT] {
        &self.roles
    }
}

impl Role {
    const fn index(self) -> usize {
        match self {
            Self::Primary => slot::PRIMARY,
            Self::Secondary => slot::SECONDARY,
            Self::Muted => slot::MUTED,
            Self::Text => slot::TEXT,
            Self::MeterLow => slot::METER_LOW,
            Self::MeterMid => slot::METER_MID,
            Self::MeterHigh => slot::METER_HIGH,
        }
    }
}

/// Adapt a whole palette to the background it is being painted on.
///
/// This is the function the roadmap asks for, and the reason it is a palette
/// and not seven colours is a measured failure of the seven-colour version.
/// Adjusting each role independently until it meets the ratio drives every role
/// that was failing to the SAME luminance -- the threshold -- so Kare's `primary`
/// and `secondary` came out 1.01:1 apart on a light terminal, having become the
/// same colour. That buys legibility by spending a distinction, which is the one
/// thing the house rule about single-channel distinctions exists to prevent.
///
/// So: one affine map on relative luminance, fitted once across the palette's
/// own range, with the brightest role landing exactly on the ratio threshold.
/// Affine on luminance preserves ORDER and relative spacing, so two roles that
/// were close stay close and two that were far stay far -- the palette's
/// structure survives the adaptation, which is the property per-colour
/// adjustment destroys.
///
/// The dark end is floored at [`DARKEST_ROLE_FRACTION`] of the threshold rather
/// than scaled proportionally, because the palette's luminances span nearly a
/// whole stop and a proportional scale would crush the already-dark roles into
/// black and lose their hues.
#[must_use]
pub fn adapt(theme: &Palette, bg: Rgb, min_ratio: f32) -> Option<Adapted> {
    let roles = Role::all();
    // A palette adapts whole or not at all. A role whose escape is not a 24-bit
    // colour is the `PLAIN` case -- a deliberate no-colour render -- and asking
    // for an `Adapted` from it has no answer, so this returns `None` rather than
    // inventing one.
    let mut originals = [Rgb::new(0, 0, 0); ROLE_COUNT];
    let mut index = 0;
    while index < ROLE_COUNT {
        originals[index] = theme.role_rgb(roles[index])?;
        index += 1;
    }

    // The luminance a colour must be AT OR BELOW (light background) or AT OR
    // ABOVE (dark background) to reach the ratio. Closed form, no search.
    let bg_l = bg.relative_luminance();
    let min_ratio = min_ratio * RATIO_MARGIN;
    let threshold = if bg_is_light(bg) {
        (bg_l + CONTRAST_OFFSET) / min_ratio - CONTRAST_OFFSET
    } else {
        min_ratio.mul_add(bg_l + CONTRAST_OFFSET, -CONTRAST_OFFSET)
    };
    let light = bg_is_light(bg);

    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for c in originals {
        let l = c.relative_luminance();
        lo = lo.min(l);
        hi = hi.max(l);
    }

    // Where the adapted range sits. The constrained end is the one that has to
    // reach the threshold; the other end keeps its own value if it already
    // clears the floor, so a role that reads is not moved for company.
    let (adapted_lo, adapted_hi) = if light {
        (lo.min(threshold * DARKEST_ROLE_FRACTION), threshold)
    } else {
        (threshold, hi.max(threshold / DARKEST_ROLE_FRACTION))
    };
    let span = (hi - lo).max(f32::EPSILON);

    let mut out = [Rgb::new(0, 0, 0); ROLE_COUNT];
    let mut index = 0;
    while index < ROLE_COUNT {
        let position = (originals[index].relative_luminance() - lo) / span;
        let target = position.mul_add(adapted_hi - adapted_lo, adapted_lo);
        out[index] = at_luminance(originals[index], target);
        index += 1;
    }
    Some(Adapted { roles: out })
}

/// Head-room on the ratio, for 8-bit quantisation.
///
/// A target luminance is solved in floating point and then rendered as three
/// bytes, and the rounding lands a little under it: a palette mapped to exactly
/// 4.50 came out at 4.47, which fails the very check it was built to pass.
/// Rather than loosen the floor -- which would let a palette sit under it forever
/// -- the solve aims this far above it. One 8-bit step of luminance is about
/// 0.0015, so this is generous rather than tuned, and it costs nothing visible.
const RATIO_MARGIN: f32 = 1.05;

/// How much of the luminance range below the threshold the darkest adapted role
/// is allowed to occupy, on a light background.
///
/// A palette's luminances span a wide range, and the floor exists so the
/// darkest role lands somewhere with a visible hue rather than at black. At a
/// third, a two-hue palette still separates: the brightest role at the threshold
/// and the darkest at a third of it are 2:1 apart in luminance alone, before hue.
pub const DARKEST_ROLE_FRACTION: f32 = 0.34;

/// Where each role sits in the arrays that hold a palette's colours.
///
/// Named because the correspondence between a role and its slot is the one thing
/// that has to agree between [`Role::index`], [`Role::all`] and every
/// `[Rgb; ROLE_COUNT]` in this file. As bare numbers it is a correspondence
/// nobody can grep for, and reordering the `match` would silently re-label every
/// colour in the palette.
mod slot {
    pub const PRIMARY: usize = 0;
    pub const SECONDARY: usize = 1;
    pub const MUTED: usize = 2;
    pub const TEXT: usize = 3;
    pub const METER_LOW: usize = 4;
    pub const METER_MID: usize = 5;
    pub const METER_HIGH: usize = 6;
    /// How many roles there are, which is how long every array over them is.
    pub const COUNT: usize = METER_HIGH + 1;
}

use slot::COUNT as ROLE_COUNT;

/// One of the roles a palette answers for.
///
/// Named so a caller asks for meaning rather than for a colour, which is what
/// makes the surface fix a one-mechanism change rather than sixteen palettes:
/// the role is stable, and the colour behind it is what the background decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Headers, main metrics, titles -- the cyan every host banner is drawn in.
    Primary,
    /// Rules, subtitles, secondary metrics.
    Secondary,
    /// PIDs, totals, indents.
    Muted,
    /// Process and container names, primary values.
    Text,
    /// A meter reading low.
    MeterLow,
    /// A meter reading medium -- yellow.
    MeterMid,
    /// A meter reading high, and the alert tint.
    MeterHigh,
}

impl Role {
    /// The palette escape for this role, read straight from the theme.
    #[must_use]
    pub const fn escape(self, theme: &Palette) -> &'static str {
        match self {
            Self::Primary => theme.primary(),
            Self::Secondary => theme.secondary(),
            Self::Muted => theme.muted(),
            Self::Text => theme.text(),
            Self::MeterLow => theme.meter_low(),
            Self::MeterMid => theme.meter_mid(),
            Self::MeterHigh => theme.meter_high(),
        }
    }

    /// Every role, for the checks that must hold for all of them.
    #[must_use]
    pub const fn all() -> [Self; ROLE_COUNT] {
        [
            Self::Primary,
            Self::Secondary,
            Self::Muted,
            Self::Text,
            Self::MeterLow,
            Self::MeterMid,
            Self::MeterHigh,
        ]
    }
}

impl Palette {
    /// The colour this theme draws this role in, as components.
    ///
    /// `None` for a palette that does not give the role a 24-bit colour --
    /// [`crate::color::PLAIN`], which is a deliberate no-colour render.
    #[must_use]
    pub const fn role_rgb(&self, role: Role) -> Option<Rgb> {
        Rgb::from_ansi(role.escape(self))
    }

    /// The one background this theme commits to, as components.
    ///
    /// A theme paints a background for the keybar strip and NOTHING else: every
    /// other cell is left to whatever the terminal itself paints, which is the
    /// whole reason a light terminal breaks these palettes. So this is the only
    /// statement any theme makes about what is behind it, which makes it the
    /// honest per-theme backdrop for asking whether a palette reads -- and the
    /// first thing detection will have to make real.
    #[must_use]
    // reachability: not yet called from production. The wiring step reads this
    // as the fallback when background detection is unavailable, which is the only
    // case where a theme's own keybar colour is the honest answer.
    pub const fn own_background(&self) -> Rgb {
        Rgb::new(
            self.ratatui_keybar_bg.0,
            self.ratatui_keybar_bg.1,
            self.ratatui_keybar_bg.2,
        )
    }
}

#[cfg(test)]
#[path = "adapt_tests.rs"]
mod tests;
