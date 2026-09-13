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
    /// Mirror left-to-right.
    ///
    /// A mirror is not a scale of `-1`: it changes which part of the picture
    /// lands where and nothing else. The clip covers exactly the same region of
    /// the frame afterwards, which is why this is a flag and not a sign — a
    /// negative scale would run through the Inspector's sliders, through
    /// keyframe interpolation, and through every `fit`/`fill` multiply, and
    /// would have to be made to mean "mirror" again at each one.
    #[serde(default)]
    pub flip_h: bool,
    /// Mirror top-to-bottom.
    #[serde(default)]
    pub flip_v: bool,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            rotation_degrees: 0.0,
            anchor: Vec2::new(0.5, 0.5),
            flip_h: false,
            flip_v: false,
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
            && !self.flip_h
            && !self.flip_v
    }
}

/// Which way a mirror runs, so a caller cannot pass the wrong `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlipAxis {
    /// Left becomes right.
    Horizontal,
    /// Top becomes bottom.
    Vertical,
}

impl FlipAxis {
    pub const ALL: [Self; 2] = [Self::Horizontal, Self::Vertical];

    pub fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Flip Horizontal",
            Self::Vertical => "Flip Vertical",
        }
    }

    /// The flag this axis owns, for reading or writing.
    pub fn flag(self, transform: &mut Transform) -> &mut bool {
        match self {
            Self::Horizontal => &mut transform.flip_h,
            Self::Vertical => &mut transform.flip_v,
        }
    }

    pub fn is_set(self, transform: &Transform) -> bool {
        match self {
            Self::Horizontal => transform.flip_h,
            Self::Vertical => transform.flip_v,
        }
    }
}

/// How much of the source is thrown away before anything else happens (§22).
///
/// Each field is the fraction of the source taken off that edge, so all zero is
/// the whole picture. §22 puts this *before* the transform, and that ordering is
/// the whole difference between a crop and a rectangular mask:
///
/// * a **mask** hides part of a layer that is already placed, so the shot stays
///   the size it was and a hole appears in it;
/// * a **crop** changes what the layer *is*. What is left is a new picture, of
///   a new shape, and it is then fitted to the frame like any other source —
///   which is why cropping a 16:9 shot to a square and dropping it on a 9:16
///   sequence fills the width.
///
/// Fractions rather than pixels, so a crop set on a proxy still means the same
/// thing when the full-resolution media arrives (§13).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Crop {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// The least of the source a crop may leave on either axis.
///
/// A crop that takes everything leaves a zero-sized quad, which is not a
/// picture and cannot be dragged back out of — the same trap `ScaleX`'s lower
/// bound exists for.
pub const MIN_CROP_REMAINING: f32 = 0.05;

impl Crop {
    pub const NONE: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// True when nothing is taken off, so the renderer can skip the remap.
    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }

    /// Brought into range, as every value a project file can carry is.
    ///
    /// Each edge is held to 0..1 first, and then the *pair* on each axis is
    /// held apart by [`MIN_CROP_REMAINING`] — clamping the two independently
    /// would still allow 0.6 and 0.6, which leaves nothing at all.
    pub fn clamped(self) -> Self {
        let (left, right) = clamped_pair(self.left, self.right);
        let (top, bottom) = clamped_pair(self.top, self.bottom);
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// The shape of what is left, given the shape of the source it came from.
    ///
    /// **The one place this is worked out.** The renderer fits this to the
    /// frame, and the preview's handles have to sit on the picture the renderer
    /// drew — so both ask here. When the preview computed its own box from the
    /// *source's* aspect, every cropped clip had its move, scale and rotate
    /// handles drawn around a shape the picture no longer was.
    pub fn applied_to(self, source_aspect: f32) -> f32 {
        let (kept_x, kept_y) = self.remaining();
        if kept_y <= 0.0 || !source_aspect.is_finite() {
            return source_aspect;
        }
        source_aspect * kept_x / kept_y
    }

    /// What is left across and down, each in 0..1.
    pub fn remaining(self) -> (f32, f32) {
        let crop = self.clamped();
        (1.0 - crop.left - crop.right, 1.0 - crop.top - crop.bottom)
    }
}

/// Hold two opposite edges inside the frame with something left between them.
///
/// The excess comes off both edges evenly rather than off whichever is named
/// second: a crop read from a file is not a gesture with an order to respect,
/// and taking it all off one side would slide the surviving picture sideways.
fn clamped_pair(low: f32, high: f32) -> (f32, f32) {
    let low = if low.is_finite() {
        low.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let high = if high.is_finite() {
        high.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let over = low + high - (1.0 - MIN_CROP_REMAINING);
    if over <= 0.0 {
        return (low, high);
    }
    ((low - over / 2.0).max(0.0), (high - over / 2.0).max(0.0))
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
    /// White balance, warm/cool, on -1..1. Zero leaves the picture alone.
    ///
    /// `serde(default)` because it arrived after the three above: a project
    /// written before it says nothing about the white balance, and nothing is
    /// exactly what zero means.
    #[serde(default)]
    pub temperature: f32,
    /// White balance, green/magenta, on -1..1.
    #[serde(default)]
    pub tint: f32,
}

impl Default for ColorAdjust {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
        }
    }
}

impl ColorAdjust {
    /// The adjustment that changes nothing: every control at its identity.
    pub const IDENTITY: Self = Self {
        brightness: 1.0,
        contrast: 1.0,
        saturation: 1.0,
        temperature: 0.0,
        tint: 0.0,
    };

