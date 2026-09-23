//! Adjustment layers: a grade over a stretch of the edit (§22, §45).
//!
//! ## A clip that shows nothing of its own
//!
//! An adjustment clip has a span on the timeline and no picture. What it does
//! is change the picture beneath it for as long as it runs — a colour grade, a
//! softening — which is how "put a filter on this part of the video" is done
//! without touching the clips themselves, and without the grade being lost the
//! moment one of those clips is swapped for another take.
//!
//! Like a title it is a generated source, so it implements [`Clip`] and lives in
//! an ordinary [`Track`]: insert, move, trim, split and ripple delete come with
//! the container, already written and already tested. Its source range starts
//! at zero and runs as long as the clip does, for the reason `text.rs` gives.
//!
//! ## What it grades
//!
//! Every picture beneath it, and none of the titles. Adjustment lanes sit above
//! the video tracks and below the text lanes, which is where a filter belongs:
//! over the footage, and never softening the words laid on top of it.

use bettercut_foundation::{ClipId, MediaTime, TimelineTime};
use serde::{Deserialize, Serialize};

use crate::clip::{Clip, ColorAdjust, SourceRange, TimelineRange, impl_clip};
use crate::error::TimelineError;
use crate::track::Track;

/// How long an adjustment is when it is first added.
///
/// Three seconds: long enough to see the grade play, short enough that it does
/// not swallow the edit, and drawn wide enough on the timeline to take hold of
/// and stretch.
pub const DEFAULT_DURATION: TimelineTime = TimelineTime::from_seconds(3);

/// How an adjustment changes the picture beneath it, carried whole.
///
/// One value, for the reason `ClipLook` is one: the renderer receives it as it
/// is, so a field added here reaches the picture without anyone having to
/// remember to copy it across in two places.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentLook {
    /// The same colour controls a clip has, with the same identity.
    #[serde(default)]
    pub color: ColorAdjust,
    /// §45's blur, 0–100, over the whole picture beneath.
    #[serde(default)]
    pub blur: f32,
    /// How much of the adjustment shows: 0 is none, 1 is all of it. A way to
    /// ease a grade back without redoing it.
    #[serde(default = "full_strength")]
    pub strength: f32,
    /// How much the edges of the frame are darkened, 0–1. An adjustment covers
    /// the whole frame, so its vignette frames the frame, as the master's does.
    #[serde(default)]
    pub vignette: f32,
    /// Film grain, 0–1, over the frame while the adjustment runs — for a
    /// flashback, or one scene shot "on film".
    #[serde(default)]
    pub grain: f32,
}

fn full_strength() -> f32 {
    1.0
}

impl Default for AdjustmentLook {
    fn default() -> Self {
        Self {
            color: ColorAdjust::IDENTITY,
            blur: 0.0,
            strength: 1.0,
            vignette: 0.0,
            grain: 0.0,
        }
    }
}

impl AdjustmentLook {
    /// True when it changes nothing, so it costs the picture nothing.
    pub fn is_identity(&self) -> bool {
        self.strength <= 0.0
            || (self.color.is_identity()
                && self.blur <= 0.0
                && self.vignette <= 0.0
                && self.grain <= 0.0)
    }

    /// Brought into range, as every value a project file can carry is.
    pub fn clamped(self) -> Self {
        use crate::keyframe::AnimatedParameter as A;
        let finite = |value: f32, fallback: f32| {
            if value.is_finite() { value } else { fallback }
        };
        Self {
            color: ColorAdjust {
                brightness: A::Brightness.clamp(finite(self.color.brightness, 1.0)),
                contrast: A::Contrast.clamp(finite(self.color.contrast, 1.0)),
                saturation: A::Saturation.clamp(finite(self.color.saturation, 1.0)),
                temperature: A::Temperature.clamp(finite(self.color.temperature, 0.0)),
                tint: A::Tint.clamp(finite(self.color.tint, 0.0)),
                vibrance: finite(self.color.vibrance, 0.0).clamp(-1.0, 1.0),
                wheels: self.color.wheels.clamped(),
                secondary: self.color.secondary.clamped(),
            },
            blur: A::Blur.clamp(finite(self.blur, 0.0)),
            strength: finite(self.strength, 1.0).clamp(0.0, 1.0),
            vignette: finite(self.vignette, 0.0).clamp(0.0, crate::clip::MAX_VIGNETTE),
            grain: finite(self.grain, 0.0).clamp(0.0, crate::clip::MAX_GRAIN),
        }
    }
}

/// An adjustment on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentClip {
    pub id: ClipId,
    pub timeline: TimelineRange,
    /// Zero-based within the clip's own span, as a title's is.
    pub source: SourceRange,
    pub look: AdjustmentLook,
    /// A colour tag for organising the edit.
    #[serde(default)]
    pub color_label: crate::clip::ColorLabel,
    /// A name of the clip's own, shown on the timeline instead of the
    /// file's: "interview wide", not "C0042.MP4". `None` is the file's name.
    /// Defaulted: older projects have none.
    #[serde(default)]
    pub name: Option<String>,
}

impl AdjustmentClip {
    /// An adjustment at `start` for [`DEFAULT_DURATION`], changing nothing yet.
    pub fn new(start: TimelineTime) -> Result<Self, TimelineError> {
        Self::with_duration(start, DEFAULT_DURATION)
    }

    pub fn with_duration(
        start: TimelineTime,
        duration: TimelineTime,
    ) -> Result<Self, TimelineError> {
        Ok(Self {
            id: ClipId::new(),
            timeline: TimelineRange::new(start, start + duration)?,
            source: SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(duration.ticks()))?,
            look: AdjustmentLook::default(),
            color_label: crate::clip::ColorLabel::None,
            name: None,
        })
    }
}

impl_clip!(AdjustmentClip);

/// A lane of adjustments.
pub type AdjustmentTrack = Track<AdjustmentClip>;
