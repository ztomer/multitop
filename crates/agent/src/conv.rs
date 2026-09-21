//! Numeric conversions that are decisions, not casts.
//!
//! `as` is a SILENT conversion: each site was its own unstated choice about
//! what happens at the boundary, and clippy's cast lints flag every one.
//! These are the same conversions with the boundary stated once and tested:
//! integers into `f64` exactly (below 2^53, where `f64` itself stops holding
//! integers), floats into integers with the cast's own saturating rules, and
//! `f64` into `f32` for the render path.

use num_traits::ToPrimitive;

/// A `u64` as `f64`, exact below 2^53: two exact halves, so the conversion
/// does not have to argue about the size of its input.
#[must_use]
pub fn unsigned(n: u64) -> f64 {
    let hi = u32::try_from(n >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(n & 0xffff_ffff).unwrap_or(u32::MAX);
    f64::from(hi).mul_add(4_294_967_296.0, f64::from(lo))
}

/// A count as `f64`, exact below 2^53.
#[must_use]
pub fn count(n: usize) -> f64 {
    unsigned(u64::try_from(n).unwrap_or(u64::MAX))
}

/// A signed whole number as `f64`, exact below 2^53 in magnitude.
#[must_use]
pub fn signed(n: i64) -> f64 {
    let magnitude = unsigned(n.unsigned_abs());
    if n < 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// An `f64` as `i64`, truncating toward zero, with the cast's boundaries:
/// out of range saturates, NaN is 0.
#[must_use]
pub fn whole_i64(v: f64) -> i64 {
    v.to_i64().unwrap_or_else(|| {
        if v.is_nan() {
            0
        } else if v.is_sign_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

/// An `f64` as `u64`, truncating toward zero: negative and NaN are 0, too
/// large saturates.
#[must_use]
pub fn whole_u64(v: f64) -> u64 {
    v.to_u64().unwrap_or_else(|| {
        if v.is_nan() || v.is_sign_negative() {
            0
        } else {
            u64::MAX
        }
    })
}

/// An `f64` as `usize`, same boundaries as [`whole_u64`].
#[must_use]
pub fn whole_usize(v: f64) -> usize {
    v.to_usize().unwrap_or_else(|| {
        if v.is_nan() || v.is_sign_negative() {
            0
        } else {
            usize::MAX
        }
    })
}

/// An `f64` as `f32` for the render path: values that are percentages,
/// degrees and megabytes, well inside `f32`'s range; anything beyond it
/// becomes infinity, as the cast would.
#[must_use]
pub fn single(v: f64) -> f32 {
    v.to_f32().unwrap_or(f32::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(a: f64, b: f64) {
        assert_eq!(
            a.partial_cmp(&b),
            Some(std::cmp::Ordering::Equal),
            "{a} vs {b}"
        );
    }

    #[test]
    fn integers_are_exact_up_to_the_last_one_f64_holds() {
        exact(unsigned(0), 0.0);
        exact(unsigned(4_294_967_301), 4_294_967_301.0);
        exact(unsigned(1 << 53), 9_007_199_254_740_992.0);
        exact(count(7), 7.0);
        exact(signed(-3), -3.0);
        exact(signed(12), 12.0);
    }

    #[test]
    fn floats_to_integers_keep_the_casts_boundaries() {
        assert_eq!(whole_i64(3.9), 3);
        assert_eq!(whole_i64(-3.9), -3);
        assert_eq!(whole_i64(f64::INFINITY), i64::MAX);
        assert_eq!(whole_i64(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(whole_i64(f64::NAN), 0);
        assert_eq!(whole_u64(3.9), 3);
        assert_eq!(whole_u64(-1.0), 0);
        assert_eq!(whole_u64(1e30), u64::MAX);
        assert_eq!(whole_usize(2.5), 2);
        assert_eq!(whole_usize(-0.5), 0);
    }

    #[test]
    fn single_narrows_like_the_cast() {
        assert!((single(0.1) - 0.1f32).abs() < f32::EPSILON);
        assert!(single(1e300).is_infinite());
        assert!(single(f64::NAN).is_nan());
    }
}