    /// Part of the way from this adjustment to another.
    ///
    /// What a look's *strength* is: 0 leaves the picture alone, 1 is the look
    /// as written, and between them is the same grade applied less. Each axis
    /// is independent — three multipliers around 1.0 and two offsets around
    /// 0.0 — so interpolating them separately is the whole of it.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let mix = |from: f32, to: f32| from + (to - from) * t;
        Self {
            brightness: mix(self.brightness, other.brightness),
            contrast: mix(self.contrast, other.contrast),
            saturation: mix(self.saturation, other.saturation),
            temperature: mix(self.temperature, other.temperature),
            tint: mix(self.tint, other.tint),
        }
    }

    /// How far along `self` is from [`Self::IDENTITY`] towards `look`, or
    /// `None` when it is not on that line at all.
    ///
    /// The inverse of [`Self::lerp`], and it exists so the interface can hold
    /// **one** source of truth. A clip stores the grade it ended up with and
    /// nothing else; which preset produced it, and how strongly, is recovered
    /// from the numbers. Storing the preset beside the grade would be two
    /// records of one fact, and they would disagree the first time someone
    /// nudged a slider.
    pub fn strength_towards(self, look: Self) -> Option<f32> {
        // The component that moves furthest carries the best signal; one that
        // barely moves says nothing about how far along the line we are.
        // Each axis as (where this grade is, where the look goes, where doing
        // nothing sits) — the identity is 1.0 for the multipliers and 0.0 for
        // the white balance, so it cannot be assumed.
        let axes = [
            (self.brightness, look.brightness, 1.0),
            (self.contrast, look.contrast, 1.0),
            (self.saturation, look.saturation, 1.0),
            (self.temperature, look.temperature, 0.0),
            (self.tint, look.tint, 0.0),
        ];
        let (value, target, identity) = axes
            .into_iter()
            .max_by(|a, b| (a.1 - a.2).abs().total_cmp(&(b.1 - b.2).abs()))?;

        // A look that goes nowhere is the identity, and everything is zero
        // strength towards it.
        if (target - identity).abs() < 1e-4 {
            return self.is_identity().then_some(0.0);
        }

        let t = ((value - identity) / (target - identity)).clamp(0.0, 1.0);

        // And it has to be the *same* t on the other two, or this grade merely
        // happens to share one number with the look. This is also what rejects
        // a grade *past* the look or against it: `lerp` clamps, so the
        // comparison below simply fails rather than needing a range check of
        // its own.
        Self::IDENTITY.lerp(look, t).near(self).then_some(t)
    }

    /// Whether two adjustments are the same to within what a control can set.
    fn near(self, other: Self) -> bool {
        let near = |a: f32, b: f32| (a - b).abs() < 0.005;
        near(self.brightness, other.brightness)
            && near(self.contrast, other.contrast)
            && near(self.saturation, other.saturation)
            && near(self.temperature, other.temperature)
            && near(self.tint, other.tint)
    }

    /// True when this does nothing, so the shader can take the cheap path.
    pub fn is_identity(&self) -> bool {
        self.brightness == 1.0
            && self.contrast == 1.0
            && self.saturation == 1.0
            && self.temperature == 0.0
            && self.tint == 0.0
    }
}

/// How a layer combines with what is beneath it (§22).
///
/// Normal is alpha-over, which is what §22 specifies and what every clip does
/// unless it is an overlay. The other three are what overlays are *for*: a
/// light leak, a dust plate or a glow is shot on black and screened on, and
/// compositing it normally would just cover the shot with a dark rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Alpha over: the layer covers what is beneath it.
    #[default]
    Normal,
    /// `src + dst - src*dst`. Never darkens: black in the layer disappears,
    /// which is how a light leak or a glow is laid on.
    Screen,
    /// `src * dst`. Never lightens: white in the layer disappears, which is how
    /// a shadow, a vignette or a texture is laid on.
    Multiply,
    /// `src + dst`. Brighter than screen and clips sooner — for sparks, flares
    /// and anything meant to blow out.
    Add,
}

impl BlendMode {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Screen, Self::Multiply, Self::Add];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Screen => "Screen",
            Self::Multiply => "Multiply",
            Self::Add => "Add",
        }
    }
}

/// The shape of a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    /// A straight edge: everything on one side of it is kept.
    ///
    /// What a split screen and a reveal are made of, and the only one of the
    /// three with no inside — its "size" is the softness of the edge alone.
    #[default]
    Linear,
    Rectangle,
    Ellipse,
}

impl MaskShape {
    pub const ALL: [Self; 3] = [Self::Linear, Self::Rectangle, Self::Ellipse];

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
        }
    }
}

/// Keep part of a clip and hide the rest.
///
/// Everything is in the clip's **own** frame, 0–1 across it, so a mask stays
/// over the part of the picture it was drawn on however the clip is afterwards
/// moved, scaled or re-timed. A mask in output coordinates would slide off its
/// subject the moment the clip was nudged.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub shape: MaskShape,
    /// The middle of the shape, 0–1 across the clip's frame. (0.5, 0.5) is the
    /// centre.
    pub center: [f32; 2],
    /// Half-width and half-height, in the same units. For a linear mask only
    /// the orientation and the feather matter.
    pub size: [f32; 2],
    /// How far the edge takes to fade out, 0–1. Zero is a hard edge.
    pub feather: f32,
    /// Turns the whole shape, in degrees, about its own centre.
    pub rotation_degrees: f32,
    /// Keep the *outside* instead: the same shape, cut the other way.
    pub invert: bool,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            shape: MaskShape::default(),
            center: [0.5, 0.5],
            size: [0.25, 0.25],
            feather: 0.05,
            rotation_degrees: 0.0,
            invert: false,
        }
    }
}

impl Mask {
    /// Hold every value inside what the shader can use.
    ///
    /// NaN turns the mask off rather than through, for the reason
    /// [`ChromaKey::clamped`] gives: a project file is a text file someone can
    /// edit, and one NaN makes every comparison in the shader false.
    pub fn clamped(self) -> Self {
        fn sane(value: f32, low: f32, high: f32) -> f32 {
            if value.is_nan() {
                low
            } else {
                value.clamp(low, high)
            }
        }

        Self {
            shape: self.shape,
            // A centre outside the frame is legitimate — half a mask hanging
            // off the edge is how a reveal starts — but not unboundedly so.
            center: self.center.map(|v| sane(v, -2.0, 3.0)),
            size: self.size.map(|v| sane(v, 0.0, 4.0)),
            feather: sane(self.feather, 0.0, 1.0),
            rotation_degrees: if self.rotation_degrees.is_finite() {
                self.rotation_degrees
            } else {
                0.0
            },
            invert: self.invert,
        }
    }
}

/// Making one colour transparent: a green screen.
///
/// The colour is what the user picked out of the picture, in the same sRGB the
/// project stores everywhere else. Everything else is a shape: how far from
/// that colour still counts as background, how quickly the edge gives way, and
/// how much of the screen's colour to take back out of what is kept.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChromaKey {
    /// The screen's colour, 0–1 per channel, sRGB-encoded.
    pub color: [f32; 3],
    /// How far a pixel's colour may be from the key and still be removed.
    ///
    /// Measured as *chromaticity* rather than as a plain colour distance, so a
    /// shadowed corner of the screen keys as readily as a lit one — the shape
    /// of the colour is what matters, not how much light fell on it.
    pub tolerance: f32,
    /// The width of the fade from removed to kept, in the same units.
    ///
    /// Zero gives a hard, aliased edge that reads as a cut-out. A little
    /// softness is what makes hair and motion blur survive.
    pub softness: f32,
    /// How much of the screen's colour to take out of what is kept, 0–1.
    ///
    /// A green screen throws green onto everything in front of it, and the
    /// rim of a subject keyed against one is green even where it is opaque.
    pub spill: f32,
}

impl Default for ChromaKey {
    fn default() -> Self {
        Self {
            // Chroma-key green, the colour most screens are painted.
            color: [0.0, 1.0, 0.0],
            tolerance: 0.12,
            softness: 0.08,
            spill: 0.6,
        }
    }
}

