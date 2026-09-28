//! Legibility against the background the terminal is actually painting.
//!
//! Every one of the eight themes in [`crate::color`] is a dark theme, and the
//! palette is applied as literal ANSI escapes embedded in the text, so the
//! colours are chosen here and interpreted by the terminal. Nothing in that
//! chain knows what is behind the text. On a light terminal the result is that
//! a meaning that was carried by colour is not carried at all: yellow -- the
//! `⚠` glyph, the warn state, the medium meter -- and cyan, which every host
//! banner is drawn in, both wash out to near-invisible. The glyph shape and the
//! words still say what is happening, so nothing is *wrong*; it is simply
//! unreadable, which is a legibility bug rather than a style preference.
//!
//! This module is the part of the fix that does not need a terminal: it turns
//! "this colour, on that background" into a colour that can be read. Detecting
//! the background is a separate concern, because the agent renders on the
//! REMOTE host while its escapes are interpreted by the LOCAL terminal, and so
//! the remote side cannot answer the question for itself.
//!
//! # Why WCAG's numbers
//!
//! The ratios are WCAG 2.1 relative luminance and contrast ratio. That is not
//! decoration: a terminal is a low-luminance, small-text, often-transcribed
//! medium where the usual web guidance is already at its weakest, and picking
//! thresholds by eye is how "it looked fine on my terminal" became eight dark
//! palettes. The ratio is measurable, so the claim in the roadmap can be a test
//! rather than an opinion.
//!
//! # Why hue is preserved
//!
//! The house rule is that no distinction may ride on one channel. If legibility
//! were bought by converting every failing colour to grey, or to black, then
//! two roles that were told apart by colour would become identical and the
//! adjustment would have *removed* a distinction to restore another. So the
//! adjustment moves lightness in HSL and leaves hue and saturation alone: a
//! yellow stays yellow, a cyan stays cyan, and two roles that were apart stay
//! apart. [`ensure_contrast`] guarantees the ratio; `tests` guarantees the
//! distinctness, because that is the property an eyeball check would miss.

pub const CONTRAST_OFFSET: f32 = 0.05;

/// Whether a background counts as light, which decides which way a colour has to
/// move to be legible on it.
///
/// The luminance at which a background stops counting as light: WCAG's own
/// dividing line is 0.1791, and this is rounded up to the nearest tenth so the
/// branch reads plainly in a debugger and lands in the same place.
/// The luminance at which a background stops counting as light.
///
/// WCAG's own dividing line is 0.1791; this is rounded up to the nearest tenth
/// so the branch reads plainly in a debugger and lands in the same place.
#[must_use]
pub fn bg_is_light(background: Rgb) -> bool {
    background.relative_luminance() > LIGHT_BACKGROUND_LUMINANCE
}

const LIGHT_BACKGROUND_LUMINANCE: f32 = 0.18;

pub const MIN_ADJUSTED_LIGHTNESS: f32 = 0.16;
pub const MAX_ADJUSTED_LIGHTNESS: f32 = 0.92;

pub const LIGHTNESS_STEPS: u32 = 20;

pub const LUMINANCE_STEPS: u32 = 18;

/// How close a rebuilt colour must be to the luminance asked for before the
/// search stops, in relative luminance.
pub const LUMINANCE_TOLERANCE: f32 = 0.0005;

