//! The web build's stand-in for [`crate::ffmpeg`]: the same names, the
//! browser's own decoders.
//!
//! FFmpeg is a native library and does not link into WebAssembly, so in a
//! browser `crate::ffmpeg` is this file (`lib.rs` swaps it in by path).
//! Everything else is written against the `MediaProber` / `MediaDecoder`
//! traits and the plain types in [`crate::codecs`], so it compiles unchanged.
//!
//! Files reach a browser as `File`s, not paths: the page hands each one to
//! `js/browser_media.js`, which names it `/browser/<n>/<file name>` and opens
//! it with the browser's own decoders (a `<video>` element, or
//! `createImageBitmap` for a photo). That name is the asset's path, and
//! probing and decoding read through it. Decoding is asynchronous there, so
//! a frame not decoded yet is an error for now — the track is skipped for one
//! redraw — or the last frame there was, which [`VideoFrame::timestamp`]
//! marks as not the one asked for, so it is never kept as that one. When the
//! right frame arrives, [`browser::changes`] moves on and the app draws again.
//!
//! Sound is decoded whole by the page when a file is opened, at 48 kHz, and
//! read from there in blocks.
//!
//! Not here yet: export, proxies, chapters.

use std::path::Path;

use bettercut_foundation::{FrameRate, MediaTime, TICKS_PER_AUDIO_SAMPLE, TICKS_PER_SECOND};
use wasm_bindgen::prelude::*;

use crate::decoder::{
    AudioBuffer, CancellationToken, FrameStorage, MediaDecoder, MediaProber, SeekMode, VideoFrame,
};
use crate::error::MediaError;
use crate::{ColorMetadata, MediaAsset, MediaKind};

pub use crate::codecs::{
    AUDIO_BITRATES_KBPS, ChapterMark, DEFAULT_AUDIO_BITRATE, EncodeTarget, EncoderChoice,
    EncoderKind, EncoderProbe, ExportFormat, MAX_AUDIO_BITRATE, MIN_AUDIO_BITRATE, PRORES,
    RateControl, VideoCodec,
};

#[wasm_bindgen(module = "/js/browser_media.js")]
extern "C" {
    fn bc_take_ready() -> js_sys::Array;
    fn bc_drop_point() -> JsValue;
    fn bc_changes() -> u32;
    fn bc_busy() -> bool;
    fn bc_pick_files(accept: &str);
    fn bc_probe(path: &str) -> JsValue;
    fn bc_frame(path: &str, seconds: f64) -> JsValue;
    fn bc_poster(path: &str) -> JsValue;
    fn bc_sound(path: &str, from: f64, frames: u32) -> JsValue;
    fn bc_filmstrip(path: &str) -> JsValue;
    fn bc_peaks(path: &str) -> JsValue;
}

/// The page's side of files: what the app calls each frame.
pub mod browser {
    use super::*;

    /// Files given to the page whose details are now known, ready to import.
    pub fn take_ready() -> Vec<std::path::PathBuf> {
        bc_take_ready()
            .iter()
            .filter_map(|v| v.as_string())
            .map(std::path::PathBuf::from)
            .collect()
    }

    /// Where the last files were dropped, in page pixels — `None` when they
    /// came from the file picker.
    pub fn drop_point() -> Option<(f32, f32)> {
        let point = bc_drop_point();
        if point.is_null() || point.is_undefined() {
            return None;
        }
        Some((number(&point, "x") as f32, number(&point, "y") as f32))
    }

    /// A count that moves on whenever a frame or a file arrives: when it has
    /// changed since the last look, the preview is drawn again.
    pub fn changes() -> u32 {
        bc_changes()
    }

    /// Whether anything is still loading or decoding, so the app keeps
    /// looking for it.
    pub fn busy() -> bool {
        bc_busy()
    }

    /// A small picture made by the page: RGBA, `width` by `height`.
    pub struct Picture {
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
    }

    fn picture(found: &JsValue) -> Option<Picture> {
        if found.is_undefined() || found.is_null() {
            return None;
        }
        Some(Picture {
            width: number(found, "width") as u32,
            height: number(found, "height") as u32,
            rgba: js_sys::Uint8ClampedArray::from(field(found, "data")).to_vec(),
        })
    }

    /// The media browser's thumbnail for a file, once the page has made it.
    pub fn poster(path: &std::path::Path) -> Option<Picture> {
        picture(&bc_poster(&path_str(path)))
    }

