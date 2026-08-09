//! Exact time representation (§9).
//!
//! # The rule
//!
//! Timeline positions are integer ticks. Never `f32`/`f64` seconds (§74).
//!
//! # Why 960,000 and not 1,000,000
//!
//! NTSC frame rates are everywhere — phones and screen recorders default to
//! 29.97 and 59.94. At 1,000,000 ticks/s a 29.97 fps frame is
//! `1_000_000 × 1001 / 30_000 = 33366.666…`, so every frame boundary rounds and
//! the error accumulates. Split-at-playhead then lands off-by-one deep into a
//! long clip.
//!
//! 960,000 divides exactly by every rate we support, and by 48,000 — so one
//! audio sample at 48 kHz is exactly 20 ticks.
//!
//! It does **not** divide by 44,100. All audio is therefore resampled to 48 kHz
//! at import (§9, §20a.3).

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::rational::{FrameRate, Rational};

/// Ticks per second on the timeline. See the module docs before changing this.
pub const TICKS_PER_SECOND: i64 = 960_000;

/// The audio sample rate everything is normalized to at import (§20a.3).
pub const AUDIO_SAMPLE_RATE: i64 = 48_000;

/// Ticks in one 48 kHz audio sample. Exact: 960_000 / 48_000.
pub const TICKS_PER_AUDIO_SAMPLE: i64 = TICKS_PER_SECOND / AUDIO_SAMPLE_RATE;

macro_rules! define_time_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Stored as `i64` ticks at [`TICKS_PER_SECOND`]. Range is roughly
        /// ±304,000 years, so overflow is not a practical concern.
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Default,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name {
            ticks: i64,
        }

        impl $name {
            pub const ZERO: Self = Self { ticks: 0 };
            pub const MAX: Self = Self { ticks: i64::MAX };

            pub const fn from_ticks(ticks: i64) -> Self {
                Self { ticks }
            }

            pub const fn ticks(self) -> i64 {
                self.ticks
            }

            pub const fn from_seconds(seconds: i64) -> Self {
                Self {
                    ticks: seconds.saturating_mul(TICKS_PER_SECOND),
                }
            }

            pub const fn from_millis(millis: i64) -> Self {
                Self {
                    ticks: millis.saturating_mul(TICKS_PER_SECOND / 1000),
                }
            }

            /// Position of frame `index` at `rate`.
            ///
            /// Returns `None` if `rate` does not divide the timebase exactly —
            /// we refuse to round rather than accumulate error (§9).
            pub fn from_frames(index: i64, rate: FrameRate) -> Option<Self> {
                let per_frame = ticks_per_frame(rate)?;
                index.checked_mul(per_frame).map(Self::from_ticks)
            }

            /// Index of the frame containing this instant, rounding toward
            /// negative infinity so that a position *inside* a frame belongs to
            /// that frame.
            pub fn frame_index(self, rate: FrameRate) -> Option<i64> {
                let per_frame = ticks_per_frame(rate)?;
                Some(self.ticks.div_euclid(per_frame))
            }

            /// Snap down to the start of the containing frame.
            ///
            /// §76 requires this before constructing a split command: a cut must
            /// land on a frame boundary.
            pub fn snap_to_frame(self, rate: FrameRate) -> Option<Self> {
                let per_frame = ticks_per_frame(rate)?;
                Some(Self::from_ticks(
                    self.ticks.div_euclid(per_frame) * per_frame,
                ))
            }

            /// Snap to the nearest frame boundary, rounding half away from zero.
            pub fn snap_to_nearest_frame(self, rate: FrameRate) -> Option<Self> {
                let per_frame = ticks_per_frame(rate)?;
                let rem = self.ticks.rem_euclid(per_frame);
                let floor = self.ticks - rem;
                Some(Self::from_ticks(if rem * 2 >= per_frame {
                    floor + per_frame
                } else {
                    floor
                }))
            }

            /// Convert a timestamp expressed in an FFmpeg timebase.
            ///
            /// `timestamp` counts units of `timebase` seconds. Uses an `i128`
            /// intermediate so a fine timebase (1/90000, 1/1000000) over a long
            /// duration cannot overflow before the division.
            pub fn from_timebase(timestamp: i64, timebase: Rational) -> Option<Self> {
                let scaled = (timestamp as i128)
                    .checked_mul(timebase.num() as i128)?
                    .checked_mul(TICKS_PER_SECOND as i128)?;
                let ticks = scaled / timebase.den() as i128;
                i64::try_from(ticks).ok().map(Self::from_ticks)
            }

            /// Convert back into a timestamp in `timebase` units, truncating.
            pub fn to_timebase(self, timebase: Rational) -> Option<i64> {
                let scaled = (self.ticks as i128).checked_mul(timebase.den() as i128)?;
                let divisor = (timebase.num() as i128).checked_mul(TICKS_PER_SECOND as i128)?;
                if divisor == 0 {
                    return None;
                }
                i64::try_from(scaled / divisor).ok()
            }

            /// Lossy. Display, shaders, and FFI only — never timeline
            /// arithmetic (§74).
            pub fn as_seconds_f64(self) -> f64 {
                self.ticks as f64 / TICKS_PER_SECOND as f64
            }

            pub fn checked_add(self, rhs: Self) -> Option<Self> {
                self.ticks.checked_add(rhs.ticks).map(Self::from_ticks)
            }

            pub fn checked_sub(self, rhs: Self) -> Option<Self> {
                self.ticks.checked_sub(rhs.ticks).map(Self::from_ticks)
            }

            pub fn saturating_add(self, rhs: Self) -> Self {
                Self::from_ticks(self.ticks.saturating_add(rhs.ticks))
            }

            pub fn saturating_sub(self, rhs: Self) -> Self {
                Self::from_ticks(self.ticks.saturating_sub(rhs.ticks))
            }

            pub fn max(self, other: Self) -> Self {
                Self::from_ticks(self.ticks.max(other.ticks))
            }

            pub fn min(self, other: Self) -> Self {
                Self::from_ticks(self.ticks.min(other.ticks))
            }

            pub fn is_zero(self) -> bool {
                self.ticks == 0
            }

            pub fn is_negative(self) -> bool {
                self.ticks < 0
            }
        }

        impl Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self::from_ticks(self.ticks.saturating_add(rhs.ticks))
            }
        }

        impl Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self::from_ticks(self.ticks.saturating_sub(rhs.ticks))
            }
        }

        impl AddAssign for $name {
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }

        impl SubAssign for $name {
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }

        impl Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self::from_ticks(-self.ticks)
            }
        }
    };
}