impl ChromaKey {
    /// The widest a tolerance or softness may be.
    ///
    /// Chromaticity distance runs past 1.0 between the furthest-apart colours
    /// there are — pure green against pure orange is about 1.06 — so this is
    /// not "wide enough to remove anything". It is deliberately short of that:
    /// a key that can take the whole picture is not a dial anyone can use, and
    /// past about this much it has stopped telling a screen from a subject.
    pub const MAX_SPREAD: f32 = 0.6;

    /// Hold every value inside the range the shader can use.
    ///
    /// `f32::clamp` passes NaN straight through, and a project file is a text
    /// file someone can edit: a single NaN in a key reaches the shader, where
    /// every comparison against it is false and the key does something nobody
    /// asked for. A broken value turns the effect *off* rather than on.
    pub fn clamped(self) -> Self {
        fn sane(value: f32, low: f32, high: f32) -> f32 {
            if value.is_nan() {
                low
            } else {
                value.clamp(low, high)
            }
        }

        Self {
            color: self.color.map(|channel| sane(channel, 0.0, 1.0)),
            tolerance: sane(self.tolerance, 0.0, Self::MAX_SPREAD),
            softness: sane(self.softness, 0.0, Self::MAX_SPREAD),
            spill: sane(self.spill, 0.0, 1.0),
        }
    }
}

/// What is drawn behind a clip that does not fill the frame (§36's reframing).
///
/// A property of the clip rather than a second clip on a lower track: it is the
/// *same picture*, and anything that moves, trims or re-times the clip has to
/// carry it. A backdrop built from a duplicate clip would come apart the first
/// time either was touched — the same argument §25 makes for attaching a
/// transition to its clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    /// Nothing: black bars where the shapes disagree.
    #[default]
    None,
    /// The clip's own picture, scaled to cover the frame and blurred.
    Blur,
}

impl Backdrop {
    pub const ALL: [Self; 2] = [Self::None, Self::Blur];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Blur => "Blur",
        }
    }
}

/// How much the blurred backdrop is softened, on [`MAX_BLUR`]'s 0–100 scale.
///
/// Heavy on purpose. A lightly blurred copy reads as a second, wrong picture
/// competing with the shot; at this strength it reads as colour and movement
/// behind it, which is what it is for.
pub const BACKDROP_BLUR: f32 = 70.0;

/// How much larger than "just covering" the backdrop is drawn.
///
/// A blur samples outside its own edges, so a backdrop scaled to exactly cover
/// the frame shows a soft, darker rim where the filter runs off the picture.
/// A tenth over hides it.
pub const BACKDROP_OVERSCAN: f32 = 1.1;

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

/// The top of the sharpen control. On the same 0–100 scale as the blur, so the
/// two read as the two ends of one idea rather than two unrelated numbers.
pub const MAX_SHARPEN: f32 = 100.0;

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

    /// §22's crop, ahead of the transform. Framing, like the transform beside
    /// it: a project written before it had none, and none is the default.
    #[serde(default)]
    pub crop: Crop,
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
    /// Sharpening, 0–100 ([`MAX_SHARPEN`]). Zero is none, and older projects
    /// have none.
    ///
    /// Not animated, unlike the blur: a blur that eases in is a transition
    /// people use, and a sharpen that eases in is not something anyone asks
    /// for. It is a property of the shot.
    #[serde(default)]
    pub sharpen: f32,

    /// RGB split, 0–100 ([`MAX_GLITCH`]): red and blue pulled apart sideways.
    /// Not animated, like the sharpen.
    #[serde(default)]
    pub rgb_split: f32,

    /// Glitch, 0–100: bands of the picture thrown sideways, different ones
    /// every frame.
    #[serde(default)]
    pub glitch: f32,

    /// The picture folded onto itself: mirrored halves, four-way, or a
    /// kaleidoscope (`crate::reflection`). Not animated.
    #[serde(default)]
    pub reflection: crate::reflection::Reflection,

    /// A colour lookup table and how strongly it applies (`crate::lut`).
    /// Not animated: a LUT is the look of the shot.
    #[serde(default)]
    pub lut: Option<crate::lut::ClipLut>,

    /// How this clip combines with what is beneath it (§22).
    #[serde(default)]
    pub blend: BlendMode,

    /// Keep part of the picture and hide the rest.
    #[serde(default)]
    pub mask: Option<Mask>,

    /// Make one colour transparent: a green screen.
    ///
    /// `None` on everything that was not shot against one, which is almost
    /// everything — and on every project written before this existed.
    #[serde(default)]
    pub chroma_key: Option<ChromaKey>,

    /// What fills the frame around a clip that does not cover it.
    ///
    /// Landscape footage in a vertical sequence is the everyday case, and the
    /// choice is bars or *something*. [`Backdrop::Blur`] is what every phone
    /// editor does: the same picture, blown up to cover and softened, so the
    /// frame is full without cropping the shot itself.
    #[serde(default)]
    pub backdrop: Backdrop,

    /// Smear the picture along the way it is moving (§45's "motion blur").
    ///
    /// Off by default: it costs several draws of the same frame, and on a shot
    /// that is not moving it costs them for nothing.
    #[serde(default)]
    pub motion_blur: bool,

    /// How the shot arrives and leaves: the animations every phone editor
    /// puts one tap away.
    ///
    /// Separate from the transitions on either side, which are about the *cut*
    /// between two clips. An animation belongs to this clip alone and happens
    /// whether or not there is anything next to it — which is what makes it the
    /// right tool for a clip standing on its own over a background.
    #[serde(default)]
    pub motion: crate::motion::ClipMotion,

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
    /// **Not in the development guide.** Re-timing appears nowhere in it: not
    /// in §10's editing list, not in §45's effects, nowhere. It is here because
    /// an editor without it is not one anyone would use, and the guide's own
    /// rules — §9's exact timebase and §74's ban on floats in position
    /// arithmetic — are what shape it. This comment exists because the code
    /// used to cite "§51" for speed in forty-four places, and §51 is the
    /// testing strategy: a citation pointing at the wrong section is worse
    /// than none, because the next person follows it.
    ///
    /// `serde(default)` gives projects written before speed existed the only
    /// answer that preserves them: normal.
    #[serde(default = "normal_speed")]
    pub speed: Rational,

    /// Played backwards: the clip's first instant shows the last frame of its
    /// material and its last instant the first (`Clip::reversed`). The source
    /// range is the same material either way, so reversing twice is exactly
    /// where it started. Defaulted, so older projects play forwards.
    #[serde(default)]
    pub reversed: bool,

    /// A colour tag for organising the edit ([`ColorLabel`]).
    #[serde(default)]
    pub color_label: ColorLabel,

    /// A transition at this clip's *end*, if any (§25).
    ///
    /// On the clip rather than on the track so that moving, trimming,
    /// splitting or pasting carries it along; see [`crate::transition`].
    #[serde(default)]
    pub transition_out: Option<Transition>,

    /// Hold one frame for the clip's whole length: a freeze frame.
    ///
    /// The frame is the one at the clip's in-point, so trimming the start
    /// chooses a different one. Everything else about the clip behaves
    /// normally — it can be moved, scaled, graded and animated, and because
    /// every instant of it asks the decoder for the same source time, it costs
    /// one decode however long it is held (exactly as a photo does).
    #[serde(default)]
    pub frozen: bool,
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

    /// What shows where no picture does (§22): the letterbox around a clip
    /// that does not fill the frame, a gap between clips, the space under the
    /// lowest track.
    ///
    /// sRGB, and black by default, which is what it always used to be. Every
    /// project written before this existed gets black from `serde(default)`,
    /// which is the same picture they had.
    #[serde(default)]
    pub background: [f32; 3],

    /// How much the edges of the frame are darkened, 0–1 (see [`MAX_VIGNETTE`]).
    ///
    /// On the whole picture rather than on a clip, because a vignette frames
    /// the *frame*: on a shot shrunk into a corner, darkening that shot's own
    /// edges would look like a mistake. Defaulted, so older projects have none.
    #[serde(default)]
    pub vignette: f32,

    /// Film grain over the whole picture, 0–1 (see [`MAX_GRAIN`]). On the
    /// frame for the same reason as the vignette: grain is the texture of the
    /// film the whole picture was shot on, not of one shot in it. It moves
    /// every frame, as real grain does. Defaulted, so older projects have none.
    #[serde(default)]
    pub grain: f32,
}

