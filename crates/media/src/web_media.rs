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
//! Export goes through the browser's encoders (`js/browser_export.js`,
//! [`browser::export`]) into an MP4 written by [`crate::mp4_mux`], and its
//! frames are fetched exactly — a decoder told to
//! [`FfmpegDecoder::wait_for_exact_frames`] answers "not yet"
//! ([`browser::is_not_yet`]) rather than with a stand-in.
//!
//! Not here yet: proxies, chapters.

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
    fn bc_pick_files(accept: &str, multiple: bool);
    fn bc_take_opened() -> js_sys::Array;
    fn bc_opened_text(path: &str) -> Option<String>;
    fn bc_file_size(path: &str) -> f64;
    fn bc_probe(path: &str) -> JsValue;
    fn bc_frame(path: &str, seconds: f64) -> JsValue;
    fn bc_frame_exact(path: &str, seconds: f64) -> JsValue;
    fn bc_setting(path: &str) -> Option<String>;
    fn bc_keep_setting(path: &str, text: &str) -> bool;
    fn bc_poster(path: &str) -> JsValue;
    fn bc_sound(path: &str, from: f64, frames: u32) -> JsValue;
    fn bc_filmstrip(path: &str) -> JsValue;
    fn bc_peaks(path: &str) -> JsValue;
    fn bc_stored_project() -> Option<String>;
    fn bc_keep_project(json: &str, paths: Vec<String>);
}

#[wasm_bindgen(module = "/js/browser_export.js")]
extern "C" {
    fn bc_export_start(width: u32, height: u32, rate_num: u32, rate_den: u32, bitrate: u32, channels: u32);
    fn bc_export_state() -> String;
    fn bc_export_has_room() -> bool;
    fn bc_export_sound_codec() -> Option<String>;
    fn bc_export_frame(rgba: &[u8], width: u32, height: u32, index: u32, key: bool);
    fn bc_export_sound(planes: js_sys::Array);
    fn bc_export_finish();
    fn bc_export_take() -> JsValue;
    fn bc_export_cancel();
    fn bc_download(bytes: &[u8], name: &str, kind: &str);
    fn bc_while_hidden(callback: &Closure<dyn FnMut()>);
}

/// What a decoder told to wait for exact frames says while one is on its way.
const NOT_YET: &str = "not decoded yet";

/// The page's side of files: what the app calls each frame.
pub mod browser {
    use super::*;

    /// Whether `err` only means the frame asked for is still on its way: ask
    /// again on a later frame.
    pub fn is_not_yet(err: &MediaError) -> bool {
        matches!(err, MediaError::DecodeFailed(message) if message == NOT_YET)
    }

    /// One of the editor's small files kept in this browser, by its path.
    pub fn setting(path: &str) -> Option<String> {
        bc_setting(path)
    }

    /// Keep one of the editor's small files in this browser; false when the
    /// browser would not.
    pub fn keep_setting(path: &str, text: &str) -> bool {
        bc_keep_setting(path, text)
    }

    /// Call `tick` often while the page's tab is hidden, when the app's own
    /// frames stop: for work that must go on meanwhile, an export. Once a
    /// page; later calls do nothing.
    pub fn while_hidden(tick: impl FnMut() + 'static) {
        let tick = Closure::<dyn FnMut()>::new(tick);
        bc_while_hidden(&tick);
        // Called for as long as the page is open.
        tick.forget();
    }

    /// Hand the person a file, as a download called `name`.
    pub fn download(bytes: &[u8], name: &str, kind: &str) {
        bc_download(bytes, name, kind);
    }

    /// The browser's encoders, for an export. One at a time; nothing here
    /// waits — the app calls again on its next frame.
    pub mod export {
        use super::super::*;

        /// Open the encoders: H.264 at `width` × `height`, and sound when
        /// `channels` is more than none and the browser can make AAC or Opus.
        pub fn start(width: u32, height: u32, rate: FrameRate, bitrate: u32, channels: u32) {
            bc_export_start(
                width,
                height,
                rate.as_rational().num() as u32,
                rate.as_rational().den() as u32,
                bitrate,
                channels,
            );
        }