define_time_type!(TimelineTime, "A position or duration on the timeline (§9).");
define_time_type!(
    MediaTime,
    "A position or duration inside a source media file.\n\nSame timebase as [`TimelineTime`] but a distinct type, so a source offset can never be added to a timeline position by accident."
);

impl TimelineTime {
    /// Format as `HH:MM:SS.mmm` for the HUD and timecode readouts.
    pub fn format_timecode(self) -> String {
        let negative = self.ticks < 0;
        let total = self.ticks.unsigned_abs();
        let total_ms = total / (TICKS_PER_SECOND as u64 / 1000);
        let (ms, secs) = (total_ms % 1000, total_ms / 1000);
        let (s, mins) = (secs % 60, secs / 60);
        let (m, h) = (mins % 60, mins / 60);
        format!(
            "{}{h:02}:{m:02}:{s:02}.{ms:03}",
            if negative { "-" } else { "" }
        )
    }
}

/// Exact tick count of one frame at `rate`, or `None` if the rate does not
/// divide the timebase.
///
/// `960_000 × den / num` — computed in that order so the multiplication happens
/// before the division and the remainder check is meaningful.
pub fn ticks_per_frame(rate: FrameRate) -> Option<i64> {
    let r = rate.as_rational();
    let numerator = TICKS_PER_SECOND.checked_mul(r.den())?;
    (numerator % r.num() == 0).then(|| numerator / r.num())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §83.10 — the table in §9, asserted.
    #[test]
    fn ticks_per_frame_matches_the_guide() {
        let expected = [
            (FrameRate::FILM_23_976, 40_040),
            (FrameRate::FILM_24, 40_000),
            (FrameRate::PAL_25, 38_400),
            (FrameRate::NTSC_29_97, 32_032),
            (FrameRate::FPS_30, 32_000),
            (FrameRate::PAL_50, 19_200),
            (FrameRate::NTSC_59_94, 16_016),
            (FrameRate::FPS_60, 16_000),
            (FrameRate::FPS_120, 8_000),
        ];
        for (rate, ticks) in expected {
            assert_eq!(ticks_per_frame(rate), Some(ticks), "rate {rate}");
        }
    }

    /// §83.10 — frame index round-trips exactly at every supported rate, over a
    /// range long enough that any per-frame rounding error would show up.
    #[test]
    fn frame_round_trip_is_exact_at_all_supported_rates() {
        for rate in FrameRate::SUPPORTED {
            for frame in [0i64, 1, 2, 29, 30, 1000, 100_000, 1_000_000] {
                let t = TimelineTime::from_frames(frame, rate)
                    .unwrap_or_else(|| panic!("{rate} does not divide the timebase"));
                assert_eq!(
                    t.frame_index(rate),
                    Some(frame),
                    "rate {rate} frame {frame}"
                );
            }
        }
    }

    /// An hour of 29.97 must land exactly, with no accumulated drift. This is
    /// the failure §9 was written to prevent.
    ///
    /// Note 3600 s is not a whole number of 29.97 frames, so this uses the last
    /// frame that starts within the hour and checks it is tick-exact.
    #[test]
    fn ntsc_does_not_drift_over_an_hour() {
        let rate = FrameRate::NTSC_29_97;
        let frames = 107_892; // floor(3600 × 30000/1001)
        let t = TimelineTime::from_frames(frames, rate).expect("exact rate");

        // Exactly 107892 × 1001/30000 seconds, in ticks. No rounding anywhere.
        assert_eq!(t.ticks(), 3_455_996_544);
        assert_eq!(t.ticks(), frames * 32_032);
        assert_eq!(t.frame_index(rate), Some(frames));

        // Within one frame of the hour mark, and just short of it.
        let hour = TimelineTime::from_seconds(3600);
        assert!(t < hour);
        assert!((hour - t).ticks() < 32_032);
    }

    #[test]
    fn one_audio_sample_is_twenty_ticks() {
        assert_eq!(TICKS_PER_AUDIO_SAMPLE, 20);
        assert_eq!(TICKS_PER_SECOND % AUDIO_SAMPLE_RATE, 0);
    }

    /// §9's stated consequence: 44.1 kHz cannot be represented, which is why
    /// import resamples. Asserted so nobody "fixes" the timebase later.
    #[test]
    fn forty_four_one_khz_does_not_divide_the_timebase() {
        assert_ne!(TICKS_PER_SECOND % 44_100, 0);
    }

    #[test]
    fn snapping_floors_into_the_containing_frame() {
        let rate = FrameRate::NTSC_29_97;
        let inside = TimelineTime::from_ticks(32_032 * 5 + 17);
        assert_eq!(
            inside.snap_to_frame(rate),
            Some(TimelineTime::from_ticks(32_032 * 5))
        );
        assert_eq!(inside.frame_index(rate), Some(5));
    }

    #[test]
    fn snapping_to_nearest_rounds_up_past_the_midpoint() {
        let rate = FrameRate::FPS_30; // 32_000 ticks/frame
        let just_past = TimelineTime::from_ticks(16_000);
        assert_eq!(
            just_past.snap_to_nearest_frame(rate),
            Some(TimelineTime::from_ticks(32_000))
        );
        let just_under = TimelineTime::from_ticks(15_999);
        assert_eq!(
            just_under.snap_to_nearest_frame(rate),
            Some(TimelineTime::ZERO)
        );
    }

    #[test]
    fn negative_positions_snap_toward_negative_infinity() {
        let rate = FrameRate::FPS_30;
        let t = TimelineTime::from_ticks(-1);
        assert_eq!(t.frame_index(rate), Some(-1));
        assert_eq!(
            t.snap_to_frame(rate),
            Some(TimelineTime::from_ticks(-32_000))
        );
    }

    /// §83.7 — conversion to and from FFmpeg rationals.
    ///
    /// The stride per timebase is the smallest timestamp step that lands on a
    /// whole tick. For most timebases it is 1; for 1/90000 (MPEG-TS) it is 3,
    /// because one 90 kHz unit is 10⅔ ticks. Real frame timestamps always land
    /// on the stride — a 29.97 fps frame at 90 kHz is 3003 units, and 3003 is a
    /// multiple of 3 — so this is not a limitation in practice.
    #[test]
    fn ffmpeg_timebase_round_trip_is_exact_on_representable_timestamps() {
        let cases = [
            ((1, 1_000), 1),      // common MP4 timebase
            ((1, 48_000), 1),     // audio
            ((1001, 30_000), 1),  // per-frame NTSC timebase
            ((1, 90_000), 3),     // MPEG-TS
            ((1, 1_000_000), 25), // microseconds
        ];

        for ((num, den), stride) in cases {
            let tb = Rational::new(num, den).expect("nonzero den");
            for step in [0i64, 1, 7, 4115, 30_000, 333_333] {
                let ts = step * stride;
                let t = MediaTime::from_timebase(ts, tb).expect("in range");
                assert_eq!(t.to_timebase(tb), Some(ts), "timebase {tb} ts {ts}");
            }
        }
    }

    /// A real 29.97 fps frame timestamp in the MPEG-TS timebase converts exactly.
    #[test]
    fn mpeg_ts_frame_timestamps_convert_exactly() {
        let tb = Rational::new(1, 90_000).expect("nonzero den");
        // One 29.97 fps frame at 90 kHz: 90000 × 1001/30000.
        let per_frame_90k = 3003;
        for frame in [0i64, 1, 30, 107_892] {
            let ts = frame * per_frame_90k;
            let t = MediaTime::from_timebase(ts, tb).expect("in range");
            assert_eq!(t.ticks(), frame * 32_032, "frame {frame}");
            assert_eq!(t.to_timebase(tb), Some(ts));
        }
    }

    /// Timestamps that fall between ticks truncate toward zero rather than
    /// rounding. Documented so nobody mistakes it for a bug later: at 960,000
    /// ticks/s the finest representable step is ~1.04 microseconds.
    #[test]
    fn timestamps_between_ticks_truncate() {
        let tb = Rational::new(1, 90_000).expect("nonzero den");
        // 1 unit = 10.666… ticks.
        assert_eq!(
            MediaTime::from_timebase(1, tb).expect("in range").ticks(),
            10
        );
        assert_eq!(
            MediaTime::from_timebase(2, tb).expect("in range").ticks(),
            21
        );
        assert_eq!(
            MediaTime::from_timebase(3, tb).expect("in range").ticks(),
            32
        );
    }

    #[test]
    fn ffmpeg_timebase_handles_long_durations_without_overflow() {
        // 24 hours at a 1/1000000 timebase — the intermediate would overflow i64.
        let tb = Rational::new(1, 1_000_000).expect("nonzero den");
        let ts = 86_400 * 1_000_000;
        let t = MediaTime::from_timebase(ts, tb).expect("in range");
        assert_eq!(t.ticks(), 86_400 * TICKS_PER_SECOND);
        assert_eq!(t.to_timebase(tb), Some(ts));
    }

    #[test]
    fn timecode_formats_both_signs() {
        assert_eq!(
            TimelineTime::from_millis(3_723_456).format_timecode(),
            "01:02:03.456"
        );
        assert_eq!(TimelineTime::ZERO.format_timecode(), "00:00:00.000");
        assert_eq!(
            TimelineTime::from_millis(-1_500).format_timecode(),
            "-00:00:01.500"
        );
    }

    #[test]
    fn arithmetic_saturates_rather_than_wrapping() {
        assert_eq!(
            TimelineTime::MAX + TimelineTime::from_seconds(1),
            TimelineTime::MAX
        );
        assert!(
            TimelineTime::MAX
                .checked_add(TimelineTime::from_ticks(1))
                .is_none()
        );
    }
}