/// The top of the voice clean-up slider (`bettercut_audio::voice`).
pub const MAX_DENOISE: f32 = 100.0;

/// The top of the RGB split and glitch sliders.
pub const MAX_GLITCH: f32 = 100.0;

/// The heaviest grain: clearly textured, still a picture rather than static.
pub const MAX_GRAIN: f32 = 1.0;

/// The strongest vignette: the corners go fully dark. Past this the darkening
/// would have to reach into the middle of the picture, which stops being a
/// frame and starts being a spotlight.
pub const MAX_VIGNETTE: f32 = 1.0;

impl Default for MasterLook {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust::default(),
            blur: 0.0,
            background: [0.0, 0.0, 0.0],
            vignette: 0.0,
            grain: 0.0,
        }
    }
}

impl MasterLook {
    /// True when this changes nothing *the extra pass would do*.
    ///
    /// The background is deliberately not part of it: it is the colour the
    /// frame is cleared to, which costs nothing and happens either way — so a
    /// white background must not drag in a full-resolution scratch texture it
    /// has no use for.
    pub fn is_identity(&self) -> bool {
        self.transform.is_identity()
            && self.opacity == 1.0
            && self.color.is_identity()
            && self.blur == 0.0
            && self.vignette == 0.0
            && self.grain == 0.0
    }
}

/// A clip's appearance at one instant, with animation already applied.
///
/// The fields above are what the user set with the sliders; this is what the
/// renderer draws. They differ only where a parameter is keyed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipLook {
    /// §22's first stage, ahead of the transform. Not part of a *copied* look —
    /// see `Editor::clip_look`, which leaves framing alone deliberately.
    pub crop: Crop,
    pub transform: Transform,
    pub opacity: f32,
    pub color: ColorAdjust,
    pub blur: f32,
    /// Sharpening, 0–100. Run as an effect-graph node after the blur.
    pub sharpen: f32,
    /// A colour lookup table, run as the first effect-graph node.
    pub lut: Option<crate::lut::ClipLut>,
    /// RGB split, 0–100, and glitch, 0–100: the effect graph's last node.
    pub rgb_split: f32,
    pub glitch: f32,
    /// A reflection (`crate::reflection`): the effect graph's first node.
    pub reflection: crate::reflection::Reflection,
    /// The chroma key. Not animated — it is a choice about the footage, not
    /// a dial that moves through a shot.
    pub chroma_key: Option<ChromaKey>,
    /// The mask, in the clip's own frame.
    pub mask: Option<Mask>,
    /// §22's blend mode.
    pub blend: BlendMode,
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

    /// How fast this clip plays, exactly as on a video clip.
    ///
    /// Re-timing sound means resampling it, which changes the pitch — sped-up
    /// audio is higher, as it is on tape. That is what the effect *is*, and
    /// preserving pitch is a different feature needing a phase vocoder rather
    /// than a resampler.
    #[serde(default = "normal_speed")]
    pub speed: Rational,

    /// Played backwards, as on a video clip — sound reversed with its picture.
    #[serde(default)]
    pub reversed: bool,

    /// A colour tag for organising the edit ([`ColorLabel`]).
    #[serde(default)]
    pub color_label: ColorLabel,

    /// Linear gain, not decibels. Applied first in the §20a.4 mix graph.
    #[serde(default = "one")]
    pub gain: f32,

    /// Voice clean-up, 0–100 (`bettercut_audio::voice`): rumble removed and
    /// the background pulled down between words. Not animated.
    #[serde(default)]
    pub denoise: f32,
    #[serde(default)]
    pub enabled: bool,

    /// How long the sound takes to rise from silence at the clip's start, and
    /// to fall back to it at its end. Timeline durations; zero is none.
    /// Absent from older projects, which load with none.
    #[serde(default)]
    pub fade_in: TimelineTime,
    #[serde(default)]
    pub fade_out: TimelineTime,

    /// Volume over the clip (§24), for ducking music under a voice and for
    /// riding a level by hand.
    ///
    /// Only [`AnimatedParameter::Gain`] means anything here; the rest of the
    /// parameters are about a picture. Empty on most clips, and empty in every
    /// project written before this existed, which is exactly what
    /// `serde(default)` gives them.
    #[serde(default)]
    pub keyframes: Keyframes,
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

/// How much a fitted layer has to be scaled to **cover** the frame.
///
/// Fitting leaves bars on two sides, which is right for a shot the user wants
/// to see all of and wrong for footage being reframed — landscape material in
/// a vertical edit is the everyday case. Beside [`fit_scale`] because it is
/// the same decision seen the other way round, and the clip's scale control is
/// what carries it: the renderer needs no second mode (§46).
pub fn fill_scale(source_aspect: f32, output_aspect: f32) -> f32 {
    let (x, y) = fit_scale(source_aspect, output_aspect);
    let smallest = x.min(y);
    if smallest > f32::MIN_POSITIVE {
        1.0 / smallest
    } else {
        1.0
    }
}

