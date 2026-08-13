//! Clips: references into media, placed on the timeline (§8).
//!
//! A clip never contains media. It contains a `MediaId` and two ranges — where
//! it sits on the timeline, and which part of the source it shows. That is what
//! makes editing non-destructive (§2).

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use serde::{Deserialize, Serialize};

use crate::error::TimelineError;
use crate::keyframe::{AnimatedParameter, Keyframes};

/// A 2D point or size. Spatial, not temporal — floats are fine here (§74 bans
/// them only in timeline position arithmetic).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Geometric placement of a clip in the output frame (§59 "Basic transform").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Offset from centre, in normalized output-frame units.
    pub position: Vec2,
    pub scale: Vec2,
    pub rotation_degrees: f32,
    /// Rotation/scale origin, normalized: (0.5, 0.5) is the clip centre.
    pub anchor: Vec2,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            rotation_degrees: 0.0,
            anchor: Vec2::new(0.5, 0.5),
        }
    }
}

impl Transform {
    /// True when the transform is the identity, so the renderer can skip it.
    pub fn is_identity(&self) -> bool {
        self.position == Vec2::ZERO
            && self.scale == Vec2::ONE
            && self.rotation_degrees == 0.0
            && self.anchor == Vec2::new(0.5, 0.5)
    }
}

/// Basic colour adjustment (§45 "Colour adjustment → Cheap", Milestone 8).
///
/// Three numbers, each `1.0` when it does nothing, so the default is the
/// identity and `is_identity` lets the renderer skip the work entirely.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorAdjust {
    /// Exposure-style multiply. 1.0 leaves the picture alone.
    pub brightness: f32,
    /// Expansion around mid-grey. 1.0 leaves the picture alone.
    pub contrast: f32,
    /// 0.0 is greyscale, 1.0 is unchanged, above 1.0 is more saturated.
    pub saturation: f32,
}

impl Default for ColorAdjust {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

impl ColorAdjust {
    /// True when this does nothing, so the shader can take the cheap path.
    pub fn is_identity(&self) -> bool {
        self.brightness == 1.0 && self.contrast == 1.0 && self.saturation == 1.0
    }
}

/// Gaussian blur strength, on the 0–100 scale §35's effect schema defines
/// (`{"id": "gaussian_blur", "parameters": [{"id": "amount", "min": 0,
/// "max": 100, "default": 0}]}`).
///
/// Deliberately *not* a pixel radius. Preview and export differ in resolution
/// and in which media they read — proxy versus original (§46) — so a radius in
/// pixels would blur a 720p proxy and a 1080p original by visibly different
/// amounts and the preview would be lying about the result. The renderer turns
/// this into texels against whatever it is actually sampling.
pub const MAX_BLUR: f32 = 100.0;

/// The span a clip occupies on the timeline, half-open: `[start, end)`.
///
/// Half-open is what makes two clips butt-joined without a one-tick gap or a
/// one-tick overlap, which is the difference between a clean cut and a visible
/// flash frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineRange {
    pub start: TimelineTime,
    pub end: TimelineTime,
}

impl TimelineRange {
    pub fn new(start: TimelineTime, end: TimelineTime) -> Result<Self, TimelineError> {
        if end <= start {
            return Err(TimelineError::EmptyRange { start, end });
        }
        Ok(Self { start, end })
    }

    pub fn duration(self) -> TimelineTime {
        self.end - self.start
    }

    pub fn contains(self, t: TimelineTime) -> bool {
        t >= self.start && t < self.end
    }

    /// True when the two spans share at least one tick. Touching endpoints do
    /// not overlap, because the range is half-open.
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// The span within the source media that a clip shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start: MediaTime,
    pub end: MediaTime,
}

impl SourceRange {
    pub fn new(start: MediaTime, end: MediaTime) -> Result<Self, TimelineError> {
        if end <= start {
            return Err(TimelineError::EmptySourceRange { start, end });
        }
        Ok(Self { start, end })
    }

    pub fn duration(self) -> MediaTime {
        self.end - self.start
    }
}

