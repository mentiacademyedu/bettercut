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
pub mod proxy;

pub use asset::{MediaAsset, MediaKind};
pub use color::{ColorMatrix, ColorMetadata, ColorPrimaries, ColorRange, TransferFunction};
pub use decoder::{
    AudioBuffer, CancellationToken, FrameStorage, MediaDecoder, MediaProber, NeverCancelled,
    SeekMode, VideoFrame,
};
pub use error::MediaError;
pub use ffmpeg::{
    EncodeTarget, EncoderChoice, EncoderKind, EncoderProbe, ExportFormat, FfmpegDecoder,
    FfmpegProber, VideoWriter, generate_proxy, probe_all,
};
pub use proxy::{ProxyAsset, ProxyResolution, ProxySpec, ProxyStatus};