/// §33's auto crop: the crop that makes a source the frame's own shape.
///
/// The third answer to "landscape footage, vertical edit", and the best one
/// where the subject is near the middle:
///
/// * **fit** shows the whole shot and leaves bars;
/// * **fill** ([`fill_scale`]) scales up until the bars are gone, which throws
///   away resolution — a 1080p shot covering a 1080×1920 frame is enlarged
///   1.8× and is no longer 1080p;
/// * **crop** takes the frame's shape out of the source at full size. Nothing
///   is enlarged and nothing is letterboxed; what is lost is the part of the
///   picture that was never going to be on screen anyway.
///
/// Centred, because the middle is where the subject is when nothing has told us
/// otherwise. §28's subject tracking is what would move it, and that needs a
/// model; this is the honest version without one.
///
/// The result always leaves the source's *other* axis untouched: making a wide
/// shot tall is done by taking from the sides, never by taking from both.
pub fn crop_to_aspect(source_aspect: f32, output_aspect: f32) -> Crop {
    if !source_aspect.is_finite()
        || !output_aspect.is_finite()
        || source_aspect <= 0.0
        || output_aspect <= 0.0
    {
        return Crop::NONE;
    }

    // How much of each axis survives. Exactly one of these is below 1: the
    // source is either wider than the frame or taller than it, and a source
    // already the right shape keeps all of both.
    let (keep_x, keep_y) = if source_aspect > output_aspect {
        (output_aspect / source_aspect, 1.0)
    } else {
        (1.0, source_aspect / output_aspect)
    };

    let side = |keep: f32| ((1.0 - keep) / 2.0).clamp(0.0, 1.0);
    Crop {
        left: side(keep_x),
        right: side(keep_x),
        top: side(keep_y),
        bottom: side(keep_y),
    }
    .clamped()
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

/// Where `at` lands when the clip spanning `span` plays the other way.
///
/// Anything found by reading a clip's material forwards — a cut, a pause, a
/// beat — lands on a reversed clip at the mirror image of where it would land
/// forwards: as far from the clip's end as it would have been from its start.
/// One reflection, shared by every detector, so they cannot disagree about
/// where a reversed clip's events are.
pub fn mirror_in(span: TimelineRange, at: TimelineTime) -> TimelineTime {
    TimelineTime::from_ticks(span.start.ticks() + span.end.ticks() - at.ticks())
}

/// A range within `span`, mirrored the same way: its start and end trade places.
pub fn mirror_range_in(span: TimelineRange, range: TimelineRange) -> TimelineRange {
    TimelineRange {
        start: mirror_in(span, range.end),
        end: mirror_in(span, range.start),
    }
}

/// The instant of `source` a clip reads `into_clip` timeline ticks after its
/// start, at `speed`, forwards or backwards.
///
/// Backwards reads from the end: the range is half-open, so the first instant
/// shown is one tick before `source.end`, the last frame of the material,
/// and a reversed clip's last instant lands on the first frame — the exact
/// mirror of the forward mapping. Every picture and sound lookup goes through
/// this, so a reversed clip cannot show one direction and sound the other.
pub fn source_time(
    source: SourceRange,
    speed: Rational,
    reversed: bool,
    into_clip: i64,
) -> MediaTime {
    let travelled = speed.scale(into_clip);
    if reversed {
        MediaTime::from_ticks(source.end.ticks() - 1 - travelled)
    } else {
        MediaTime::from_ticks(source.start.ticks() + travelled)
    }
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

    /// The clip's colour tag ([`ColorLabel`]). Every kind has one, so the
    /// edit that sets it is written once for whichever lane holds the clip.
    fn color_label(&self) -> ColorLabel;
    fn set_color_label(&mut self, label: ColorLabel);

    /// What this clip is linked to, if anything (§12).
    fn link(&self) -> Option<LinkId> {
        None
    }

    /// Replace the link (§12). Defaulted to nothing for a title, which has no
    /// sound to be tied to.
    fn set_link(&mut self, _link: Option<LinkId>) {}

    /// Whether the clip plays its material backwards.
    ///
    /// On the trait for the reason [`Self::speed`] is: trimming and splitting
    /// are generic over the clip kind, and on a reversed clip the timeline's
    /// start is the *end* of the material — trimming the start moves the
    /// source's out-point, and the left half of a split keeps the later
    /// material. Defaulted to forwards for a title, which has no material.
    fn reversed(&self) -> bool {
        false
    }

    /// Set the playback rate.
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

    /// Drop anything on this clip's *start*: the mirror of
    /// [`Self::clear_transition_out`], applied to a split's right half.
    ///
    /// Only a title has one — its entrance (`crate::motion`). A transition is
    /// stored on the outgoing clip, so a video clip's start has nothing.
    fn clear_transition_in(&mut self) {}
}

/// A colour tag on a clip, for finding your way round a long edit: the
/// interview in blue, B-roll in green, the takes still to check in red.
///
/// Organisation only — it changes nothing that is drawn or heard. Every kind
/// of clip carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorLabel {
    #[default]
    None,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Pink,
}

