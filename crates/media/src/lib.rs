//! Media assets, colour metadata, and the decoder abstraction (§3, §12, §21a).
//!
//! This is the only crate that will reference an FFmpeg binding (§86). At
//! Milestone 1 it contains no FFmpeg dependency at all — §83 defers that to
//! Milestone 2 — but the traits FFmpeg will implement are defined here now, so
//! the rest of the application can be written against them.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod asset;
pub mod color;
pub mod decoder;
pub mod error;
pub mod ffmpeg;
pub mod generated;
pub mod generated_sound;
pub mod image_sequence;
pub mod proxy;

pub use asset::{MAX_STILL_EDGE, MediaAsset, MediaKind, STILL_DURATION, fit_within};
pub use color::{ColorMatrix, ColorMetadata, ColorPrimaries, ColorRange, TransferFunction};
pub use decoder::{
    AudioBuffer, CancellationToken, FrameStorage, MediaDecoder, MediaProber, NeverCancelled,
    SeekMode, VideoFrame,
};
pub use error::MediaError;
pub use ffmpeg::{
    AUDIO_BITRATES_KBPS, ChapterMark, DEFAULT_AUDIO_BITRATE, EncodeTarget, EncoderChoice,
    EncoderKind, EncoderProbe, ExportFormat, FfmpegDecoder, FfmpegProber, MAX_AUDIO_BITRATE,
    MIN_AUDIO_BITRATE, RateControl, VideoCodec, VideoWriter, generate_proxy, probe_all,
    probe_chapters,
};
pub use generated::Generated;
pub use generated_sound::{GeneratedSound, LINE_UP_DB, LINE_UP_HZ};
pub use image_sequence::{
    ImageSequence, MAX_SEQUENCE_FRAMES, Numbered, numbered, pattern_for, sequence_at,
};
pub use proxy::{ProxyAsset, ProxyResolution, ProxySpec, ProxyStatus};