        /// Where the encoders are.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum State {
            Starting,
            Ready,
            Finishing,
            Done,
            Failed(String),
            /// None started, or cancelled.
            Idle,
        }

        pub fn state() -> State {
            let state = bc_export_state();
            match state.as_str() {
                "starting" => State::Starting,
                "ready" => State::Ready,
                "finishing" => State::Finishing,
                "done" => State::Done,
                "none" | "cancelled" => State::Idle,
                other => State::Failed(other.strip_prefix("failed: ").unwrap_or(other).to_owned()),
            }
        }

        /// Whether the video encoder will take another frame now.
        pub fn has_room() -> bool {
            bc_export_has_room()
        }

        /// Whether the export carries sound, once started.
        pub fn sound_codec() -> Option<String> {
            bc_export_sound_codec()
        }

        /// Frame `index`, tightly packed RGBA.
        pub fn frame(rgba: &[u8], width: u32, height: u32, index: u32, key: bool) {
            bc_export_frame(rgba, width, height, index, key);
        }

        /// The next stretch of sound, one plane a channel.
        pub fn sound(planes: &[Vec<f32>]) {
            let array = js_sys::Array::new();
            for plane in planes {
                array.push(&js_sys::Float32Array::from(plane.as_slice()));
            }
            bc_export_sound(array);
        }

        /// No more frames: let the encoders finish.
        pub fn finish() {
            bc_export_finish();
        }

        pub fn cancel() {
            bc_export_cancel();
        }

        /// What the encoders made.
        pub struct Encoded {
            /// The H.264 decoder configuration (`avcC`).
            pub video_description: Vec<u8>,
            /// Each frame, in order, and whether it is a keyframe.
            pub video: Vec<(Vec<u8>, bool)>,
            pub sound: Option<EncodedSound>,
        }

        pub struct EncodedSound {
            /// `"aac"` or `"opus"`.
            pub codec: String,
            pub description: Option<Vec<u8>>,
            pub channels: u16,
            /// Each packet, and how long it lasts in microseconds.
            pub chunks: Vec<(Vec<u8>, f64)>,
        }

        fn bytes(value: &JsValue) -> Option<Vec<u8>> {
            (!value.is_null() && !value.is_undefined())
                .then(|| js_sys::Uint8Array::new(value).to_vec())
        }

