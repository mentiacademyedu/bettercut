//! Captions: subtitle tracks, import and export (§27, Milestone 10).
//!
//! A caption is a span of time and what is said in it. This crate owns the
//! *formats* — reading and writing the files everyone else's tools produce —
//! and nothing about how they are drawn: that is §26's text system, and a
//! caption on the timeline is an ordinary text clip.
//!
//! Two formats, because between them they cover what transcription tools emit:
//! SubRip (`.srt`) and WebVTT (`.vtt`).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod error;
pub mod file;
pub mod segment;
pub mod srt;
pub mod vtt;

pub use error::CaptionError;
pub use file::{Format, read, write};
pub use segment::{CaptionSegment, CaptionWord, MIN_DURATION, tidy};
pub use srt::Parsed;
