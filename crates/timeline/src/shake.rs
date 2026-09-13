//! Camera shake: a handheld wobble, or the jolt of an impact, written as
//! keyframes.
//!
//! # Keys, not an effect
//!
//! Like [`crate::Movement`], a shake is ordinary position keys (§24) rather
//! than a mode the renderer has to know about. The preview and the export
//! draw it with the same interpolation they draw every other key with (§46),
//! and the keyframe editor can adjust or thin it afterwards.
//!
//! # Hiding the edges
//!
//! Moving a picture that fills the frame uncovers a strip of background at
//! the edge it moves away from. So a shake also enlarges the clip by just
//! enough to cover its own travel — [`ShakeStrength::overscan`] — written as a
//! constant pair of scale keys, which is what tells it apart from a zoom
//! movement and lets it come off again cleanly.
//!
//! # The same shake every time
//!
//! The wobble is pseudo-random but fixed: the same clip gets the same shake on
//! every machine and after every reload, so a cut timed against a jolt stays
//! timed. No random-number generator is involved; the offsets come from a
//! small integer hash of the step number.

use bettercut_foundation::MediaTime;

use crate::clip::{SourceRange, Vec2};
use crate::keyframe::{AnimatedParameter, Interpolation, Keyframe};

/// How hard the camera shakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShakeStrength {
    /// A handheld drift: alive, not distracting.
    Gentle,
    /// A running camera, all the way through.
    Strong,
    /// One hard jolt at the start that settles in half a second — for a hit,
    /// a landing, a beat drop.
    Impact,
}

/// Time between shake keys, in source time: fifteen a second. Fast enough to
/// read as a shake, slow enough that eased keys still glide between points
/// rather than flicker frame to frame.
pub const SHAKE_STEP: MediaTime =
    MediaTime::from_ticks(bettercut_foundation::TICKS_PER_SECOND / 15);

/// How long an impact takes to settle.
pub const IMPACT_SETTLE: MediaTime =
    MediaTime::from_ticks(bettercut_foundation::TICKS_PER_SECOND / 2);