/// How many steps each bisection takes, and why those numbers.
///
/// Over lightness, twenty is well past the point where 8-bit quantisation stops
/// changing the answer. To a target luminance, eighteen over the full lightness
/// range is finer than one 8-bit step, so the result is quantisation-limited
/// rather than search-limited. Both are fixed counts so the cost does not depend
/// on the input.
///
/// The constant block the colour maths reads from. Every literal that decides a
/// channel value lives here, so `check_magic_numbers.py` has nothing to guess at
/// and a change to one of them is a diff against a name rather than against a
/// number that also appears in a test.
pub mod units {
    /// The largest value one 8-bit channel holds, and the divisor that turns a
    /// byte into the 0..=1 the WCAG definitions are written in.
    pub const CHANNEL_MAX: f32 = 255.0;
    /// The sRGB gamma knee and its two coefficients, from the WCAG definition.
    pub const GAMMA_KNEE: f32 = 0.040_45;
    pub const GAMMA_DIVISOR: f32 = 12.92;
    pub const GAMMA_OFFSET: f32 = 0.055;
    pub const GAMMA_SCALE: f32 = 1.055;
    pub const GAMMA_EXPONENT: f32 = 2.4;
    /// Luminance weights, which are the WCAG coefficients and not free choices:
    /// the eye is far more sensitive to green than to blue.
    pub const RED_LUMA: f32 = 0.2126;
    pub const GREEN_LUMA: f32 = 0.7152;
    pub const BLUE_LUMA: f32 = 0.0722;
    /// HSL divides the hue circle into six sectors, 60 degrees each, and the
    /// sector each primary's colour falls in is what selects the RGB pair.
    pub const HSL_SECTORS: f32 = 6.0;
    /// The same count as an integer, for selecting a sector with a `match`.
    pub const HSL_SECTOR_COUNT: u32 = 6;
    pub const HSL_DEGREES_PER_SECTOR: f32 = 60.0;
    /// The sector offsets in that selection: green is the second sector, blue
    /// the third, and each is reached by adding whole sectors to a ratio that
    /// runs -1..=1.
    pub const HSL_SECTOR_OFFSET: f32 = 2.0;
    pub const HSL_SECTOR_OFFSET_TWO: f32 = 4.0;
    /// A full turn, for `rem_euclid` on a hue that may have come in negative.
    pub const FULL_TURN: f32 = 360.0;
    /// Above this lightness an HSL colour is on its light half, where
    /// saturation is scaled by the distance to white rather than to black.
    pub const HALF: f32 = 0.5;
    /// How close two channels have to be to count as the same one.
    pub const CHANNEL_TIE_EPSILON: f32 = 1e-6;
    /// A hue is meaningless without chroma, so a grey reports this rather than
    /// a hue that depends on which comparison happened to be equal first.
    pub const GREY_HUE: f32 = 0.0;
    /// Two channels this close are the same channel, and dividing by the span
    /// below would amplify quantisation into a visible hue.
    pub const CHROMA_EPSILON: f32 = 1e-6;
    /// The most digits a colour component can have, being three.
    pub const MAX_COMPONENT_DIGITS: usize = 3;
    /// Decimal, so accumulating a component is a multiply by this and an add.
    pub const DECIMAL_RADIX: u8 = 10;
}

use num_traits::ToPrimitive;

use units::{
    BLUE_LUMA, CHANNEL_MAX, CHANNEL_TIE_EPSILON, CHROMA_EPSILON, DECIMAL_RADIX, FULL_TURN,
    GAMMA_DIVISOR, GAMMA_EXPONENT, GAMMA_KNEE, GAMMA_OFFSET, GAMMA_SCALE, GREEN_LUMA, GREY_HUE,
    HALF, HSL_DEGREES_PER_SECTOR, HSL_SECTORS, HSL_SECTOR_COUNT, HSL_SECTOR_OFFSET,
    HSL_SECTOR_OFFSET_TWO, MAX_COMPONENT_DIGITS, RED_LUMA,
};

