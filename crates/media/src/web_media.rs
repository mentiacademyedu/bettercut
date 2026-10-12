//! The web build's stand-in for [`crate::ffmpeg`]: the same names, none of
//! FFmpeg.
//!
//! FFmpeg is a native library and does not link into WebAssembly, so in a
//! browser `crate::ffmpeg` is this file (`lib.rs` swaps it in by path).
//! Everything else is written against the `MediaProber` / `MediaDecoder`
//! traits and the plain types in [`crate::codecs`], so it compiles unchanged;
//! here, every file is refused, cleanly, with an error that says why.
//!
//! What still works in a browser is everything bettercut makes itself —
//! colours, gradients, titles, shapes, generated sound. Decoding real video
//! here is the browser's own codecs' job (WebCodecs), behind the same traits.

use std::path::Path;

use bettercut_foundation::{FrameRate, MediaTime};

use crate::decoder::{AudioBuffer, CancellationToken, MediaDecoder, MediaProber, SeekMode, VideoFrame};
use crate::error::MediaError;
use crate::MediaAsset;

pub use crate::codecs::{
    AUDIO_BITRATES_KBPS, ChapterMark, DEFAULT_AUDIO_BITRATE, EncodeTarget, EncoderChoice,
    EncoderKind, EncoderProbe, ExportFormat, MAX_AUDIO_BITRATE, MIN_AUDIO_BITRATE, PRORES,
    RateControl, VideoCodec,
};

/// The rate everything is mixed at: the same as the desktop build's.
pub const INTERNAL_SAMPLE_RATE: i32 = 48_000;

/// How far along a long job is, from 0 to 1.
pub type ProgressFn<'a> = &'a dyn Fn(f32);

fn unsupported(path: &Path) -> MediaError {
    MediaError::UnsupportedFormat(path.to_path_buf())
}

/// Opens nothing yet: every file is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct FfmpegProber;

impl MediaProber for FfmpegProber {
    fn probe(&self, path: &Path) -> Result<MediaAsset, MediaError> {
        Err(unsupported(path))
    }
}

impl FfmpegProber {
    pub fn probe_image_sequence(
        &self,
        path: &Path,
        _rate: FrameRate,
    ) -> Result<MediaAsset, MediaError> {
        Err(unsupported(path))
    }
}

/// A decoder that never opens.
#[derive(Debug, Default)]
pub struct FfmpegDecoder;

impl FfmpegDecoder {
    pub fn new(_threads: u32) -> Result<Self, MediaError> {
        Ok(Self)
    }
    pub fn width(&self) -> u32 {
        0
    }
    pub fn height(&self) -> u32 {
        0
    }
    pub fn has_video(&self) -> bool {
        false
    }
    pub fn has_audio(&self) -> bool {
        false
    }
}

impl MediaDecoder for FfmpegDecoder {
    fn open(&mut self, asset: &MediaAsset) -> Result<(), MediaError> {
        Err(unsupported(&asset.path))
    }
    fn seek(&mut self, _timestamp: MediaTime, _mode: SeekMode) -> Result<(), MediaError> {
        Err(MediaError::NotOpen)
    }
    fn decode_frame(
        &mut self,
        _cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError> {
        Err(MediaError::NotOpen)
    }
    fn decode_frame_at(
        &mut self,
        _target: MediaTime,
        _cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError> {
        Err(MediaError::NotOpen)
    }
    fn position(&self) -> Option<MediaTime> {
        None
    }
    fn frame_duration(&self) -> MediaTime {
        MediaTime::ZERO
    }
    fn decode_audio(
        &mut self,
        _cancel: &dyn CancellationToken,
    ) -> Result<Option<AudioBuffer>, MediaError> {
        Err(MediaError::NotOpen)
    }
    fn duration(&self) -> MediaTime {
        MediaTime::ZERO
    }
}

/// No proxies in a browser: there is nothing to make them from yet.
pub fn generate_proxy(
    asset: &MediaAsset,
    _output: &Path,
    _height: u32,
    _threads: u32,
    _cancel: &dyn CancellationToken,
    _progress: ProgressFn<'_>,
) -> Result<(), MediaError> {
    Err(unsupported(&asset.path))
}

/// No encoders found: exporting a file is not there yet in a browser.
pub fn probe_all(_target: EncodeTarget) -> Vec<EncoderProbe> {
    Vec::new()
}

/// No file to read chapters from.
pub fn probe_chapters(path: &Path) -> Result<Vec<ChapterMark>, MediaError> {
    Err(unsupported(path))
}

/// A writer that never opens.
pub struct VideoWriter {
    choice: EncoderChoice,
}

impl VideoWriter {
    pub fn create(path: &Path, format: ExportFormat) -> Result<Self, MediaError> {
        Self::create_with_chapters(path, format, &[])
    }
    pub fn create_with_chapters(
        path: &Path,
        _format: ExportFormat,
        _chapters: &[ChapterMark],
    ) -> Result<Self, MediaError> {
        Err(unsupported(path))
    }
    pub fn encoder(&self) -> EncoderChoice {
        self.choice
    }
    pub fn frames_written(&self) -> i64 {
        0
    }
    pub fn push_frame(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        Err(MediaError::NotOpen)
    }
    pub fn push_audio(&mut self, _planes: &[Vec<f32>]) -> Result<(), MediaError> {
        Err(MediaError::NotOpen)
    }
    pub fn finish(self) -> Result<(), MediaError> {
        Err(MediaError::NotOpen)
    }
}

/// Choosing a file's streams: plain logic, the same as the desktop build's.
pub mod streams {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StreamInfo {
        pub index: i32,
        pub kind: StreamKind,
        pub decodable: bool,
        pub default: bool,
        pub cover: bool,
        pub part: bool,
        pub area: u64,
        pub channels: u16,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum StreamKind {
        Video,
        Audio,
        Other,
    }

    /// No streams are ever read in a browser yet.
    pub fn choose(_streams: &[StreamInfo]) -> (Option<i32>, Option<i32>) {
        (None, None)
    }
}
