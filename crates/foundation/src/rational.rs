//! Exact rational numbers, for frame rates and FFmpeg timebases.
//!
//! FFmpeg expresses every timestamp against an `AVRational` timebase. Converting
//! those to `TimelineTime` without a rational type means converting through
//! `f64`, which is exactly what §9 and §74 forbid.

use serde::{Deserialize, Serialize};

/// A rational number `num / den`. `den` is always positive and the value is
/// always stored in lowest terms, so equality is value equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    num: i64,
    den: i64,
}

impl Rational {
    /// Returns `None` if `den` is zero.
    pub fn new(num: i64, den: i64) -> Option<Self> {
        if den == 0 {
            return None;
        }
        // Normalize sign onto the numerator so `den > 0` always holds.
        let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()) as i64;
        if g == 0 {
            // num == 0: canonical zero is 0/1.
            return Some(Self { num: 0, den: 1 });
        }
        Some(Self {
            num: num / g,
            den: den / g,
        })
    }

    pub const fn num(self) -> i64 {
        self.num
    }

    pub const fn den(self) -> i64 {
        self.den
    }

    pub fn inverse(self) -> Option<Self> {
        Self::new(self.den, self.num)
    }

    /// Lossy. Display and diagnostics only — never timeline arithmetic (§74).
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub const ONE: Self = Self { num: 1, den: 1 };

    /// Build a ratio in a `const`, from parts already in lowest terms.
    ///
    /// [`Self::new`] cannot be `const` — it runs a gcd — so the constants that
    /// bound the speed control need this. **Not normalized**: pass it
    /// something like `2/4` and equality against `1/2` will be false. Every
    /// caller is a literal a reader can check, which is why this is acceptable
    /// here and why `new` remains the way to build one from computed values.
    pub const fn from_parts(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub fn is_one(self) -> bool {
        self.num == self.den
    }

    /// Multiply a tick count by this ratio, exactly.
    ///
    /// The one operation the speed control needs and §74 forbids doing in
    /// floating point: at 2× a clip advances two source ticks per timeline
    /// tick, and doing that through `as_f64` would drift across a long clip in
    /// exactly the way §9's timebase exists to prevent.
    ///
    /// Widened to `i128` for the multiply so the product never has to be
    /// reasoned about. Realistic values fit in `i64` — an hour is 3.5 billion
    /// ticks, and even times a numerator of 30,000 that is only 1e14 — but the
    /// margin depends on the ratio, and a cast that is safe *for the ratios we
    /// happen to use today* is the kind that breaks quietly later.
    ///
    /// Rounds to nearest, away from zero at the half. A tick is a 960,000th of
    /// a second, so what rounding loses is far below a sample, let alone a
    /// frame; what it buys is that scaling by a ratio and back lands where it
    /// started rather than drifting one tick earlier every time.
    pub fn scale(self, ticks: i64) -> i64 {
        if self.den == 0 {
            return ticks;
        }
        let numerator = i128::from(ticks) * i128::from(self.num);
        let denominator = i128::from(self.den);
        let half = denominator / 2;
        let rounded = if (numerator < 0) == (denominator < 0) {
            (numerator + half) / denominator
        } else {
            (numerator - half) / denominator
        };
        rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }
}

impl std::fmt::Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// A frame rate, in frames per second, as an exact rational.
///
/// NTSC rates are not integers — 29.97 fps is exactly 30000/1001 — and treating
/// them as 29.97 accumulates error across a long clip (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FrameRate(Rational);

impl FrameRate {
    pub const FILM_23_976: Self = Self(Rational {
        num: 24_000,
        den: 1001,
    });
    pub const FILM_24: Self = Self(Rational { num: 24, den: 1 });
    pub const PAL_25: Self = Self(Rational { num: 25, den: 1 });
    pub const NTSC_29_97: Self = Self(Rational {
        num: 30_000,
        den: 1001,
    });
    pub const FPS_30: Self = Self(Rational { num: 30, den: 1 });
    pub const PAL_50: Self = Self(Rational { num: 50, den: 1 });
    pub const NTSC_59_94: Self = Self(Rational {
        num: 60_000,
        den: 1001,
    });
    pub const FPS_60: Self = Self(Rational { num: 60, den: 1 });
    pub const FPS_120: Self = Self(Rational { num: 120, den: 1 });

    /// Every frame rate the MVP claims to support exactly (§83.10).
    pub const SUPPORTED: [Self; 9] = [
        Self::FILM_23_976,
        Self::FILM_24,
        Self::PAL_25,
        Self::NTSC_29_97,
        Self::FPS_30,
        Self::PAL_50,
        Self::NTSC_59_94,
        Self::FPS_60,
        Self::FPS_120,
    ];

