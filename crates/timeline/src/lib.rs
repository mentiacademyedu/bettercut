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
pub mod burn_in;
pub mod chapters;
pub mod clip;
pub mod colour_match;
pub mod corner_pin;
pub mod counter;
pub mod curves;
pub mod error;
pub mod karaoke;
pub mod keyframe;
pub mod lut;
pub mod marker;
pub mod marker_file;
pub mod motion;
pub mod reflection;
pub mod render;
pub mod sequence;
pub mod shake;
pub mod snap;
pub mod text;
pub mod track;
pub mod track_volume;
pub mod transition;
pub mod visualizer;
pub mod watermark;

pub use adjustment::{
    AdjustmentClip, AdjustmentLook, AdjustmentTrack,
    DEFAULT_DURATION as DEFAULT_ADJUSTMENT_DURATION,
};
pub use burn_in::{BurnIn, MAX_BURN_IN_SIZE, MIN_BURN_IN_SIZE};
pub use chapters::{ChapterProblem, Chapters, chapter_list, chapter_ranges, chapter_starts};
pub use clip::MAX_CLIP_NAME;
pub use clip::{
    AudioClip, BACKDROP_BLUR, BACKDROP_OVERSCAN, BAR_PRESETS, Backdrop, BlendMode, Border,
    ChannelMode, ChromaKey, Clip, ClipEq, ClipLook, ClipSpace, ColorAdjust, ColorLabel,
    ColorWheels, Crop, EQ_HIGH_CUT_MAX, EQ_HIGH_CUT_MIN, EQ_LOW_CUT_MAX, EQ_LOW_CUT_MIN,
    EQ_PRESENCE_MAX, FadeShape, FlipAxis, HslSecondary, LumaKey, MAX_BARS_ASPECT, MAX_BLUR,
    MAX_BORDER_WIDTH, MAX_DENOISE, MAX_GLITCH, MAX_GRAIN, MAX_PROGRESS_BAR, MAX_SHADOW_DISTANCE,
    MAX_SHADOW_SOFTNESS, MAX_SHARPEN, MAX_VIGNETTE, MIN_CROP_REMAINING, Mask, MaskShape,
    MasterLook, ProgressBar, Shadow, SourceRange, SpaceKind, TimelineRange, Transform, Vec2,
    VideoClip, bar_height,
};
pub use clip::{
    MAX_CROSSFADE, MAX_FADE, MAX_SPEED, MIN_SPEED, SPEED_PRESETS, clamped_speed, crop_to_aspect,
    crossfade_halves, fill_scale, fit_scale, mirror_in, mirror_range_in, natural_size_transform,
    source_time, speed_label, timeline_ticks_for,
};
pub use corner_pin::{CornerPin, MAX_CORNER_REACH};
pub use counter::{CountDirection, CountFormat, Counter};
pub use error::TimelineError;
pub use keyframe::{
    AnimatedParameter, Interpolation, Keyframe, KeyframeTrack, Keyframes, Movement,
    MovementStrength, PAN_SCALE, ZOOM_AMOUNT,
};
pub use lut::{ClipLut, CubeLut, LutError, MAX_LUT_SIZE, load_cube_file, parse_cube};
pub use marker::{MAX_MARKERS, Marker};
pub use marker_file::{
    MarkerFileFormat, frame_timecode, marker_file, markers_csv, markers_edl, parse_csv,
    parse_timecode,
};
pub use motion::{
    ClipMotion, DEFAULT_MOTION, LoopMotion, MAX_MOTION, MIN_MOTION, Motion, MotionKind, Scroll,
    TextAnimation, TextLook,
};
pub use reflection::{KALEIDOSCOPE_SEGMENTS, Reflection};
pub use render::RenderedRange;
pub use sequence::{ClipMark, ClipNote, ClipSpan, Resolution, Sequence, TrackKind};
pub use shake::{IMPACT_SETTLE, SHAKE_STEP, ShakeStrength};
pub use snap::{SnapKind, SnapTarget};
pub use text::{DEFAULT_DURATION as DEFAULT_TEXT_DURATION, TextClip, TextTrack};
pub use track::{AudioTrack, MAX_TRACK_GAIN, SplitOutcome, Track, VideoTrack, track_plays};
pub use track_volume::{MAX_VOLUME, TrackVolume, VolumePoint};
pub use transition::{
    DEFAULT_DURATION as DEFAULT_TRANSITION, MIN_DURATION as MIN_TRANSITION, Transition,
    TransitionKind,
};