impl ColorLabel {
    /// Every label, in the order the menu offers them.
    pub const ALL: [Self; 8] = [
        Self::None,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Pink,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
        }
    }

    /// The colour it is drawn in, sRGB. `None` for no label.
    pub fn rgb(self) -> Option<[u8; 3]> {
        Some(match self {
            Self::None => return None,
            Self::Red => [229, 72, 77],
            Self::Orange => [240, 140, 50],
            Self::Yellow => [240, 205, 60],
            Self::Green => [80, 190, 110],
            Self::Blue => [70, 140, 235],
            Self::Purple => [160, 100, 225],
            Self::Pink => [235, 110, 180],
        })
    }
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
            fn color_label(&self) -> $crate::clip::ColorLabel {
                self.color_label
            }
            fn set_color_label(&mut self, label: $crate::clip::ColorLabel) {
                self.color_label = label;
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
    fn reversed(&self) -> bool {
        self.reversed
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
    // A fade belongs to the edge it is on, so a split keeps the fade in on
    // the left half and the fade out on the right (§25's rule for transitions).
    fn clear_transition_out(&mut self) {
        self.fade_out = TimelineTime::ZERO;
    },
    fn clear_transition_in(&mut self) {
        self.fade_in = TimelineTime::ZERO;
    },
    fn speed(&self) -> Rational {
        self.speed
    },
    fn set_speed(&mut self, speed: Rational) {
        self.speed = speed;
    },
    fn reversed(&self) -> bool {
        self.reversed
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
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            reflection: crate::Reflection::None,
            id: ClipId::new(),
            media_id,
            timeline: TimelineRange::new(start, end)?,
            source,
            crop: Crop::NONE,
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust::default(),
            blur: 0.0,
            backdrop: Backdrop::default(),
            motion: crate::motion::ClipMotion::default(),
            motion_blur: false,
            chroma_key: None,
            mask: None,
            blend: BlendMode::default(),
            keyframes: Keyframes::default(),
            link: None,
            speed: Rational::ONE,
            reversed: false,
            color_label: ColorLabel::None,
            transition_out: None,
            enabled: true,
            frozen: false,
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
            // Not animated: a crop that moved through a shot is a pan, and a
            // pan is the transform's job (§24 animates position, not framing).
            crop: self.crop,
            transform: self.transform,
            opacity: self.opacity,
            color: self.color,
            blur: self.blur,
            sharpen: self.sharpen,
            lut: self.lut.map(crate::lut::ClipLut::clamped),
            rgb_split: self.rgb_split,
            glitch: self.glitch,
            reflection: self.reflection,
            chroma_key: self.chroma_key,
            mask: self.mask,
            blend: self.blend,
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
        animated(AnimatedParameter::Temperature, &mut look.color.temperature);
        animated(AnimatedParameter::Tint, &mut look.color.tint);
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
            // Sound, not picture: a video clip has no volume of its own, and
            // its linked audio carries the envelope (§12).
            AnimatedParameter::Gain => AnimatedParameter::Gain.default_value(),
            AnimatedParameter::Brightness => self.color.brightness,
            AnimatedParameter::Contrast => self.color.contrast,
            AnimatedParameter::Saturation => self.color.saturation,
            AnimatedParameter::Temperature => self.color.temperature,
            AnimatedParameter::Tint => self.color.tint,
            AnimatedParameter::Blur => self.blur,
        }
    }

    /// Where in the source the playhead at `position` is reading.
    ///
    /// Integer throughout (§74): the offset into the clip is scaled by the
    /// clip's speed, and a frozen clip reads its in-point whatever the
    /// position.
    pub fn source_time_at(&self, position: TimelineTime) -> MediaTime {
        if self.frozen {
            return self.source.start;
        }
        let into_clip = position.ticks() - self.timeline.start.ticks();
        source_time(self.source, self.speed, self.reversed, into_clip)
    }

    /// How far through the clip `position` is, in source terms, *ignoring* a
    /// freeze.
    ///
    /// What keyframes are evaluated against, so a frozen frame can still drift
    /// or fade: the picture is held, the animation is not.
    ///
    /// On a reversed clip the animation reverses with the picture: keys stay
    /// on the frames they were set on. Anchoring them to forward progress
    /// instead would keep a zoom zooming the same way, but every trim of a
    /// reversed clip's start would slide its keys along the timeline — and an
    /// edit that moves animation the user did not touch is the worse surprise.
    pub fn progress_time_at(&self, position: TimelineTime) -> MediaTime {
        let into_clip = position.ticks() - self.timeline.start.ticks();
        source_time(self.source, self.speed, self.reversed, into_clip)
    }

    /// Where a key sits on the timeline, given keys are stored in source time.
    ///
    /// The inverse of [`Self::progress_time_at`], which is what keys are
    /// evaluated against — so a key on a 2× clip lands where the animation
    /// actually reaches it, at half the source offset.
    pub fn timeline_time_of(&self, key: MediaTime) -> TimelineTime {
        // Backwards, a key's distance is measured from the material's end —
        // the mirror of `source_time`, less the tick it reads before the end.
        let into_source = if self.reversed {
            MediaTime::from_ticks(self.source.end.ticks() - 1 - key.ticks())
        } else {
            MediaTime::from_ticks(key.ticks() - self.source.start.ticks())
        };
        TimelineTime::from_ticks(
            self.timeline.start.ticks() + timeline_ticks_for(into_source, self.speed),
        )
    }

    /// The nearest key on either side of `from`, as a timeline position.
    ///
    /// Only keys inside the clip's own range. Trimming does not delete the keys
    /// outside it — that is what lets trimming back restore them (§24) — but
    /// there is nowhere to put the playhead that would reach one, so offering
    /// to jump there would strand the user off the clip.
    ///
    /// Strictly past `from`, so pressing it twice moves twice rather than
    /// sticking on the key it just landed on.
    pub fn key_beside(&self, from: TimelineTime, forward: bool) -> Option<TimelineTime> {
        let inside = |key: &MediaTime| {
            key.ticks() >= self.source.start.ticks() && key.ticks() <= self.source.end.ticks()
        };
        let times = self
            .keyframes
            .times()
            .into_iter()
            .filter(inside)
            .map(|key| self.timeline_time_of(key));

        if forward {
            times.filter(|at| at.ticks() > from.ticks()).min()
        } else {
            times.filter(|at| at.ticks() < from.ticks()).max()
        }
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
            denoise: 0.0,
            link: None,
            speed: Rational::ONE,
            reversed: false,
            color_label: ColorLabel::None,
            enabled: true,
            fade_in: TimelineTime::ZERO,
            fade_out: TimelineTime::ZERO,
            keyframes: Keyframes::default(),
        })
    }

    /// Both fades in ticks, shrunk in proportion when together they are longer
    /// than the clip — a trimmed clip keeps the fades it was given, and they
    /// share whatever is left rather than overlapping into nonsense.
    /// Where in the source this clip is reading at a timeline instant.
    ///
    /// Scaled by the clip's speed, like every other timeline-to-source
    /// mapping — and it is source time that keyframes are anchored to (§24), so
    /// trimming the clip's start does not slide its envelope.
    pub fn source_time_at(&self, position: TimelineTime) -> MediaTime {
        let into_clip = position.ticks() - self.timeline.start.ticks();
        source_time(self.source, self.speed, self.reversed, into_clip)
    }

    /// The clip's volume at a timeline instant: its envelope where it has one,
    /// its static gain where it does not (§24).
    ///
    /// One function rather than "read the keyframes if animated, else the
    /// field" at each call site, because the two ways of asking must not be
    /// able to disagree — the preview and the export both come through here
    /// (§46).
    pub fn gain_at(&self, position: TimelineTime) -> f32 {
        match self
            .keyframes
            .value_at(AnimatedParameter::Gain, self.source_time_at(position))
        {
            Some(value) => AnimatedParameter::Gain.clamp(value),
            None => self.gain,
        }
    }

    pub fn fitted_fades(&self) -> (i64, i64) {
        let length = self.timeline.duration().ticks().max(0);
        let fade_in = self.fade_in.ticks().max(0);
        let fade_out = self.fade_out.ticks().max(0);
        let total = fade_in + fade_out;
        if total <= length || total == 0 {
            return (fade_in, fade_out);
        }
        // i128: a tick count times a tick count is past i64 for long clips.
        let share =
            |ticks: i64| (i128::from(ticks) * i128::from(length) / i128::from(total)) as i64;
        (share(fade_in), share(fade_out))
    }
}

/// The longest fade a clip may have. Longer is a volume change, which the
/// gain control is for.
pub const MAX_FADE: TimelineTime = TimelineTime::from_seconds(30);

#[cfg(test)]
mod gain_tests {
    use super::*;
    use crate::keyframe::{Interpolation, Keyframe};
    use bettercut_foundation::MediaId;