    /// The timeline's filmstrip sheet for a file, once the page has made it:
    /// the desktop's layout, `bettercut_playback::TILES` tiles side by side.
    pub fn filmstrip(path: &std::path::Path) -> Option<Picture> {
        picture(&bc_filmstrip(&path_str(path)))
    }

    /// The timeline's waveform for a file, once the page has made it: the
    /// lowest and highest of each of `bettercut_cache::PEAKS_PER_SECOND`
    /// buckets a second, interleaved.
    pub fn peaks(path: &std::path::Path) -> Option<Vec<i8>> {
        let found = bc_peaks(&path_str(path));
        if found.is_undefined() || found.is_null() {
            return None;
        }
        Some(js_sys::Int8Array::from(found).to_vec())
    }

    /// Open the browser's file picker. What is chosen arrives later, through
    /// [`take_ready`].
    pub fn pick_files(extensions: &[&str]) {
        let accept: Vec<String> = extensions.iter().map(|e| format!(".{e}")).collect();
        bc_pick_files(&accept.join(","));
    }
}

fn field(object: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(object, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}

fn number(object: &JsValue, name: &str) -> f64 {
    field(object, name).as_f64().unwrap_or(0.0)
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn seconds_of(time: MediaTime) -> f64 {
    time.ticks() as f64 / TICKS_PER_SECOND as f64
}

/// Sound read per `decode_audio`: 100 ms at 48 kHz.
const SOUND_BLOCK: u32 = 4800;

/// The rate assumed for browser video: a `<video>` element does not say.
const ASSUMED_RATE: FrameRate = FrameRate::FPS_30;

/// The rate everything is mixed at: the same as the desktop build's.
pub const INTERNAL_SAMPLE_RATE: i32 = 48_000;

/// How far along a long job is, from 0 to 1.
pub type ProgressFn<'a> = &'a dyn Fn(f32);

fn unsupported(path: &Path) -> MediaError {
    MediaError::UnsupportedFormat(path.to_path_buf())
}

/// Reads what the browser found out about a file the page was given.
#[derive(Debug, Clone, Copy, Default)]
pub struct FfmpegProber;

impl MediaProber for FfmpegProber {
    fn probe(&self, path: &Path) -> Result<MediaAsset, MediaError> {
        let details = bc_probe(&path_str(path));
        if details.is_undefined() || details.is_null() {
            return Err(MediaError::FileNotFound(path.to_path_buf()));
        }
        if let Some(error) = field(&details, "error").as_string() {
            return Err(MediaError::DecodeFailed(format!(
                "{}: {error}",
                path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            )));
        }
        let kind = match field(&details, "kind").as_string().as_deref() {
            Some("image") => MediaKind::Image,
            Some("audio") => MediaKind::Audio,
            _ => MediaKind::Video,
        };
        let duration = MediaTime::from_millis((number(&details, "duration") * 1000.0) as i64);
        let mut asset = MediaAsset::new(kind, path, duration);
        if let Some(name) = field(&details, "name").as_string() {
            asset.file_name = name;
        }
        asset.file_size = number(&details, "size") as u64;
        let (width, height) = (number(&details, "width") as u32, number(&details, "height") as u32);
        match kind {
            MediaKind::Video => {
                asset = asset.with_video(width, height, ASSUMED_RATE);
                // Already turned to RGB by the browser, in sRGB.
                asset.color = ColorMetadata::srgb();
            }
            MediaKind::Image => {
                asset.width = width;
                asset.height = height;
                asset.color = ColorMetadata::srgb();
            }
            MediaKind::Audio => {}
        }
        let channels = number(&details, "channels") as u16;
        if channels > 0 {
            asset = asset.with_audio(INTERNAL_SAMPLE_RATE as u32, channels);
            // Which codec the browser does not say; that there is one is
            // what the editor goes by.
            asset.audio_codec = Some("browser".to_owned());
        }
        if kind == MediaKind::Video {
            asset.video_codec = Some("browser".to_owned());
        }
        Ok(asset)
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

/// Frames from the browser's decoder for one file.
#[derive(Debug, Default)]
pub struct FfmpegDecoder {
    path: Option<String>,
    /// Where the next `decode_frame` reads: the last seek.
    at: MediaTime,
    width: u32,
    height: u32,
    video: bool,
    audio: bool,
}

impl FfmpegDecoder {
    pub fn new(_threads: u32) -> Result<Self, MediaError> {
        Ok(Self::default())
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn has_video(&self) -> bool {
        self.video
    }
    pub fn has_audio(&self) -> bool {
        self.audio
    }

    /// The frame at `at`, or the last one there was while it is fetched.
    fn frame(&self, at: MediaTime) -> Result<Option<VideoFrame>, MediaError> {
        let path = self.path.as_deref().ok_or(MediaError::NotOpen)?;
        let found = bc_frame(path, seconds_of(at));
        if found.is_undefined() || found.is_null() {
            return Err(MediaError::DecodeFailed("not decoded yet".to_owned()));
        }
        let width = number(&found, "width") as u32;
        let height = number(&found, "height") as u32;
        let data = js_sys::Uint8ClampedArray::from(field(&found, "data")).to_vec();
        // Not the frame asked for: stamped a tick off, so nothing keeps it
        // as that one.
        let exact = field(&found, "exact").as_bool().unwrap_or(false);
        let timestamp = if exact {
            at
        } else {
            MediaTime::from_ticks(at.ticks() - 1)
        };
        Ok(Some(VideoFrame {
            timestamp,
            width,
            height,
            color: ColorMetadata::srgb(),
            storage: FrameStorage::System {
                data,
                stride: width * 4,
            },
        }))
    }
}

impl MediaDecoder for FfmpegDecoder {
    fn open(&mut self, asset: &MediaAsset) -> Result<(), MediaError> {
        let path = path_str(&asset.path);
        let details = bc_probe(&path);
        if details.is_undefined() || details.is_null() {
            return Err(MediaError::FileNotFound(asset.path.clone()));
        }
        self.width = asset.width;
        self.height = asset.height;
        self.video = asset.kind.has_video();
        self.audio = asset.audio_channels.is_some_and(|c| c > 0);
        self.path = Some(path);
        self.at = MediaTime::ZERO;
        Ok(())
    }
    fn seek(&mut self, timestamp: MediaTime, _mode: SeekMode) -> Result<(), MediaError> {
        self.at = timestamp;
        Ok(())
    }
    fn decode_frame(
        &mut self,
        _cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError> {
        self.frame(self.at)
    }
    fn decode_frame_at(
        &mut self,
        target: MediaTime,
        _cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError> {
        self.at = target;
        self.frame(target)
    }
    /// Never a position: every request is answered by time, so the caller
    /// always seeks.
    fn position(&self) -> Option<MediaTime> {
        None
    }
    fn frame_duration(&self) -> MediaTime {
        MediaTime::from_frames(1, ASSUMED_RATE).unwrap_or(MediaTime::ZERO)
    }
    fn decode_audio(
        &mut self,
        _cancel: &dyn CancellationToken,
    ) -> Result<Option<AudioBuffer>, MediaError> {
        let path = self.path.as_deref().ok_or(MediaError::NotOpen)?;
        let from = self.at.ticks().max(0) / TICKS_PER_AUDIO_SAMPLE;
        let found = bc_sound(path, from as f64, SOUND_BLOCK);
        if found.is_null() || found.is_undefined() {
            return Ok(None);
        }
        let planes: Vec<Vec<f32>> = js_sys::Array::from(&found)
            .iter()
            .map(|plane| js_sys::Float32Array::from(plane).to_vec())
            .collect();
        let frames = planes.first().map_or(0, Vec::len);
        if frames == 0 {
            return Ok(None);
        }
        let timestamp = MediaTime::from_ticks(from * TICKS_PER_AUDIO_SAMPLE);
        self.at = MediaTime::from_ticks((from + frames as i64) * TICKS_PER_AUDIO_SAMPLE);
        Ok(Some(AudioBuffer {
            timestamp,
            planes,
            sample_rate: INTERNAL_SAMPLE_RATE as u32,
        }))
    }
    fn duration(&self) -> MediaTime {
        MediaTime::ZERO
    }
}

/// No proxies in a browser: the frames are preview-sized already.
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

/// No chapters read in a browser.
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

/// Choosing a file's streams: the browser does it, so nothing is chosen here.
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

    pub fn choose(_streams: &[StreamInfo]) -> (Option<i32>, Option<i32>) {
        (None, None)
    }
}
