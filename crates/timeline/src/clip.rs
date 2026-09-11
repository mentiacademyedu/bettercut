//! Clips: references into media, placed on the timeline (§8).
//!
//! A clip never contains media. It contains a `MediaId` and two ranges — where
//! it sits on the timeline, and which part of the source it shows. That is what
//! makes editing non-destructive (§2).

use bettercut_foundation::{ClipId, LinkId, MediaId, MediaTime, Rational, TimelineTime};
use serde::{Deserialize, Serialize};

use crate::error::TimelineError;
use crate::keyframe::{AnimatedParameter, Keyframes};
use crate::transition::Transition;

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
    /// The sound that came from the same file, if it is on the timeline too
    /// (§12). See [`LinkId`].
    #[serde(default)]
    pub link: Option<LinkId>,

    /// How fast this clip plays, as an exact ratio: 2/1 is twice speed.
    ///
    /// A ratio rather than a float because it is used in *position*
    /// arithmetic — the source time for a timeline position is scaled by it —
    /// and §9 and §74 both forbid doing that through `f64`. At 2× on an hour
    /// of footage the difference between exact and floating point is a
    /// visible drift by the end.
    ///
    /// `serde(default)` gives projects written before speed existed the only
    /// answer that preserves them: normal.
    #[serde(default = "normal_speed")]
    pub speed: Rational,

    /// A transition at this clip's *end*, if any (§25).
    ///
    /// On the clip rather than on the track so that moving, trimming,
    /// splitting or pasting carries it along; see [`crate::transition`].
    #[serde(default)]
    pub transition_out: Option<Transition>,
    #[serde(default)]
    pub enabled: bool,
}

/// Adjustments applied to the finished picture, not to any one clip (§22).
///
/// The same four controls a clip has, but a *composite* of them: a blur here
/// softens the assembled image, where a blur on two stacked clips softens each
/// before they are combined. Those are different pictures, and "adjust the
/// whole video" means the first.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MasterLook {
    #[serde(default)]
    pub transform: Transform,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub color: ColorAdjust,
    #[serde(default)]
    pub blur: f32,
}

impl Default for MasterLook {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust::default(),
            blur: 0.0,
        }
    }
}

impl MasterLook {
    /// True when this changes nothing, so the renderer can skip the extra pass
    /// and the full-resolution texture it needs. The common case by far.
    pub fn is_identity(&self) -> bool {
        self.transform.is_identity()
            && self.opacity == 1.0
            && self.color.is_identity()
            && self.blur == 0.0
    }
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

    /// The picture that came from the same file, if it is on the timeline too
    /// (§12). See [`LinkId`].
    #[serde(default)]
    pub link: Option<LinkId>,

    /// How fast this clip plays (§51), exactly as on a video clip.
    ///
    /// Re-timing sound means resampling it, which changes the pitch — sped-up
    /// audio is higher, as it is on tape. That is what the effect *is*, and
    /// preserving pitch is a different feature needing a phase vocoder rather
    /// than a resampler.
    #[serde(default = "normal_speed")]
    pub speed: Rational,

    /// Linear gain, not decibels. Applied first in the §20a.4 mix graph.
    #[serde(default = "one")]
    pub gain: f32,
    #[serde(default)]
    pub enabled: bool,
}

fn one() -> f32 {
    1.0
}

fn normal_speed() -> Rational {
    Rational::ONE
}

/// Slowest and fastest a clip may play.
///
/// Bounds rather than taste. Below the floor a second of footage becomes half
/// a minute of timeline, and above the ceiling the decoder is asked to leap so
/// far between frames that every one of them is a seek — both are still
/// *correct*, and neither is something anyone wants to discover by accident.
pub const MIN_SPEED: Rational = Rational::from_parts(1, 10);
pub const MAX_SPEED: Rational = Rational::from_parts(10, 1);

/// How much of the frame a source fills when fitted inside it, per axis.
///
/// A source wider than the frame is limited by width and letterboxed top and
/// bottom; a narrower one is limited by height and pillarboxed. The result is
/// the fraction of the frame the picture covers before the clip's own scale is
/// applied.
///
/// **One implementation, because three things have to agree.** The shader
/// composites with it, the preview draws its drag handles around exactly the
/// rectangle it produces, and §26's titles undo it to draw at their own size.
/// If any of those computed it separately the handles would sit somewhere the
/// picture is not. It lives here rather than in the renderer because the other
/// two cannot depend on the renderer.
pub fn fit_scale(source_aspect: f32, output_aspect: f32) -> (f32, f32) {
    if source_aspect > output_aspect {
        (1.0, output_aspect / source_aspect)
    } else {
        (source_aspect / output_aspect, 1.0)
    }
}