    /// Returns `None` for a zero or negative rate.
    pub fn new(num: i64, den: i64) -> Option<Self> {
        let r = Rational::new(num, den)?;
        (r.num() > 0).then_some(Self(r))
    }

    pub const fn as_rational(self) -> Rational {
        self.0
    }

    /// Lossy. For display: `"29.97"`.
    pub fn as_f64(self) -> f64 {
        self.0.as_f64()
    }
}

impl std::fmt::Display for FrameRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.den() == 1 {
            write!(f, "{}", self.0.num())
        } else {
            write!(f, "{:.3}", self.as_f64())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_normalizes_to_lowest_terms() {
        let r = Rational::new(30_000, 1000).expect("nonzero den");
        assert_eq!((r.num(), r.den()), (30, 1));
    }

    #[test]
    fn rational_moves_sign_to_numerator() {
        let r = Rational::new(1, -4).expect("nonzero den");
        assert_eq!((r.num(), r.den()), (-1, 4));
    }

    #[test]
    fn rational_rejects_zero_denominator() {
        assert!(Rational::new(1, 0).is_none());
    }

    #[test]
    fn zero_has_canonical_form() {
        let r = Rational::new(0, 7).expect("nonzero den");
        assert_eq!((r.num(), r.den()), (0, 1));
    }

    #[test]
    fn ntsc_rates_are_not_integers() {
        // The whole reason Rational exists. If this ever equals 29.97 exactly,
        // someone has replaced the representation with a float.
        assert_eq!(FrameRate::NTSC_29_97.as_rational().num(), 30_000);
        assert_eq!(FrameRate::NTSC_29_97.as_rational().den(), 1001);
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    fn ratio(num: i64, den: i64) -> Rational {
        Rational::new(num, den).expect("non-zero denominator")
    }

    #[test]
    fn scaling_by_one_changes_nothing() {
        for ticks in [0, 1, -1, 960_000, i64::MAX / 2] {
            assert_eq!(Rational::ONE.scale(ticks), ticks);
        }
    }

    #[test]
    fn scaling_multiplies_and_divides_exactly() {
        assert_eq!(ratio(2, 1).scale(1000), 2000);
        assert_eq!(ratio(1, 2).scale(1000), 500);
        assert_eq!(ratio(3, 2).scale(1000), 1500);
        assert_eq!(ratio(1, 4).scale(1000), 250);
    }

    /// The whole reason this is not `as_f64`: an hour of ticks at an awkward
    /// ratio has to come back exact, and a float has 53 bits of mantissa
    /// against a tick count that already uses 32.
    #[test]
    fn a_long_clip_scales_without_drift() {
        let hour = 3600 * 960_000;
        assert_eq!(hour, 3_456_000_000);
        assert_eq!(ratio(1001, 1000).scale(hour), 3_459_456_000);
        assert_eq!(ratio(1000, 1001).scale(3_459_456_000), hour);
    }

    /// An hour of ticks against a frame-rate-sized numerator: the largest
    /// product this is ever asked for, and the answer is exact.
    #[test]
    fn a_large_ratio_stays_exact() {
        let hour = 3600 * 960_000;
        let scaled = ratio(30_000, 1001).scale(hour);
        assert_eq!(scaled, 103_576_423_576);
    }

    /// Rounding to nearest rather than truncating, so scaling out and back is
    /// a round trip instead of a slow march towards zero.
    #[test]
    fn scaling_out_and_back_returns_to_the_start() {
        for ticks in [1, 7, 999, 960_001, 12_345_678] {
            for (num, den) in [(2, 1), (1, 2), (3, 2), (7, 3), (1001, 1000)] {
                let out = ratio(num, den).scale(ticks);
                let back = ratio(den, num).scale(out);
                assert!(
                    (back - ticks).abs() <= 1,
                    "{ticks} × {num}/{den} = {out}, back = {back}"
                );
            }
        }
    }

    #[test]
    fn negatives_round_symmetrically() {
        assert_eq!(ratio(1, 2).scale(5), 3);
        assert_eq!(ratio(1, 2).scale(-5), -3);
    }

    #[test]
    fn one_is_recognised() {
        assert!(Rational::ONE.is_one());
        assert!(ratio(4, 4).is_one(), "4/4 normalizes to 1/1");
        assert!(!ratio(2, 1).is_one());
    }
}
