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
pub mod sequence;
pub mod snap;
pub mod track;

pub use clip::{
    AudioClip, Clip, ClipLook, ColorAdjust, MAX_BLUR, SourceRange, TimelineRange, Transform, Vec2,
    VideoClip,
};
pub use error::TimelineError;
pub use keyframe::{AnimatedParameter, Interpolation, Keyframe, KeyframeTrack, Keyframes};
pub use sequence::{Resolution, Sequence, TrackKind};
pub use snap::{SnapKind, SnapTarget};
pub use track::{AudioTrack, SplitOutcome, Track, VideoTrack};
