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