/// The transform that draws a generated bitmap at its own size (§26).
///
/// Every layer is *fitted* to the canvas — a 640×360 frame fills a 1920×1080
/// one — which is right for footage and wrong for a title: a bitmap that
/// happens to be 400 pixels wide would be blown up to fill the frame, and the
/// same words in a longer sentence would come out smaller. Text sizes are in
/// sequence pixels, so a title 400 pixels wide must cover 400/1920 of the
/// canvas whatever else it says.
///
/// Undoing the fit rather than adding a mode to the shader keeps the
/// compositing path single (§46, §74).
pub fn natural_size_transform(
    transform: Transform,
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
) -> Transform {
    let (fit_x, _) = fit_scale(
        width.max(1) as f32 / height.max(1) as f32,
        output_width.max(1) as f32 / output_height.max(1) as f32,
    );
    if fit_x <= 0.0 {
        return transform;
    }

    // The fit preserves aspect, so undoing it is one number rather than two:
    // the x and y corrections are equal by construction, and a test asserts it.
    let correction = (width as f32 / output_width.max(1) as f32) / fit_x;

    let mut natural = transform;
    natural.scale = Vec2::new(
        transform.scale.x * correction,
        transform.scale.y * correction,
    );
    natural
}

/// Shared behaviour so tracks can be generic over what they hold.
pub trait Clip {
    fn id(&self) -> ClipId;
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

    /// What this clip is linked to, if anything (§12).
    fn link(&self) -> Option<LinkId> {
        None
    }

    /// Replace the link (§12). Defaulted to nothing for a title, which has no
    /// sound to be tied to.
    fn set_link(&mut self, _link: Option<LinkId>) {}

    /// Set the playback rate (§51).
    ///
    /// Defaulted to doing nothing, for the one clip kind that has no rate: a
    /// title is drawn, not played, and there is no material to run through
    /// faster. Nothing dispatches a speed change at one — the control is not
    /// offered — and this exists so the edit can be written once and applied to
    /// whichever kind of track holds the clip.
    fn set_speed(&mut self, _speed: Rational) {}

    /// How fast this clip plays.
    ///
    /// On the trait because [`crate::track::Track`]'s edits are generic over
    /// it: trimming an edge by *n* timeline ticks moves the source edge by *n
    /// × speed*, and splitting cuts the source at the scaled offset. Written
    /// only in the video clip and defaulted to normal everywhere else, so a
    /// clip kind with no speed control behaves exactly as it did.
    fn speed(&self) -> Rational {
        Rational::ONE
    }

    /// Drop any transition on this clip's end (§25).
    ///
    /// Splitting clones the clip, and a transition belongs to the *end* the
    /// user put it on — which after a split is the right half's. Left alone,
    /// one dissolve would become two, the spurious one landing on a cut the
    /// user never asked to soften.
    ///
    /// Defaulted, because audio has no transitions to drop.
    fn clear_transition_out(&mut self) {}
}