/// A video clip on a video track (§8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoClip {
    pub id: ClipId,
    pub media_id: MediaId,

    pub timeline: TimelineRange,
    pub source: SourceRange,

    #[serde(default)]
    pub transform: Transform,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub color: ColorAdjust,
    /// Gaussian blur, 0–100 ([`MAX_BLUR`]). Zero is no blur, which is also
    /// `f32::default()`, so `serde(default)` gives older projects the right
    /// answer without a helper.
    #[serde(default)]
    pub blur: f32,
    /// Parameters that change over the clip (§24). Empty for most clips.
    #[serde(default)]
    pub keyframes: Keyframes,
    #[serde(default)]
    pub enabled: bool,
}

/// A clip's appearance at one instant, with animation already applied.
///
/// The fields above are what the user set with the sliders; this is what the
/// renderer draws. They differ only where a parameter is keyed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipLook {
    pub transform: Transform,
    pub opacity: f32,
    pub color: ColorAdjust,
    pub blur: f32,
}

/// An audio clip on an audio track (§8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: ClipId,
    pub media_id: MediaId,

    pub timeline: TimelineRange,
    pub source: SourceRange,

    /// Linear gain, not decibels. Applied first in the §20a.4 mix graph.
    #[serde(default = "one")]
    pub gain: f32,
    #[serde(default)]
    pub enabled: bool,
}

fn one() -> f32 {
    1.0
}

/// Shared behaviour so tracks can be generic over what they hold.
pub trait Clip {
    fn id(&self) -> ClipId;
    fn media_id(&self) -> MediaId;
    fn timeline(&self) -> TimelineRange;
    fn source(&self) -> SourceRange;
    fn set_timeline(&mut self, range: TimelineRange);
    fn set_source(&mut self, range: SourceRange);

    /// Give this clip a new identity.
    ///
    /// Needed by split, duplicate, and paste, which all produce clips derived
    /// from an existing one. Reusing the source clip's `ClipId` would break
    /// selection, undo, and every lookup that assumes IDs are unique.
    fn set_id(&mut self, id: ClipId);
}

macro_rules! impl_clip {
    ($t:ty) => {
        impl Clip for $t {
            fn id(&self) -> ClipId {
                self.id
            }
            fn media_id(&self) -> MediaId {
                self.media_id
            }
            fn timeline(&self) -> TimelineRange {
                self.timeline
            }
            fn source(&self) -> SourceRange {
                self.source
            }
            fn set_timeline(&mut self, range: TimelineRange) {
                self.timeline = range;
            }
            fn set_source(&mut self, range: SourceRange) {
                self.source = range;
            }
            fn set_id(&mut self, id: ClipId) {
                self.id = id;
            }
        }
    };
}

impl_clip!(VideoClip);
impl_clip!(AudioClip);

impl VideoClip {
    /// Place `source` at `start` on the timeline, at native speed.
    ///
    /// Timeline duration equals source duration: the MVP has no speed change
    /// (§59), and encoding that assumption in one constructor keeps the rest of
    /// the code from quietly assuming otherwise.
    pub fn new(
        media_id: MediaId,
        start: TimelineTime,
        source: SourceRange,
    ) -> Result<Self, TimelineError> {
        let end = start + TimelineTime::from_ticks(source.duration().ticks());
        Ok(Self {
            id: ClipId::new(),
            media_id,
            timeline: TimelineRange::new(start, end)?,
            source,
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust::default(),
            blur: 0.0,
            keyframes: Keyframes::default(),
            enabled: true,
        })
    }

    /// What to draw at `source_time` (§24, §46).
    ///
    /// A keyed parameter overrides its static field; everything else passes
    /// through. Both render configurations call this — preview and export must
    /// not each decide what "animated" means, or the exported fade would differ
    /// from the one the user watched.
    ///
    /// The time is in the *source* media, which is where keys are anchored: see
    /// [`crate::keyframe`].
    pub fn look_at(&self, source_time: MediaTime) -> ClipLook {
        let mut look = ClipLook {
            transform: self.transform,
            opacity: self.opacity,
            color: self.color,
            blur: self.blur,
        };
        if self.keyframes.is_empty() {
            return look;
        }

        let animated = |parameter: AnimatedParameter, into: &mut f32| {
            if let Some(value) = self.keyframes.value_at(parameter, source_time) {
                *into = parameter.clamp(value);
            }
        };
        animated(AnimatedParameter::Opacity, &mut look.opacity);
        animated(AnimatedParameter::PositionX, &mut look.transform.position.x);
        animated(AnimatedParameter::PositionY, &mut look.transform.position.y);
        animated(AnimatedParameter::ScaleX, &mut look.transform.scale.x);
        animated(AnimatedParameter::ScaleY, &mut look.transform.scale.y);
        animated(
            AnimatedParameter::Rotation,
            &mut look.transform.rotation_degrees,
        );
        animated(AnimatedParameter::Brightness, &mut look.color.brightness);
        animated(AnimatedParameter::Contrast, &mut look.color.contrast);
        animated(AnimatedParameter::Saturation, &mut look.color.saturation);
        animated(AnimatedParameter::Blur, &mut look.blur);
        look
    }