    /// A ten-second sound clip starting five seconds along the timeline, ten
    /// seconds into its file.
    fn clip() -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(5),
            SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(20)).unwrap(),
        )
        .unwrap()
    }

    fn key(seconds: i64, value: f32) -> Keyframe {
        Keyframe {
            time: MediaTime::from_seconds(seconds),
            value,
            interpolation: Interpolation::Linear,
        }
    }

    #[test]
    fn an_unanimated_clip_reads_its_own_gain() {
        let mut clip = clip();
        clip.gain = 0.6;
        assert_eq!(clip.gain_at(TimelineTime::from_seconds(7)), 0.6);
    }

    /// The envelope wins where there is one: §24's rule everywhere else.
    #[test]
    fn keyframes_override_the_static_gain() {
        let mut clip = clip();
        clip.gain = 0.6;
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 1.0));
        clip.keyframes.set(AnimatedParameter::Gain, key(20, 0.0));

        assert_eq!(clip.gain_at(TimelineTime::from_seconds(5)), 1.0);
        assert!((clip.gain_at(TimelineTime::from_seconds(10)) - 0.5).abs() < 1e-5);
        assert_eq!(clip.gain_at(TimelineTime::from_seconds(15)), 0.0);
    }

    /// §24: keys are anchored to *source* time, so trimming the clip's start
    /// slides the clip along the envelope rather than dragging the envelope
    /// with it. The duck stays over the words it was put on.
    #[test]
    fn trimming_the_start_does_not_slide_the_envelope() {
        let mut clip = clip();
        clip.keyframes.set(AnimatedParameter::Gain, key(15, 0.2));
        let before = clip.gain_at(TimelineTime::from_seconds(10));

        // Trim two seconds off the front, as a trim does: the source start
        // moves and the clip starts later.
        clip.source.start = MediaTime::from_seconds(12);
        clip.timeline.start = TimelineTime::from_seconds(7);

        assert_eq!(
            clip.gain_at(TimelineTime::from_seconds(10)),
            before,
            "the envelope moved when the clip was trimmed"
        );
    }

    /// at double speed the clip covers its source twice as fast, so the
    /// envelope arrives twice as fast too.
    #[test]
    fn speed_carries_the_envelope_with_it() {
        let mut clip = clip();
        clip.speed = Rational::new(2, 1).unwrap();
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 1.0));
        clip.keyframes.set(AnimatedParameter::Gain, key(20, 0.0));

        // Five seconds in at 2x is ten seconds of source: the end of the ramp.
        assert_eq!(clip.gain_at(TimelineTime::from_seconds(10)), 0.0);
        assert!((clip.gain_at(TimelineTime::from_seconds(7)) - 0.6).abs() < 1e-5);
    }

    /// A key outside the parameter's limits cannot make the mix louder than a
    /// slider is allowed to: the two ways of setting a value must agree.
    #[test]
    fn the_envelope_is_held_inside_the_limits() {
        let mut clip = clip();
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 100.0));
        assert_eq!(
            clip.gain_at(TimelineTime::from_seconds(6)),
            crate::track::MAX_TRACK_GAIN
        );
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

    fn sound(seconds: i64) -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(seconds)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn fades_that_fit_are_left_alone() {
        let mut clip = sound(10);
        clip.fade_in = TimelineTime::from_seconds(1);
        clip.fade_out = TimelineTime::from_seconds(2);
        assert_eq!(clip.fitted_fades(), (960_000, 1_920_000));
    }

    /// Trimmed shorter than its two fades, a clip shares its length between
    /// them in proportion rather than letting them overlap.
    #[test]
    fn fades_longer_than_the_clip_share_it() {
        let mut clip = sound(3);
        clip.fade_in = TimelineTime::from_seconds(2);
        clip.fade_out = TimelineTime::from_seconds(4);
        let (fade_in, fade_out) = clip.fitted_fades();
        assert_eq!(fade_in + fade_out, 3 * 960_000);
        assert_eq!(
            fade_in, 960_000,
            "a third of the length, as it asked for a third"
        );
    }

    #[test]
    fn a_split_keeps_each_fade_on_its_own_edge() {
        let mut track = crate::AudioTrack::new("A1");
        let mut clip = sound(10);
        clip.fade_in = TimelineTime::from_seconds(1);
        clip.fade_out = TimelineTime::from_seconds(1);
        let id = clip.id;
        track.insert(clip).unwrap();

        let outcome = track
            .split(
                id,
                TimelineTime::from_seconds(5),
                ClipId::new(),
                ClipId::new(),
            )
            .unwrap();
        let left = track.get(outcome.left).unwrap();
        let right = track.get(outcome.right).unwrap();
        assert_eq!(
            (left.fade_in, left.fade_out),
            (TimelineTime::from_seconds(1), TimelineTime::ZERO)
        );
        assert_eq!(
            (right.fade_in, right.fade_out),
            (TimelineTime::ZERO, TimelineTime::from_seconds(1))
        );
    }

    /// Landscape footage in a vertical edit, and the other way about.
    #[test]
    fn filling_the_frame_scales_past_the_fit() {
        let wide = 16.0 / 9.0;
        let tall = 9.0 / 16.0;

        // A 16:9 shot in a 9:16 frame is pillarboxed to a third of the height;
        // covering means scaling by (16/9)/(9/16).
        assert!((fill_scale(wide, tall) - wide / tall).abs() < 1e-4);
        assert!((fill_scale(tall, wide) - wide / tall).abs() < 1e-4);
        // A square shot in a 16:9 frame fills it at 16/9.
        assert!((fill_scale(1.0, wide) - wide).abs() < 1e-4);
        // Same shape: already filling.
        assert!((fill_scale(wide, wide) - 1.0).abs() < 1e-6);
        // Nonsense in, no panic and no infinity out.
        assert_eq!(fill_scale(0.0, wide), 1.0);
    }
}

#[cfg(test)]
mod look_strength_tests {
    use super::*;

    const PUNCHY: ColorAdjust = ColorAdjust {
        brightness: 1.02,
        contrast: 1.25,
        saturation: 1.25,
        temperature: 0.0,
        tint: 0.0,
    };

    /// A look whose largest move is on the white balance, where "no change" is
    /// zero rather than one.
    ///
    /// `strength_towards` picks the axis that moves furthest and divides by how
    /// far it goes; written against the multipliers alone it measured every
    /// axis from 1.0, so a temperature of 0.45 read as *less* movement than a
    /// contrast of 1.05 and the strength came back off the wrong axis entirely.
    const WARM: ColorAdjust = ColorAdjust {
        brightness: 1.02,
        contrast: 1.05,
        saturation: 1.08,
        temperature: 0.45,
        tint: 0.05,
    };

    #[test]
    fn strength_zero_is_the_picture_untouched() {
        assert_eq!(
            ColorAdjust::IDENTITY.lerp(PUNCHY, 0.0),
            ColorAdjust::IDENTITY
        );
    }

    #[test]
    fn strength_one_is_the_look_as_written() {
        assert_eq!(ColorAdjust::IDENTITY.lerp(PUNCHY, 1.0), PUNCHY);
    }

    #[test]
    fn half_strength_is_half_the_grade() {
        let half = ColorAdjust::IDENTITY.lerp(PUNCHY, 0.5);
        assert!((half.contrast - 1.125).abs() < 1e-5);
        assert!((half.saturation - 1.125).abs() < 1e-5);
        assert!((half.brightness - 1.01).abs() < 1e-5);
    }