/// The shared half of [`Clip`], plus whatever else a given kind of clip needs.
macro_rules! impl_clip {
    ($t:ty $(, $extra:item)*) => {
        impl Clip for $t {
            $($extra)*
            fn id(&self) -> ClipId {
                self.id
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

// Used by `text.rs` too, so the generated-source clip cannot drift from the
// media-backed ones in how it reports its own ranges.
pub(crate) use impl_clip;

impl_clip!(
    VideoClip,
    // The only kind of clip that has a transition to drop; see the trait.
    fn clear_transition_out(&mut self) {
        self.transition_out = None;
    },
    // And the only kind with a speed control; see the trait.
    fn speed(&self) -> Rational {
        self.speed
    },
    fn set_speed(&mut self, speed: Rational) {
        self.speed = speed;
    },
    fn link(&self) -> Option<LinkId> {
        self.link
    },
    fn set_link(&mut self, link: Option<LinkId>) {
        self.link = link;
    }
);
impl_clip!(
    AudioClip,
    fn speed(&self) -> Rational {
        self.speed
    },
    fn set_speed(&mut self, speed: Rational) {
        self.speed = speed;
    },
    fn link(&self) -> Option<LinkId> {
        self.link
    },
    fn set_link(&mut self, link: Option<LinkId>) {
        self.link = link;
    }
);

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
            link: None,
            speed: Rational::ONE,
            transition_out: None,
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
        MediaTime::from_ticks(self.source.start.ticks() + self.speed.scale(into_clip))
    }

    /// How long this clip runs on the timeline at its current speed.
    ///
    /// The source range is what the clip *plays*; the speed decides how long
    /// that takes. Twice the speed, half the time.
    pub fn timeline_duration(&self) -> TimelineTime {
        TimelineTime::from_ticks(timeline_ticks_for(self.source.duration(), self.speed))
    }
}

/// Clamp a speed to what a clip may actually play.
///
/// Applied in the model rather than in the interface, for §38.2's reason: the
/// journal replays commands after a crash, and a limit that only existed in a
/// slider would come back unapplied.
pub fn clamped_speed(speed: Rational) -> Rational {
    if speed.num() <= 0 {
        // A zero or negative speed is not slow motion, it is a still frame or
        // a clip that plays backwards — neither is what the control means, and
        // both divide badly.
        return MIN_SPEED;
    }
    // Compared by cross-multiplication so the comparison is exact, like
    // everything else here.
    if speed.num() * MIN_SPEED.den() < MIN_SPEED.num() * speed.den() {
        return MIN_SPEED;
    }
    if speed.num() * MAX_SPEED.den() > MAX_SPEED.num() * speed.den() {
        return MAX_SPEED;
    }
    speed
}

/// How much timeline a span of source occupies at `speed`.
///
/// The inverse of the scaling [`VideoClip::source_time_at`] does, and written
/// once so the two cannot disagree — a clip whose length did not match the
/// material it plays would run out of frames before its own end.
pub fn timeline_ticks_for(source: MediaTime, speed: Rational) -> i64 {
    let inverse = speed.inverse().unwrap_or(Rational::ONE);
    inverse.scale(source.ticks()).max(0)
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
            link: None,
            speed: Rational::ONE,
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

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// The correction is one number, not two: the fit preserves aspect, so
    /// undoing it must scale both axes equally or a title would come out
    /// stretched.
    #[test]
    fn the_natural_size_correction_is_uniform() {
        for (w, h) in [(400, 120), (1920, 1080), (100, 900), (37, 41)] {
            let natural = natural_size_transform(Transform::default(), w, h, 1920, 1080);
            assert!(
                (natural.scale.x - natural.scale.y).abs() < 1e-4,
                "{w}×{h} came out stretched: {} vs {}",
                natural.scale.x,
                natural.scale.y
            );
        }
    }

    /// A bitmap a quarter of the frame's width covers a quarter of it — which
    /// is the whole point, and is *not* what fitting would do.
    #[test]
    fn a_bitmap_covers_its_own_fraction_of_the_frame() {
        let natural = natural_size_transform(Transform::default(), 480, 270, 1920, 1080);
        let (fit_x, _) = fit_scale(480.0 / 270.0, 1920.0 / 1080.0);

        // What the shader ends up multiplying by.
        let covered = fit_x * natural.scale.x;
        assert!(
            (covered - 0.25).abs() < 1e-4,
            "covered {covered} of the frame rather than a quarter"
        );
    }

    /// A source the same size as the frame is unchanged: fitting and natural
    /// size agree there, and a correction that did not know it would be a
    /// silent zoom.
    #[test]
    fn a_full_size_bitmap_is_left_alone() {
        let natural = natural_size_transform(Transform::default(), 1920, 1080, 1920, 1080);
        assert!((natural.scale.x - 1.0).abs() < 1e-4);
    }

    /// The clip's own scale still applies on top: a title set to 200% is twice
    /// its natural size, not twice the frame.
    #[test]
    fn the_clips_own_scale_still_multiplies() {
        let doubled = Transform {
            scale: Vec2::new(2.0, 2.0),
            ..Transform::default()
        };
        let plain = natural_size_transform(Transform::default(), 480, 270, 1920, 1080);
        let scaled = natural_size_transform(doubled, 480, 270, 1920, 1080);
        assert!((scaled.scale.x - plain.scale.x * 2.0).abs() < 1e-4);
    }
}

#[cfg(test)]
mod speed_tests {
    use super::*;
    use bettercut_foundation::MediaId;

    fn ratio(num: i64, den: i64) -> Rational {
        Rational::new(num, den).expect("non-zero denominator")
    }

    /// A four-second clip reading from one second in.
    fn clip() -> VideoClip {
        let source = SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(5))
            .expect("valid");
        VideoClip::new(MediaId::new(), TimelineTime::from_seconds(10), source).expect("valid")
    }

    #[test]
    fn a_new_clip_plays_at_normal_speed() {
        assert_eq!(clip().speed, Rational::ONE);
    }

    /// The mapping, which everything else follows from: a second of timeline is
    /// two seconds of source at 2×.
    #[test]
    fn the_source_advances_with_the_speed() {
        let mut clip = clip();
        clip.speed = ratio(2, 1);

        // One second into the clip.
        let at = TimelineTime::from_seconds(11);
        assert_eq!(clip.source_time_at(at), MediaTime::from_seconds(3));

        clip.speed = ratio(1, 2);
        assert_eq!(
            clip.source_time_at(at),
            MediaTime::from_millis(1500),
            "half speed reads half as far in"
        );
    }

    #[test]
    fn the_start_of_a_clip_is_its_in_point_at_any_speed() {
        for (num, den) in [(1, 1), (2, 1), (1, 4), (7, 3)] {
            let mut clip = clip();
            clip.speed = ratio(num, den);
            assert_eq!(
                clip.source_time_at(clip.timeline.start),
                clip.source.start,
                "{num}/{den} did not start at the in-point"
            );
        }
    }

    /// Twice the speed, half the time. This is the length the clip must occupy
    /// on the timeline, or it runs out of material before its own end.
    #[test]
    fn the_timeline_duration_is_the_source_over_the_speed() {
        let mut clip = clip();
        assert_eq!(clip.timeline_duration(), TimelineTime::from_seconds(4));

        clip.speed = ratio(2, 1);
        assert_eq!(clip.timeline_duration(), TimelineTime::from_seconds(2));

        clip.speed = ratio(1, 2);
        assert_eq!(clip.timeline_duration(), TimelineTime::from_seconds(8));
    }

    /// The two directions have to agree: reading at the very end of the clip
    /// must land on the out-point, not past it.
    #[test]
    fn the_end_of_a_clip_lands_on_its_out_point() {
        for (num, den) in [(1, 1), (2, 1), (1, 2), (4, 1), (1, 10), (7, 3), (10, 1)] {
            let mut clip = clip();
            clip.speed = ratio(num, den);
            clip.timeline = TimelineRange::new(
                clip.timeline.start,
                clip.timeline.start + clip.timeline_duration(),
            )
            .expect("non-empty");

            let at_end = clip.source_time_at(clip.timeline.end);
            let overshoot = at_end.ticks() - clip.source.end.ticks();
            assert!(
                overshoot.abs() <= 1,
                "{num}/{den}: the last frame is {overshoot} ticks off the out-point"
            );
        }
    }

    #[test]
    fn speed_is_clamped_to_something_playable() {
        assert_eq!(clamped_speed(ratio(1, 1)), ratio(1, 1));
        assert_eq!(clamped_speed(ratio(100, 1)), MAX_SPEED);
        assert_eq!(clamped_speed(ratio(1, 100)), MIN_SPEED);
    }

    /// Zero is a still frame and a negative is playing backwards. Neither is
    /// what the control means, and both divide badly.
    #[test]
    fn a_zero_or_backwards_speed_is_refused() {
        assert_eq!(clamped_speed(ratio(0, 1)), MIN_SPEED);
        assert_eq!(clamped_speed(ratio(-2, 1)), MIN_SPEED);
    }

    /// §9's point, in the one place speed could reintroduce drift: an hour at
    /// an awkward ratio still lands exactly.
    #[test]
    fn a_long_clip_does_not_drift() {
        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(3600)).expect("valid");
        let mut clip = VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).expect("valid");
        clip.speed = ratio(3, 2);
        clip.timeline =
            TimelineRange::new(TimelineTime::ZERO, clip.timeline_duration()).expect("non-empty");

        assert_eq!(clip.timeline_duration(), TimelineTime::from_seconds(2400));
        let at_end = clip.source_time_at(clip.timeline.end);
        assert_eq!(at_end, MediaTime::from_seconds(3600));
    }
}