    /// The static value of one parameter — what the slider shows when the
    /// parameter is not animated, and the value a first keyframe starts from.
    pub fn parameter(&self, parameter: AnimatedParameter) -> f32 {
        match parameter {
            AnimatedParameter::Opacity => self.opacity,
            AnimatedParameter::PositionX => self.transform.position.x,
            AnimatedParameter::PositionY => self.transform.position.y,
            AnimatedParameter::ScaleX => self.transform.scale.x,
            AnimatedParameter::ScaleY => self.transform.scale.y,
            AnimatedParameter::Rotation => self.transform.rotation_degrees,
            AnimatedParameter::Brightness => self.color.brightness,
            AnimatedParameter::Contrast => self.color.contrast,
            AnimatedParameter::Saturation => self.color.saturation,
            AnimatedParameter::Blur => self.blur,
        }
    }

    /// Where in the source media the playhead at `position` is reading.
    ///
    /// Integer throughout (§74): the offset into the clip is the offset into
    /// the source, because the MVP has no speed change (§59).
    pub fn source_time_at(&self, position: TimelineTime) -> MediaTime {
        let into_clip = position.ticks() - self.timeline.start.ticks();
        MediaTime::from_ticks(self.source.start.ticks() + into_clip)
    }
}

impl AudioClip {
    pub fn new(
        media_id: MediaId,
        start: TimelineTime,
        source: SourceRange,
    ) -> Result<Self, TimelineError> {
        let end = start + TimelineTime::from_ticks(source.duration().ticks());
        Ok(Self {
            id: ClipId::new(),
            media_id,
            timeline: TimelineRange::new(start, end)?,
            source,
            gain: 1.0,
            enabled: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: i64, end: i64) -> TimelineRange {
        TimelineRange::new(
            TimelineTime::from_ticks(start),
            TimelineTime::from_ticks(end),
        )
        .expect("non-empty")
    }

    #[test]
    fn ranges_must_be_non_empty() {
        let t = TimelineTime::from_ticks(100);
        assert!(TimelineRange::new(t, t).is_err());
        assert!(TimelineRange::new(t, TimelineTime::from_ticks(99)).is_err());
    }

    /// Butt-joined clips must not overlap. A half-open range is the only way to
    /// get this right without an off-by-one tick at every cut.
    #[test]
    fn touching_ranges_do_not_overlap() {
        let a = range(0, 100);
        let b = range(100, 200);
        assert!(!a.overlaps(b));
        assert!(!b.overlaps(a));
        assert!(a.overlaps(range(99, 150)));
    }

    #[test]
    fn contains_excludes_the_end_tick() {
        let a = range(0, 100);
        assert!(a.contains(TimelineTime::from_ticks(0)));
        assert!(a.contains(TimelineTime::from_ticks(99)));
        assert!(!a.contains(TimelineTime::from_ticks(100)));
    }

    #[test]
    fn clip_timeline_duration_matches_source_duration() {
        let source = SourceRange::new(
            bettercut_foundation::MediaTime::from_ticks(1000),
            bettercut_foundation::MediaTime::from_ticks(4200),
        )
        .expect("non-empty");
        let clip = VideoClip::new(
            bettercut_foundation::MediaId::new(),
            TimelineTime::from_ticks(500),
            source,
        )
        .expect("valid");
        assert_eq!(clip.timeline.duration().ticks(), 3200);
        assert_eq!(clip.timeline.start.ticks(), 500);
        assert_eq!(clip.timeline.end.ticks(), 3700);
    }

    #[test]
    fn default_transform_is_identity() {
        assert!(Transform::default().is_identity());
    }

    fn animated_clip() -> VideoClip {
        let source = SourceRange::new(
            bettercut_foundation::MediaTime::from_ticks(1000),
            bettercut_foundation::MediaTime::from_ticks(5000),
        )
        .expect("non-empty");
        VideoClip::new(
            bettercut_foundation::MediaId::new(),
            TimelineTime::from_ticks(500),
            source,
        )
        .expect("valid")
    }

    fn media(ticks: i64) -> bettercut_foundation::MediaTime {
        bettercut_foundation::MediaTime::from_ticks(ticks)
    }

    /// With no keys the look is exactly the static fields, which is the path
    /// every clip in a project takes.
    #[test]
    fn an_unanimated_clip_looks_like_its_static_values() {
        let mut clip = animated_clip();
        clip.opacity = 0.5;
        clip.blur = 20.0;
        let look = clip.look_at(media(3000));
        assert_eq!(look.opacity, 0.5);
        assert_eq!(look.blur, 20.0);
        assert_eq!(look.transform, clip.transform);
    }

    /// The whole point: a keyed parameter ignores its slider.
    #[test]
    fn a_keyed_parameter_overrides_the_static_value() {
        let mut clip = animated_clip();
        clip.opacity = 1.0;
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            crate::keyframe::Keyframe::new(media(1000), 0.0, crate::Interpolation::Linear),
        );
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            crate::keyframe::Keyframe::new(media(5000), 1.0, crate::Interpolation::Linear),
        );

        assert_eq!(clip.look_at(media(1000)).opacity, 0.0);
        assert_eq!(clip.look_at(media(5000)).opacity, 1.0);
        let mid = clip.look_at(media(3000)).opacity;
        assert!((mid - 0.5).abs() < 1e-6, "midpoint was {mid}");
        // Untouched parameters still come from the fields.
        assert_eq!(clip.look_at(media(3000)).blur, clip.blur);
    }

