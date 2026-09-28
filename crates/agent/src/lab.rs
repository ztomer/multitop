//! Perceptual colour difference: how different two colours LOOK.
//!
//! This is a different instrument from [`crate::surface`]'s contrast ratio, and it
//! is here rather than in that module because the two answer different questions
//! and disagree. Contrast is a function of luminance alone, so a cyan and a green
//! at the same lightness score 1.0 : 1 against each other -- and they are
//! obviously different colours. Judging whether two roles are still told apart
//! with contrast made the palette half's own hue preservation look like a
//! failure, which is what sent me looking for a second measurement.
//!
//! CIE76 in Lab is the standard one, and the Lab space is the point: L* is
//! perceptual lightness rather than a gamma-expanded fraction of a byte, and a
//! and b are opponent axes, so equal numbers mean equal visible difference.

use crate::surface::{lin, Rgb};

/// CIE76 colour difference in Lab, the standard measure of how different two
/// colours LOOK rather than how far apart their numbers are.
///
/// This exists because WCAG contrast is the wrong instrument for telling two
/// foregrounds apart from each other. Contrast is a function of luminance
/// alone, so a cyan and a green at the same lightness score 1.0:1 -- and they
/// are obviously different colours. Judging role distinctness with it made
/// [`adapt`]'s own hue preservation look like a failure. 2.3 is the
/// conventional "just noticeable difference"; the floor here is well above it.
#[must_use]
// reachability: not yet called from production. This is the measurement the
// palette half uses to decide whether adaptation has collapsed two roles into
// one colour, and the caller is the wiring step that detects the terminal's
// background and adopts an `Adapted` palette -- which is not built yet. When
// `adapt` is called from the render path, so is this. Recorded rather than
// removed because the test that finds a collapsed palette needs the metric, and
// deleting it would delete the only check on the thing this whole module exists
// to protect.
pub fn colour_difference(a: Rgb, b: Rgb) -> f32 {
    let (la, aa, ba) = lab(a);
    let (lb, ab, bb) = lab(b);
    (la - lb)
        .mul_add(la - lb, (aa - ab).mul_add(aa - ab, (ba - bb).powi(2)))
        .sqrt()
}

fn lab(colour: Rgb) -> (f32, f32, f32) {
    // sRGB -> linear -> XYZ (D65), then XYZ -> Lab against a D65 white point.
    let red = lin(colour.r);
    let green = lin(colour.g);
    let blue = lin(colour.b);
    let big_x = XYZ_X_BLUE.mul_add(blue, XYZ_X_GREEN.mul_add(green, XYZ_X_RED * red)) / WHITE_X;
    let big_y = XYZ_Y_BLUE.mul_add(blue, XYZ_Y_GREEN.mul_add(green, XYZ_Y_RED * red)) / WHITE_Y;
    let big_z = XYZ_Z_BLUE.mul_add(blue, XYZ_Z_GREEN.mul_add(green, XYZ_Z_RED * red)) / WHITE_Z;
    let (f_x, f_y, f_z) = (lab_f(big_x), lab_f(big_y), lab_f(big_z));
    (
        LAB_K.mul_add(f_y, -LAB_OFFSET),
        LAB_A_SCALE * (f_x - f_y),
        LAB_B_SCALE * (f_y - f_z),
    )
}

/// The D65 white point, normalised so a pure white pixel lands at 1.
const WHITE_X: f32 = 0.950_47;
const WHITE_Y: f32 = 1.0;
const WHITE_Z: f32 = 1.088_83;

/// The sRGB-to-XYZ matrix for D65. Row-major, three columns per primary.
const XYZ_X_RED: f32 = 0.412_4;
const XYZ_X_GREEN: f32 = 0.357_6;
const XYZ_X_BLUE: f32 = 0.180_5;
const XYZ_Y_RED: f32 = 0.2126;
const XYZ_Y_GREEN: f32 = 0.7152;
const XYZ_Y_BLUE: f32 = 0.0722;
const XYZ_Z_RED: f32 = 0.019_3;
const XYZ_Z_GREEN: f32 = 0.119_2;
const XYZ_Z_BLUE: f32 = 0.950_5;

/// The Lab transfer function's knee and coefficients, from the CIE definition.
const LAB_KNEE: f32 = 0.008_856;
const LAB_LINEAR_SLOPE: f32 = 7.787;
const LAB_LINEAR_OFFSET: f32 = 16.0 / 116.0;
const LAB_K: f32 = 116.0;
const LAB_OFFSET: f32 = 16.0;
const LAB_A_SCALE: f32 = 500.0;
const LAB_B_SCALE: f32 = 200.0;

/// The Lab transfer function: a cube root above the knee, a linear ramp below,
/// which is the branch that keeps near-black from producing a negative cube root
/// and a NaN lightness.
fn lab_f(value: f32) -> f32 {
    if value > LAB_KNEE {
        value.cbrt()
    } else {
        LAB_LINEAR_SLOPE.mul_add(value, LAB_LINEAR_OFFSET)
    }
}
