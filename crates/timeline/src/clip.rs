//! Clips: references into media, placed on the timeline (§8).
//!
//! A clip never contains media. It contains a `MediaId` and two ranges — where
//! it sits on the timeline, and which part of the source it shows. That is what
//! makes editing non-destructive (§2).

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use serde::{Deserialize, Serialize};

use crate::error::TimelineError;

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
    pub enabled: bool,
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
            enabled: true,
        })
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
}
