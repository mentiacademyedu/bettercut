//! The timeline data model: sequences, tracks, clips (§8).
//!
//! This crate knows nothing about egui, FFmpeg, or wgpu — §86 requires that the
//! timeline not know the UI exists. It is pure data plus the invariants that
//! keep it consistent.
//!
//! Milestone 1 provides the model and its queries. Editing operations (move,
//! trim, split, ripple delete) land in Milestone 3 per §85.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod clip;
pub mod error;
pub mod keyframe;
pub mod marker;
pub mod motion;
pub mod sequence;
pub mod snap;
pub mod text;
pub mod track;
pub mod transition;

pub use clip::{
    AudioClip, Clip, ClipLook, ColorAdjust, MAX_BLUR, MasterLook, SourceRange, TimelineRange,
    Transform, Vec2, VideoClip,
};
pub use clip::{
    MAX_FADE, MAX_SPEED, MIN_SPEED, clamped_speed, fill_scale, fit_scale, natural_size_transform,
    timeline_ticks_for,
};
pub use error::TimelineError;
pub use keyframe::{
    AnimatedParameter, Interpolation, Keyframe, KeyframeTrack, Keyframes, Movement, ZOOM_AMOUNT,
};
pub use marker::{MAX_MARKERS, Marker};
pub use motion::{
    DEFAULT_MOTION, MAX_MOTION, MIN_MOTION, Motion, MotionKind, TextAnimation, TextLook,
};
pub use sequence::{Resolution, Sequence, TrackKind};
pub use snap::{SnapKind, SnapTarget};
pub use text::{DEFAULT_DURATION as DEFAULT_TEXT_DURATION, TextClip, TextTrack};
pub use track::{AudioTrack, MAX_TRACK_GAIN, SplitOutcome, Track, VideoTrack};
pub use transition::{
    DEFAULT_DURATION as DEFAULT_TRANSITION, MIN_DURATION as MIN_TRANSITION, Transition,
    TransitionKind,
};