        /// Everything the encoders made, once [`State::Done`]; handed over once.
        pub fn take() -> Option<Encoded> {
            let taken = bc_export_take();
            if taken.is_null() || taken.is_undefined() {
                return None;
            }
            let video = field(&taken, "video");
            let chunks = js_sys::Array::from(&field(&video, "chunks"))
                .iter()
                .filter_map(|chunk| {
                    Some((
                        bytes(&field(&chunk, "data"))?,
                        field(&chunk, "key").as_bool().unwrap_or(false),
                    ))
                })
                .collect();
            let sound = field(&taken, "sound");
            let sound = (!sound.is_null() && !sound.is_undefined()).then(|| EncodedSound {
                codec: field(&sound, "codec").as_string().unwrap_or_default(),
                description: bytes(&field(&sound, "description")),
                channels: number(&sound, "channels") as u16,
                chunks: js_sys::Array::from(&field(&sound, "chunks"))
                    .iter()
                    .filter_map(|chunk| {
                        Some((bytes(&field(&chunk, "data"))?, number(&chunk, "duration")))
                    })
                    .collect(),
            });
            Some(Encoded {
                video_description: bytes(&field(&video, "description")).unwrap_or_default(),
                video: chunks,
                sound,
            })
        }
    }

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

    /// The project kept in this browser last time, as `.vproj` JSON.
    pub fn stored_project() -> Option<String> {
        bc_stored_project()
    }

    /// Keep this project in the browser, and the page's files at `paths` —
    /// the ones it uses — letting go of any other file kept before.
    pub fn keep_project(json: &str, paths: Vec<String>) {
        bc_keep_project(json, paths);
    }

    /// Open the browser's file picker. What is chosen arrives later, through
    /// [`take_ready`] — or [`take_opened`], for a project file.
    pub fn pick_files(extensions: &[&str]) {
        pick(extensions, true);
    }

    /// The picker, for one file.
    pub fn pick_file(extensions: &[&str]) {
        pick(extensions, false);
    }

    fn pick(extensions: &[&str], multiple: bool) {
        let accept: Vec<String> = extensions.iter().map(|e| format!(".{e}")).collect();
        bc_pick_files(&accept.join(","), multiple);
    }

    /// Project files dropped on the page or picked since the last look, by
    /// the names [`opened_text`] knows them by.
    pub fn take_opened() -> Vec<std::path::PathBuf> {
        bc_take_opened()
            .iter()
            .filter_map(|v| v.as_string())
            .map(std::path::PathBuf::from)
            .collect()
    }

    /// The text of a project file from [`take_opened`]. Given once.
    pub fn opened_text(path: &std::path::Path) -> Option<String> {
        bc_opened_text(&path_str(path))
    }

    /// The size of a file the page has, or `None` when it has no such file.
    pub fn file_size(path: &std::path::Path) -> Option<u64> {
        let size = bc_file_size(&path_str(path));
        (size >= 0.0).then_some(size as u64)
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

/// The rate assumed for browser video the page could not read one from
/// (anything but an MP4 or MOV): a `<video>` element does not say.
const ASSUMED_RATE: FrameRate = FrameRate::FPS_30;

/// The rate the page read from the file's own tables — frames over their
/// length, so a phone's slightly uneven 29.97 comes out as 29.98 — taken
/// as the standard rate it is within 0.2% of, else as it is.
fn rate_from(num: f64, den: f64) -> Option<FrameRate> {
    if !(num.is_finite() && den.is_finite() && num >= 1.0 && den >= 1.0) {
        return None;
    }
    let fps = num / den;
    if !(1.0..=1000.0).contains(&fps) {
        return None;
    }
    let off = |rate: &FrameRate| (rate.as_f64() - fps).abs() / rate.as_f64();
    // The nearest: 24 is within 0.2% of 23.976 too.
    FrameRate::SUPPORTED
        .into_iter()
        .filter(|rate| off(rate) <= 0.002)
        .min_by(|a, b| off(a).total_cmp(&off(b)))
        .or_else(|| FrameRate::new((fps * 1000.0).round() as i64, 1000))
}

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
                let rate = rate_from(number(&details, "rateNum"), number(&details, "rateDen"))
                    .unwrap_or(ASSUMED_RATE);
                asset = asset.with_video(width, height, rate);
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
    /// An export's decoder: only the frame asked for, or "not yet".
    exact_only: bool,
    /// The file's rate, as probed.
    rate: Option<FrameRate>,
}

impl FfmpegDecoder {
    pub fn new(_threads: u32) -> Result<Self, MediaError> {
        Ok(Self::default())
    }

    /// Answer only with the frame asked for, at full size — an export's
    /// decoder — and while it is on its way, with an error
    /// [`browser::is_not_yet`] recognises, never a stand-in.
    pub fn wait_for_exact_frames(&mut self) {
        self.exact_only = true;
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
        let found = if self.exact_only {
            bc_frame_exact(path, seconds_of(at))
        } else {
            bc_frame(path, seconds_of(at))
        };
        if found.is_undefined() || found.is_null() {
            return Err(MediaError::DecodeFailed(NOT_YET.to_owned()));
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
        self.rate = asset.frame_rate;
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
        MediaTime::from_frames(1, self.rate.unwrap_or(ASSUMED_RATE)).unwrap_or(MediaTime::ZERO)
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