    /// Every strength comes back out again: this is what lets the interface
    /// keep no state of its own about which look is on.
    #[test]
    fn a_strength_is_recovered_from_the_grade() {
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let graded = ColorAdjust::IDENTITY.lerp(PUNCHY, t);
            let found = graded.strength_towards(PUNCHY).expect("on the line");
            assert!(
                (found - t).abs() < 0.01,
                "applied at {t} and read back as {found}"
            );
        }
    }

    /// A grade nobody made from this look is not attributed to it.
    #[test]
    fn an_unrelated_grade_is_not_on_the_line() {
        let hand_made = ColorAdjust {
            brightness: 1.02,
            contrast: 0.6,
            saturation: 1.9,
            temperature: 0.0,
            tint: 0.0,
        };
        assert_eq!(hand_made.strength_towards(PUNCHY), None);
    }

    /// Sharing one number with a look is not being that look: a grade that
    /// happens to have the right contrast but nothing else must not light the
    /// button up.
    #[test]
    fn matching_one_axis_is_not_enough() {
        let coincidence = ColorAdjust {
            brightness: 1.0,
            contrast: 1.25,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
        };
        assert_eq!(coincidence.strength_towards(PUNCHY), None);
    }

    /// Black and white is the awkward one: its saturation goes to zero while
    /// its brightness does not move at all.
    #[test]
    fn a_look_that_moves_one_axis_still_inverts() {
        let mono = ColorAdjust {
            brightness: 1.0,
            contrast: 1.08,
            saturation: 0.0,
            temperature: 0.0,
            tint: 0.0,
        };
        let half = ColorAdjust::IDENTITY.lerp(mono, 0.5);
        assert!((half.saturation - 0.5).abs() < 1e-5);
        let found = half.strength_towards(mono).expect("on the line");
        assert!((found - 0.5).abs() < 0.01, "read back as {found}");
    }

    /// The same round trip on a look that lives mostly on the white balance.
    #[test]
    fn a_strength_is_recovered_from_a_white_balance_look() {
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let graded = ColorAdjust::IDENTITY.lerp(WARM, t);
            let found = graded.strength_towards(WARM).expect("on the line");
            assert!(
                (found - t).abs() < 0.01,
                "applied at {t} and read back as {found}"
            );
        }
    }

    /// A warm grade is not a cool one, however alike the other axes are.
    #[test]
    fn a_grade_warmed_the_other_way_is_not_this_look() {
        let cooled = ColorAdjust {
            temperature: -WARM.temperature,
            tint: -WARM.tint,
            ..WARM
        };
        assert_eq!(cooled.strength_towards(WARM), None);
    }

    #[test]
    fn nothing_is_at_full_strength_towards_the_identity() {
        assert_eq!(
            ColorAdjust::IDENTITY.strength_towards(ColorAdjust::IDENTITY),
            Some(0.0)
        );
        assert_eq!(PUNCHY.strength_towards(ColorAdjust::IDENTITY), None);
    }
}

#[cfg(test)]
mod key_navigation_tests {
    use super::*;
    use crate::keyframe::{AnimatedParameter, Interpolation, Keyframe};

    fn secs(n: i64) -> TimelineTime {
        TimelineTime::from_seconds(n)
    }

    fn media(n: i64) -> MediaTime {
        MediaTime::from_seconds(n)
    }

    /// A clip playing 10–20 s of a file, sitting at 100 s on the timeline.
    fn clip_with_keys(speed: Rational) -> VideoClip {
        let mut clip = VideoClip::new(
            MediaId::new(),
            secs(100),
            SourceRange::new(media(10), media(20)).expect("range"),
        )
        .expect("clip");
        clip.speed = speed;
        for at in [media(12), media(16)] {
            clip.keyframes.set(
                AnimatedParameter::Opacity,
                Keyframe::new(at, 0.5, Interpolation::Linear),
            );
        }
        clip
    }

    /// Keys are stored in source time and the playhead is in timeline time, so
    /// the mapping is the whole feature.
    #[test]
    fn a_key_maps_to_where_the_animation_reaches_it() {
        let clip = clip_with_keys(Rational::ONE);
        assert_eq!(clip.timeline_time_of(media(12)), secs(102));
        assert_eq!(clip.timeline_time_of(media(16)), secs(106));
    }

    /// At double speed the clip covers its source in half the time, so a key
    /// two seconds into the footage arrives one second in.
    #[test]
    fn speed_moves_the_keys_with_the_footage() {
        let clip = clip_with_keys(Rational::new(2, 1).expect("ratio"));
        assert_eq!(clip.timeline_time_of(media(12)), secs(101));
        assert_eq!(clip.timeline_time_of(media(16)), secs(103));
    }

    #[test]
    fn jumping_finds_the_next_key_and_then_stops() {
        let clip = clip_with_keys(Rational::ONE);

        assert_eq!(clip.key_beside(secs(100), true), Some(secs(102)));
        assert_eq!(clip.key_beside(secs(102), true), Some(secs(106)));
        assert_eq!(clip.key_beside(secs(106), true), None, "ran off the end");

        assert_eq!(clip.key_beside(secs(110), false), Some(secs(106)));
        assert_eq!(clip.key_beside(secs(106), false), Some(secs(102)));
        assert_eq!(clip.key_beside(secs(102), false), None);
    }

    /// Strictly past the playhead, or pressing it twice would land on the same
    /// key and the user would think the control was broken.
    #[test]
    fn a_key_under_the_playhead_is_not_where_it_jumps_to() {
        let clip = clip_with_keys(Rational::ONE);
        assert_ne!(clip.key_beside(secs(102), true), Some(secs(102)));
        assert_ne!(clip.key_beside(secs(102), false), Some(secs(102)));
    }

    /// §24: trimming does not delete the keys outside the trim, so that
    /// trimming back restores them. There is nowhere to put the playhead that
    /// reaches one, so jumping must not offer it.
    #[test]
    fn keys_outside_the_trimmed_range_are_not_jumped_to() {
        let mut clip = clip_with_keys(Rational::ONE);
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(media(5), 0.5, Interpolation::Linear),
        );
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(media(30), 0.5, Interpolation::Linear),
        );

        assert_eq!(
            clip.key_beside(secs(100), false),
            None,
            "jumped back to a key before the clip's in-point"
        );
        assert_eq!(
            clip.key_beside(secs(106), true),
            None,
            "jumped past the out-point"
        );
    }

    #[test]
    fn a_clip_with_no_keys_has_nowhere_to_jump() {
        let clip = VideoClip::new(
            MediaId::new(),
            secs(100),
            SourceRange::new(media(10), media(20)).expect("range"),
        )
        .expect("clip");
        assert_eq!(clip.key_beside(secs(105), true), None);
        assert_eq!(clip.key_beside(secs(105), false), None);
    }
}