/// A 24-bit colour, in the form the terminal will be told to paint.
/// A 24-bit colour, in the form the terminal will be told to paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Read the colour out of a 24-bit ANSI escape: `ESC [ 38 ; 2 ; R ; G ; B m`.
    ///
    /// A `const fn` on purpose. The palette stores escape STRINGS and nothing
    /// else, so deriving the components from them is the only way to reach the
    /// colour without adding a second source of truth -- eight more fields per
    /// theme, which is sixteen palettes' worth of drift waiting to happen.
    ///
    /// It returns `Option` rather than panicking, and that is not a retreat from
    /// the compile-error guarantee it was first written with. `clippy::panic` is
    /// on in this repository and `#[allow]` is forbidden by policy, so a panic
    /// here is not available -- and the guarantee survives anyway, one level up:
    /// [`crate::surface::adapt`] returns `None` if any role has no colour, and
    /// `every_theme_adapts` names the theme and role that broke. A palette with a
    /// hole in it is also a legitimate state rather than corruption --
    /// [`crate::color::PLAIN`] is exactly that, every escape empty, which is what
    /// a no-colour render asks for.
    #[must_use]
    pub const fn from_ansi(escape: &str) -> Option<Self> {
        let bytes = escape.as_bytes();
        let mut start = 0;
        while start + MARKER.len() <= bytes.len() {
            let at_marker = bytes[start] == MARKER[0]
                && bytes[start + 1] == MARKER[1]
                && bytes[start + 2] == MARKER[2]
                && bytes[start + 3] == MARKER[3]
                && bytes[start + 4] == MARKER[4];
            if at_marker {
                // `?` is not available in a const fn -- `Try` is not a const
                // trait -- so each step is matched by hand.
                return match component(bytes, start + MARKER.len()) {
                    None => None,
                    Some((after_r, red)) => match component(bytes, after_r) {
                        None => None,
                        Some((after_g, green)) => match component(bytes, after_g) {
                            None => None,
                            Some((after_b, blue)) => {
                                // A fourth component means this was not the
                                // colour escape it looked like; taking the
                                // first three would be a guess.
                                if after_b >= bytes.len() || bytes[after_b] == b'm' {
                                    Some(Self::new(red, green, blue))
                                } else {
                                    None
                                }
                            }
                        },
                    },
                };
            }
            start += 1;
        }
        None
    }

    /// WCAG 2.1 relative luminance, 0.0 (black) to 1.0 (white).
    ///
    /// Not `const`, and cannot be: `f32::powf` is not a const fn. That is why
    /// [`Rgb::from_ansi`] is `const` and this is not -- the parse has to happen
    /// during const evaluation to turn a malformed palette entry into a compile
    /// error, while the arithmetic only ever runs at render time.
    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        let red = lin(self.r);
        let green = lin(self.g);
        let blue = lin(self.b);
        // mul_add rather than a*b + c: both faster and correctly rounded, and a
        // colour comparison is exactly where a last-bit error becomes a flake.
        red.mul_add(RED_LUMA, green.mul_add(GREEN_LUMA, blue * BLUE_LUMA))
    }

    /// WCAG 2.1 contrast ratio, 1.0 (identical) to 21.0 (black on white).
    #[must_use]
    pub fn contrast_ratio(self, other: Self) -> f32 {
        let a = self.relative_luminance();
        let b = other.relative_luminance();
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The HSL form. Hue in degrees, saturation and lightness in 0..=1.
    ///
    /// Named constants rather than bare numbers because the six sector
    /// boundaries are what the whole function is: an off-by-one in one of them
    /// is a colour that keeps its red channel where it should have kept its
    /// blue, and the result is a plausible wrong answer rather than a crash.
    #[must_use]
    pub fn hsl(self) -> (f32, f32, f32) {
        let red = f32::from(self.r) / CHANNEL_MAX;
        let green = f32::from(self.g) / CHANNEL_MAX;
        let blue = f32::from(self.b) / CHANNEL_MAX;
        let highest = red.max(green).max(blue);
        let lowest = red.min(green).min(blue);
        let lightness = highest.midpoint(lowest);
        let span = highest - lowest;
        if span <= CHROMA_EPSILON {
            return (GREY_HUE, 0.0, lightness);
        }
        let saturation = if lightness > HALF {
            span / (2.0 - highest - lowest)
        } else {
            span / (highest + lowest)
        };
        // Which channel IS the maximum is decided by a margin rather than by
        // equality, because the two are the same number here and only by
        // construction -- the compiler is not told, and an `==` on a float in
        // colour code is the kind of thing that breaks under a fast-math
        // rewrite long after anyone remembers writing it.
        let sixths = if (highest - red).abs() < CHANNEL_TIE_EPSILON {
            ((green - blue) / span).rem_euclid(HSL_SECTORS)
        } else if (highest - green).abs() < CHANNEL_TIE_EPSILON {
            (blue - red) / span + HSL_SECTOR_OFFSET
        } else {
            (red - green) / span + HSL_SECTOR_OFFSET_TWO
        };
        (sixths * HSL_DEGREES_PER_SECTOR, saturation, lightness)
    }

    /// The inverse of [`Rgb::hsl`].
    #[must_use]
    pub fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> Self {
        // |2L - 1| is the distance from the halfway point of the range, and one
        // minus it is the chroma ceiling at that lightness: 1 at black or white,
        // 0 at the midpoint, which is why the same formula serves both ends.
        let offset = 2.0f32.mul_add(lightness, -1.0).abs();
        let chroma = offset.mul_add(-1.0, 1.0) * saturation;
        let position = hue.rem_euclid(FULL_TURN) / HSL_DEGREES_PER_SECTOR;
        let rise = chroma * (1.0 - (position % 2.0 - 1.0).abs());
        let sector = position.to_u32().unwrap_or(0) % HSL_SECTOR_COUNT;
        let (red_1, green_1, blue_1) = match sector {
            0 => (chroma, rise, 0.0),
            1 => (rise, chroma, 0.0),
            2 => (0.0, chroma, rise),
            3 => (0.0, rise, chroma),
            4 => (rise, 0.0, chroma),
            _ => (chroma, 0.0, rise),
        };
        let lift = lightness - chroma / 2.0;
        Self::new(
            to_byte(red_1 + lift),
            to_byte(green_1 + lift),
            to_byte(blue_1 + lift),
        )
    }
}

/// sRGB gamma expansion, the `c <= 0.04045 ? c/12.92 : ((c+0.055)/1.055)^2.4`
/// of the WCAG definition.
#[must_use]
pub fn lin(channel: u8) -> f32 {
    let value = f32::from(channel) / CHANNEL_MAX;
    if value <= GAMMA_KNEE {
        value / GAMMA_DIVISOR
    } else {
        ((value + GAMMA_OFFSET) / GAMMA_SCALE).powf(GAMMA_EXPONENT)
    }
}