impl ShakeStrength {
    pub const ALL: [Self; 3] = [Self::Gentle, Self::Strong, Self::Impact];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gentle => "Gentle",
            Self::Strong => "Strong",
            Self::Impact => "Impact",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Gentle => "A slight handheld drift through the whole clip",
            Self::Strong => "A running-camera shake through the whole clip",
            Self::Impact => "One hard jolt at the start that settles in half a second",
        }
    }

    /// The furthest the picture travels from its place, in normalized frame
    /// units (1.0 is a whole frame).
    pub fn amplitude(self) -> f32 {
        match self {
            Self::Gentle => 0.006,
            Self::Strong => 0.018,
            Self::Impact => 0.04,
        }
    }

    /// The enlargement that keeps the frame's edges covered through the whole
    /// shake: travel of `a` either way uncovers `a` at an edge, and scaling by
    /// `1 + 2a` grows each side by exactly `a`.
    pub fn overscan(self) -> f32 {
        1.0 + 2.0 * self.amplitude()
    }

    /// The keys this shake writes on a clip playing `source`, around the
    /// clip's own `position` and `scale`: position keys every [`SHAKE_STEP`],
    /// and a constant scale pair for the overscan.
    ///
    /// The first and last position keys sit exactly at the clip's place, so a
    /// shake starts and ends where the shot was framed.
    pub fn keyframes(
        self,
        position: Vec2,
        scale: Vec2,
        source: SourceRange,
    ) -> Vec<(AnimatedParameter, Keyframe)> {
        let first = source.start.ticks();
        // Half-open range: a key at `end` would never be reached.
        let last = (source.end.ticks() - 1).max(first);
        let end_of_shake = match self {
            Self::Impact => (first + IMPACT_SETTLE.ticks()).min(last),
            Self::Gentle | Self::Strong => last,
        };

        let mut keys = Vec::new();
        let mut step = 0_u32;
        loop {
            let at = first + i64::from(step) * SHAKE_STEP.ticks();
            if at >= end_of_shake {
                break;
            }
            let (dx, dy) = if step == 0 {
                (0.0, 0.0)
            } else {
                let fade = match self {
                    // Squared, so the jolt spends its energy early and
                    // settles rather than dying linearly.
                    Self::Impact => {
                        let left = 1.0 - (at - first) as f32 / IMPACT_SETTLE.ticks() as f32;
                        left.max(0.0).powi(2)
                    }
                    Self::Gentle | Self::Strong => 1.0,
                };
                let amount = self.amplitude() * fade;
                (wobble(step, 0) * amount, wobble(step, 1) * amount)
            };
            let time = MediaTime::from_ticks(at);
            keys.push((
                AnimatedParameter::PositionX,
                Keyframe::new(time, position.x + dx, Interpolation::EaseInOut),
            ));
            keys.push((
                AnimatedParameter::PositionY,
                Keyframe::new(time, position.y + dy, Interpolation::EaseInOut),
            ));
            step += 1;
        }
        // Back at its place when the shake ends.
        let settle = MediaTime::from_ticks(end_of_shake);
        for (parameter, value) in [
            (AnimatedParameter::PositionX, position.x),
            (AnimatedParameter::PositionY, position.y),
        ] {
            keys.push((
                parameter,
                Keyframe::new(settle, value, Interpolation::EaseInOut),
            ));
        }

        let overscan = self.overscan();
        for (parameter, base) in [
            (AnimatedParameter::ScaleX, scale.x),
            (AnimatedParameter::ScaleY, scale.y),
        ] {
            for time in [MediaTime::from_ticks(first), MediaTime::from_ticks(last)] {
                keys.push((
                    parameter,
                    Keyframe::new(time, base * overscan, Interpolation::Linear),
                ));
            }
        }
        keys
    }

    /// Which shake, if any, a clip's keys were written by.
    ///
    /// Read from the overscan: a shake's scale is exactly two equal keys at
    /// `base_scale ×` [`Self::overscan`], which names the strength without
    /// guessing from how far a random wobble happened to stray. And the
    /// position keys must sit on the shake's grid — every [`SHAKE_STEP`] from
    /// the clip's first instant, plus the settling key — so a hand-made
    /// animation is never taken for a shake and replaced.
    pub fn recognise(
        position: &[Keyframe],
        scale: &[Keyframe],
        base_scale: f32,
        source: SourceRange,
    ) -> Option<Self> {
        let [from, to] = scale else {
            return None;
        };
        if from.value != to.value || base_scale == 0.0 {
            return None;
        }
        let strength = Self::ALL
            .into_iter()
            .find(|s| (from.value / base_scale - s.overscan()).abs() < 1e-4)?;

        Self::on_grid(position, source).then_some(strength)
    }

    /// Whether position keys sit where a shake puts them: every
    /// [`SHAKE_STEP`] from the clip's first instant, then one settling key.
    /// Enough to know a shake can be taken off — even after a zoom movement
    /// replaced its overscan — without mistaking a hand-made animation for
    /// one.
    pub fn on_grid(position: &[Keyframe], source: SourceRange) -> bool {
        let Some((settle, grid)) = position.split_last() else {
            return false;
        };
        // At least two keys on the grid: one key at the clip's start and one
        // later is the commonest hand-made animation there is, and must not
        // pass for a shake.
        grid.len() >= 2
            && grid.iter().enumerate().all(|(index, key)| {
                key.time.ticks() == source.start.ticks() + index as i64 * SHAKE_STEP.ticks()
            })
            && settle.time > grid[grid.len() - 1].time
    }
}

/// A fixed pseudo-random value in -1..1 for step `n` on axis `axis`.
fn wobble(n: u32, axis: u32) -> f32 {
    // SplitMix-style mixing of (n, axis): cheap, well spread, and the same on
    // every platform because it is integer arithmetic.
    let mut z = u64::from(n)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(axis).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1_u64 << 53) as f32 * 2.0 - 1.0
}
