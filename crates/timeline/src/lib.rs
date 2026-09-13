//! The timeline data model: sequences, tracks, clips (§8).
//!
//! This crate knows nothing about egui, FFmpeg, or wgpu — §86 requires that the
//! timeline not know the UI exists. It is pure data plus the invariants that
//! keep it consistent.
//!
//! Milestone 1 provides the model and its queries. Editing operations (move,
//! trim, split, ripple delete) land in Milestone 3 per §85.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adjustment;
pub mod chapters;
pub mod clip;
pub mod counter;
pub mod error;
pub mod keyframe;
pub mod lut;
pub mod marker;
pub mod motion;
pub mod reflection;
pub mod sequence;
pub mod shake;
pub mod snap;
pub mod text;
pub mod track;
pub mod transition;

pub use adjustment::{
    AdjustmentClip, AdjustmentLook, AdjustmentTrack,
    DEFAULT_DURATION as DEFAULT_ADJUSTMENT_DURATION,
};
pub use chapters::{ChapterProblem, Chapters, chapter_list};
pub use clip::{
    AudioClip, BACKDROP_BLUR, BACKDROP_OVERSCAN, Backdrop, BlendMode, ChromaKey, Clip, ClipLook,
    ColorAdjust, ColorLabel, Crop, FlipAxis, MAX_BLUR, MAX_DENOISE, MAX_GLITCH, MAX_GRAIN,
    MAX_SHARPEN, MAX_VIGNETTE, MIN_CROP_REMAINING, Mask, MaskShape, MasterLook, SourceRange,
    TimelineRange, Transform, Vec2, VideoClip,
};
pub use clip::{
    MAX_FADE, MAX_SPEED, MIN_SPEED, clamped_speed, crop_to_aspect, fill_scale, fit_scale,
    mirror_in, mirror_range_in, natural_size_transform, source_time, timeline_ticks_for,
};
pub use counter::{CountDirection, CountFormat, Counter};
pub use error::TimelineError;
pub use keyframe::{
    AnimatedParameter, Interpolation, Keyframe, KeyframeTrack, Keyframes, Movement, ZOOM_AMOUNT,
};
pub use lut::{ClipLut, CubeLut, LutError, MAX_LUT_SIZE, load_cube_file, parse_cube};
pub use marker::{MAX_MARKERS, Marker};
pub use motion::{
    ClipMotion, DEFAULT_MOTION, MAX_MOTION, MIN_MOTION, Motion, MotionKind, TextAnimation, TextLook,
};
pub use reflection::{KALEIDOSCOPE_SEGMENTS, Reflection};
pub use sequence::{ClipNote, ClipSpan, Resolution, Sequence, TrackKind};
pub use shake::{IMPACT_SETTLE, SHAKE_STEP, ShakeStrength};
pub use snap::{SnapKind, SnapTarget};
pub use text::{DEFAULT_DURATION as DEFAULT_TEXT_DURATION, TextClip, TextTrack};
pub use track::{AudioTrack, MAX_TRACK_GAIN, SplitOutcome, Track, VideoTrack, track_plays};
pub use transition::{
    DEFAULT_DURATION as DEFAULT_TRANSITION, MIN_DURATION as MIN_TRANSITION, Transition,
    TransitionKind,
};
