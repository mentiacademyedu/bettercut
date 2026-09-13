//! Shared primitives for the editor: exact time, rational frame rates, and IDs.
//!
//! See `README.md` for why this crate exists and how it fits §86's layering.
//!
//! This crate has no I/O, no threads, and no knowledge of FFmpeg, wgpu, or egui.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod ids;
pub mod rational;
pub mod time;

pub use ids::{ClipId, EffectId, LinkId, LutId, MarkerId, MediaId, ProjectId, SequenceId, TrackId};
pub use rational::{FrameRate, Rational};
pub use time::{
    AUDIO_SAMPLE_RATE, MediaTime, TICKS_PER_AUDIO_SAMPLE, TICKS_PER_SECOND, TimelineTime,
    ticks_per_frame,
};