    /// A curve that overshoots must not produce a value the slider could never
    /// reach: §24's Bézier is allowed to bounce, the renderer is not.
    #[test]
    fn an_overshooting_curve_is_clamped_to_the_parameter_limits() {
        let mut clip = animated_clip();
        let bounce = crate::Interpolation::Bezier {
            x1: 0.5,
            y1: 0.0,
            x2: 0.5,
            y2: 2.5,
        };
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            crate::keyframe::Keyframe::new(media(1000), 0.0, bounce),
        );
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            crate::keyframe::Keyframe::new(media(5000), 1.0, crate::Interpolation::Linear),
        );

        for tick in (1000..=5000).step_by(50) {
            let opacity = clip.look_at(media(tick)).opacity;
            assert!(
                (0.0..=1.0).contains(&opacity),
                "opacity left its range at {tick}: {opacity}"
            );
        }
    }

    /// Keys are anchored to the source, so moving the clip must not move the
    /// animation relative to the picture.
    #[test]
    fn moving_a_clip_does_not_move_its_animation() {
        let mut clip = animated_clip();
        clip.keyframes.set(
            AnimatedParameter::Blur,
            crate::keyframe::Keyframe::new(media(2000), 60.0, crate::Interpolation::Linear),
        );

        let before = clip.look_at(clip.source_time_at(TimelineTime::from_ticks(1500)));
        // The same edit `MoveClip` makes: timeline moves, source does not.
        clip.timeline = TimelineRange::new(
            TimelineTime::from_ticks(90_000),
            TimelineTime::from_ticks(94_000),
        )
        .expect("valid");
        let after = clip.look_at(clip.source_time_at(TimelineTime::from_ticks(91_000)));

        assert_eq!(
            before.blur, after.blur,
            "the animation drifted when the clip moved"
        );
    }

    #[test]
    fn source_time_tracks_the_offset_into_the_clip() {
        let clip = animated_clip(); // timeline 500.., source 1000..
        assert_eq!(
            clip.source_time_at(TimelineTime::from_ticks(500)).ticks(),
            1000
        );
        assert_eq!(
            clip.source_time_at(TimelineTime::from_ticks(1500)).ticks(),
            2000
        );
    }
}
