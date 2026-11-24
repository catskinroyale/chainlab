//! Fixed-point arithmetic.
//!
//! All simulation *state* (buffer contents, stock, coin, cycle progress) is
//! stored as `i64` "milli" values: 1 unit = 1000. Integer arithmetic is exact
//! and identical on every platform, so a run is reproducible bit for bit.
//!
//! Non-linear formulas (exponential cost growth, demand curves, logistic
//! regrowth, diminishing returns) are evaluated in `f64` as *pure functions of
//! the current integer state* and rounded back to [`Fx`] straight away. Only
//! the rounded value is stored, so floating-point error can never accumulate
//! from one tick to the next. The f64 operations used (`+ - * / powf`) are
//! evaluated in a fixed order, which keeps results deterministic on a given
//! target.

/// A fixed-point quantity: value * 1000.
pub type Fx = i64;

/// 1.0 in fixed point.
pub const ONE: Fx = 1000;

/// Convert a human `f64` to fixed point (rounded to nearest milli).
pub fn fx(x: f64) -> Fx {
    (x * ONE as f64).round() as Fx
}

/// Convert fixed point to `f64` (display and non-linear formulas only).
pub fn to_f64(x: Fx) -> f64 {
    x as f64 / ONE as f64
}

/// Fixed-point multiply, truncating toward zero.
pub fn mul(a: Fx, b: Fx) -> Fx {
    ((a as i128 * b as i128) / ONE as i128) as Fx
}

/// Fixed-point divide, truncating toward zero. Division by zero yields 0.
pub fn div(a: Fx, b: Fx) -> Fx {
    if b == 0 {
        0
    } else {
        ((a as i128 * ONE as i128) / b as i128) as Fx
    }
}

/// Scale `a` by an integer percentage (100 = unchanged).
pub fn pct(a: Fx, percent: i64) -> Fx {
    ((a as i128 * percent as i128) / 100) as Fx
}

/// Format with a fixed number of decimals.
pub fn fmt(x: Fx, dp: usize) -> String {
    format!("{:.*}", dp, to_f64(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_ops() {
        assert_eq!(fx(1.5), 1500);
        assert_eq!(to_f64(2500), 2.5);
        assert_eq!(mul(fx(1.5), fx(2.0)), fx(3.0));
        assert_eq!(div(fx(3.0), fx(2.0)), fx(1.5));
        assert_eq!(div(fx(3.0), 0), 0);
        assert_eq!(pct(fx(4.0), 50), fx(2.0));
    }
}