/// A normalised channel (0..=1) to a byte, saturating.
///
/// The float-to-int hop goes through `num_traits`, which is what the rest of
/// this crate does (`conv.rs`) and which returns an `Option` rather than
/// defining behaviour for a NaN. The documented fallback is zero: a NaN can
/// only reach here if a caller passed one, and a black pixel is a visible
/// failure to a test rather than a wrapped 255 that looks deliberate.
fn to_byte(channel: f32) -> u8 {
    if !channel.is_finite() {
        return 0;
    }
    let scaled = (channel * CHANNEL_MAX).round().clamp(0.0, CHANNEL_MAX);
    let whole = scaled.to_u32().unwrap_or(0);
    u8::try_from(whole).unwrap_or(0)
}

/// The five bytes that introduce a 24-bit colour: `38;2;`.
const MARKER: &[u8; 5] = b"38;2;";

/// Read one decimal colour component starting at `at`.
///
/// Returns the value and the index just past the terminator, which is `;` for
/// the first two components and `m` for the last. Reading only `;` is how the
/// first version of this parser rejected every escape in the palette: the final
/// component is closed by the letter that ends the sequence.
///
/// `None` for anything that is not one to three digits -- including an empty
/// component, a non-digit, and a value that would not fit a byte. Three digits
/// is the ceiling because a byte is, and 255 is three digits; `999` is rejected
/// here rather than wrapped into 231 somewhere downstream.
const fn component(bytes: &[u8], at: usize) -> Option<(usize, u8)> {
    // Accumulated in u8 with CHECKED arithmetic, which is what removes the cast
    // and rejects an out-of-range value in the same breath: `999` overflows on
    // the third digit and is refused, rather than truncating to 231 and being
    // drawn as a plausible wrong colour.
    let mut value: u8 = 0;
    let mut digits = 0;
    let mut index = at;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b';' || byte == b'm' {
            return if digits == 0 {
                None
            } else {
                Some((index + 1, value))
            };
        }
        if !byte.is_ascii_digit() || digits == MAX_COMPONENT_DIGITS {
            return None;
        }
        value = match value.checked_mul(DECIMAL_RADIX) {
            None => return None,
            Some(shifted) => match shifted.checked_add(byte - b'0') {
                None => return None,
                Some(sum) => sum,
            },
        };
        digits += 1;
        index += 1;
    }
    None
}

/// Bisect for the lightness nearest the original that meets `min_ratio`.
///
/// Prefers the smallest adjustment: on a light background that is the lightest
/// passing colour, on a dark one the darkest. `lo` and `hi` bound the direction
/// worth searching, so the search never walks away from a colour that was
/// already close.
#[must_use]
pub fn bisect_lightness(
    hue: f32,
    saturation: f32,
    lo: f32,
    hi: f32,
    bg: Rgb,
    min_ratio: f32,
) -> Option<f32> {
    if lo > hi {
        return None;
    }
    let light = bg_is_light(bg);
    let (mut lo, mut hi) = (lo, hi);
    let mut best = None;
    for _ in 0..LIGHTNESS_STEPS {
        let mid = lo.midpoint(hi);
        let candidate = Rgb::from_hsl(hue, saturation, mid);
        if candidate.contrast_ratio(bg) >= min_ratio {
            best = Some(mid);
            // Passes: look for a smaller adjustment, i.e. back toward the
            // original, which is the other end of the interval.
            if light {
                lo = mid;
            } else {
                hi = mid;
            }
        } else if light {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    best
}

/// Rebuild `rgb` at a given WCAG relative luminance, keeping hue and saturation.
///
/// Bisected on HSL lightness, which is monotone in luminance for a fixed hue and
/// saturation, so the search is well-posed. This is the primitive the
/// palette-level map in [`crate::adapt`] is built from, and it is what lets that
/// map be an affine transform of luminance rather than a per-colour search.
#[must_use]
pub fn at_luminance(rgb: Rgb, target: f32) -> Rgb {
    let current = rgb.relative_luminance();
    if (current - target).abs() < LUMINANCE_TOLERANCE {
        return rgb;
    }
    let (hue, saturation, lightness) = rgb.hsl();
    let (mut low, mut high) = if target < current {
        (0.0, lightness)
    } else {
        (lightness, 1.0)
    };
    for _ in 0..LUMINANCE_STEPS {
        let middle = low.midpoint(high);
        let candidate = Rgb::from_hsl(hue, saturation, middle);
        let reached = candidate.relative_luminance();
        if (reached - target).abs() < LUMINANCE_TOLERANCE {
            return candidate;
        }
        if reached < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    Rgb::from_hsl(hue, saturation, low.midpoint(high))
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
