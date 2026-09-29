//! The FFmpeg implementation of this crate's traits (§3, §84).
//!
//! # The boundary
//!
//! **This module is the only place in the entire project that names an FFmpeg
//! type** (§86). Everything outside it sees `MediaAsset`, `MediaProber`, and
//! `MediaDecoder` — the traits in [`crate::decoder`]. FFmpeg is one
//! implementation of the application's media layer, not its foundation.
//!
//! # Unsafe
//!
//! The workspace forbids `unsafe_code`; this crate opts out, and every `unsafe`
//! block in the project lives below this module. That is the trade the
//! abstraction buys: FFI danger is contained in one reviewable place instead of
//! spreading through the editor.
//!
//! Rules followed by every `unsafe` block here:
//!
//! * Every allocating FFmpeg call is paired with its free on **all** paths,
//!   including error paths — see [`InputContext`]'s `Drop`.
//! * Raw pointers are null-checked before dereference. FFmpeg returns null far
//!   more often than its docs suggest.
//! * No FFmpeg error is discarded (§74 forbids it); every non-zero return is
//!   converted into a [`MediaError`] carrying FFmpeg's own message.

mod decode;
mod encode;
mod encoders;
mod export;
mod filter;
mod heif;
mod probe;
mod raii;
pub mod streams;

pub use decode::FfmpegDecoder;
pub use encode::generate_proxy;
pub use encoders::{
    EncodeTarget, EncoderChoice, EncoderKind, EncoderProbe, PRORES, RateControl, VideoCodec,
    probe_all,
};
pub use export::{
    AUDIO_BITRATES_KBPS, ChapterMark, DEFAULT_AUDIO_BITRATE, ExportFormat, MAX_AUDIO_BITRATE,
    MIN_AUDIO_BITRATE, VideoWriter,
};

/// The chapters a file carries, in order, as a writer would have given them
/// (`VideoWriter::create_with_chapters`): what a player lists. Empty for a
/// file with none, an error for one that cannot be opened.
pub fn probe_chapters(path: &Path) -> Result<Vec<ChapterMark>, MediaError> {
    let input = InputContext::open(path)?;
    let mut chapters = Vec::new();
    // SAFETY: the context is open and stays so until `input` drops; every
    // chapter pointer FFmpeg lists is valid for that long.
    unsafe {
        let ctx = input.inner;
        for index in 0..(*ctx).nb_chapters as usize {
            let chapter = *(*ctx).chapters.add(index);
            if chapter.is_null() {
                continue;
            }
            let base = (*chapter).time_base;
            let to_ticks = |t: i64| {
                bettercut_foundation::MediaTime::from_ticks(
                    t * bettercut_foundation::TICKS_PER_SECOND * i64::from(base.num)
                        / i64::from(base.den.max(1)),
                )
            };
            let entry =
                ffi::av_dict_get((*chapter).metadata, c"title".as_ptr(), std::ptr::null(), 0);
            let title = if entry.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*entry).value)
                    .to_string_lossy()
                    .into_owned()
            };
            chapters.push(ChapterMark {
                title,
                start: to_ticks((*chapter).start),
                end: to_ticks((*chapter).end),
            });
        }
    }
    Ok(chapters)
}
pub use probe::FfmpegProber;

/// The one internal audio format (§20a.3).
///
/// 48 kHz because §9's 960,000-tick timebase divides by it exactly and does not
/// divide by 44,100. Everything is resampled to this at decode, so the mixer
/// and the timeline never disagree about where a sample sits.
pub const INTERNAL_SAMPLE_RATE: i32 = 48_000;

use std::ffi::{CStr, CString};
use std::path::Path;

use rusty_ffmpeg::ffi;

use crate::error::MediaError;

/// Translate an FFmpeg error code into its human-readable message.
///
/// §74: "Silently ignore FFmpeg failures" is prohibited. A bare `-22` in a log
/// is barely better than silence, so every error carries FFmpeg's own text.
/// FFmpeg's "no output yet, send more input" — `AVERROR(EAGAIN)` — which is
/// routine, not a failure.
///
/// Not `ffi::EAGAIN`: the committed binding was generated on Linux, where
/// EAGAIN is 11, and the number belongs to the C library of the platform
/// running. It is 11 on Windows and Linux but 35 on macOS and the BSDs, and
/// with the Linux number every encoder and decoder on a Mac stopped at its
/// first "try again" as if it had failed.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
pub(crate) const AVERROR_EAGAIN: i32 = -35;
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
pub(crate) const AVERROR_EAGAIN: i32 = -11;

pub(crate) fn error_string(code: i32) -> String {
    let mut buffer = [0_i8; ffi::AV_ERROR_MAX_STRING_SIZE as usize];

    // SAFETY: `buffer` is exactly AV_ERROR_MAX_STRING_SIZE bytes, which is what
    // av_strerror is documented to require. It always NUL-terminates.
    let ok = unsafe {
        ffi::av_strerror(
            code,
            buffer.as_mut_ptr().cast(),
            ffi::AV_ERROR_MAX_STRING_SIZE as usize,
        ) == 0
    };

    if !ok {
        return format!("unknown FFmpeg error {code}");
    }

    // SAFETY: av_strerror returned success, so `buffer` holds a NUL-terminated
    // string within its own bounds.
    unsafe { CStr::from_ptr(buffer.as_ptr().cast()) }
        .to_string_lossy()
        .into_owned()
}

/// Read a possibly-null C string as an owned `String`.
///
/// # Safety
///
/// `ptr` must be null or point to a NUL-terminated string valid for the
/// duration of the call.
pub(crate) unsafe fn opt_cstr(ptr: *const std::os::raw::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: delegated to the caller by this function's contract.
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Convert a path for FFmpeg, which wants UTF-8 even on Windows.
pub(crate) fn path_to_cstring(path: &Path) -> Result<CString, MediaError> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| MediaError::UnsupportedFormat(path.to_path_buf()))
}

/// An open input file.
///
/// Exists so that `avformat_close_input` runs on every path out of a probe,
/// including early returns and panics. Doing it by hand is how demuxer handles
/// get leaked, and a leaked handle keeps the user's file locked on Windows.
pub(crate) struct InputContext {
    inner: *mut ffi::AVFormatContext,
}

impl InputContext {
    pub(crate) fn open(path: &Path) -> Result<Self, MediaError> {
        if !path.exists() {
            return Err(MediaError::FileNotFound(path.to_path_buf()));
        }
        Self::open_with(path, &[])
    }

    /// Open `asset`'s file — or, for an image sequence, its `image2` pattern
    /// with the start number and rate the demuxer needs to count the frames
    /// as the asset does.
    pub(crate) fn open_asset(asset: &crate::MediaAsset) -> Result<Self, MediaError> {
        match (asset.sequence, asset.sequence_pattern()) {
            (Some(sequence), Some(pattern)) => {
                if !asset.path.exists() {
                    return Err(MediaError::FileNotFound(asset.path.clone()));
                }
                let rate = asset
                    .frame_rate
                    .unwrap_or(bettercut_foundation::FrameRate::PAL_25)
                    .as_rational();
                Self::open_with(
                    &pattern,
                    &[
                        ("start_number", sequence.start.to_string()),
                        ("framerate", format!("{}/{}", rate.num(), rate.den())),
                    ],
                )
            }
            _ => Self::open(&asset.path),
        }
    }

    /// Open `path` with demuxer `options`, which is how a pattern path is
    /// told where its numbers start. No existence check: a pattern is not a
    /// file, and FFmpeg's own error covers a run that is not there.
    fn open_with(path: &Path, options: &[(&str, String)]) -> Result<Self, MediaError> {
        let c_path = path_to_cstring(path)?;
        let mut ctx: *mut ffi::AVFormatContext = std::ptr::null_mut();

        let mut dict: *mut ffi::AVDictionary = std::ptr::null_mut();
        for (key, value) in options {
            let key = CString::new(*key)
                .map_err(|_| MediaError::UnsupportedFormat(path.to_path_buf()))?;
            let value = CString::new(value.as_str())
                .map_err(|_| MediaError::UnsupportedFormat(path.to_path_buf()))?;
            // SAFETY: `dict` is a valid in/out pointer (null allocates); the
            // strings outlive the call, and av_dict_set copies them.
            unsafe { ffi::av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), 0) };
        }

        // SAFETY: `ctx` is a valid out-pointer initialised to null, which is
        // what avformat_open_input requires (it allocates the context itself).
        // `c_path` outlives the call; `dict` is ours to free afterwards
        // (FFmpeg leaves the options it did not take in it).
        let code = unsafe {
            let code = ffi::avformat_open_input(
                &mut ctx,
                c_path.as_ptr(),
                std::ptr::null_mut(),
                &mut dict,
            );
            ffi::av_dict_free(&mut dict);
            code
        };

        if code < 0 {
            // On failure avformat_open_input has already freed and nulled ctx,
            // so there is nothing to clean up here.
            // §74: report FFmpeg's own message rather than swallowing it.
            tracing::debug!(
                path = %path.display(),
                reason = %error_string(code),
                "avformat_open_input failed"
            );
            return Err(MediaError::UnsupportedFormat(path.to_path_buf()));
        }
        if ctx.is_null() {
            return Err(MediaError::UnsupportedFormat(path.to_path_buf()));
        }

        let this = Self { inner: ctx };

        // Container headers frequently omit resolution, frame rate, and colour.
        // This decodes a little of each stream to fill the gaps — without it,
        // §12's metadata list is half-empty for common files.
        // SAFETY: `this.inner` is non-null and owned by `this`.
        let code = unsafe { ffi::avformat_find_stream_info(this.inner, std::ptr::null_mut()) };
        if code < 0 {
            tracing::debug!(
                path = %path.display(),
                reason = %error_string(code),
                "avformat_find_stream_info failed"
            );
            return Err(MediaError::NoStream(path.to_path_buf()));
        }

        Ok(this)
    }

    pub(crate) fn as_ptr(&self) -> *const ffi::AVFormatContext {
        self.inner
    }

    /// FFmpeg's read and seek calls take a mutable context even though they do
    /// not change the container's structure.
    pub(crate) fn as_mut_ptr(&self) -> *mut ffi::AVFormatContext {
        self.inner
    }

    /// The demuxer's short name, e.g. `"mov,mp4,m4a,3gp,3g2,mj2"`, `"png_pipe"`.
    ///
    /// This is how still images are told apart from one-frame videos: FFmpeg
    /// hands a PNG to an `image2`/`*_pipe` demuxer, which reports a synthetic
    /// 25 fps frame rate. Believing that rate would make a still image look
    /// like a 40 ms video clip.
    pub(crate) fn format_name(&self) -> Option<String> {
        // SAFETY: `inner` is non-null; `iformat` is set by avformat_open_input
        // and its `name` is a static string owned by FFmpeg.
        unsafe {
            let iformat = (*self.inner).iformat;
            if iformat.is_null() {
                return None;
            }
            opt_cstr((*iformat).name)
        }
    }

    /// The container's streams.
    pub(crate) fn streams(&self) -> Vec<*mut ffi::AVStream> {
        // SAFETY: `inner` is non-null for the lifetime of `self`, and FFmpeg
        // guarantees `streams` points to `nb_streams` valid stream pointers.
        unsafe {
            let ctx = &*self.inner;
            (0..ctx.nb_streams as usize)
                .map(|i| *ctx.streams.add(i))
                .filter(|s| !s.is_null())
                .collect()
        }
    }
}

impl Drop for InputContext {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            // SAFETY: `inner` was produced by avformat_open_input and has not
            // been closed; avformat_close_input nulls the pointer it is given.
            unsafe { ffi::avformat_close_input(&mut self.inner) };
        }
    }
}

// FFmpeg contexts are not thread-safe to share, and this one owns a file
// handle. Sending it to another thread is fine; sharing it is not.
unsafe impl Send for InputContext {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_strings_are_human_readable() {
        // AVERROR(EINVAL) — the most common error we will show a user.
        let message = error_string(-22);
        assert!(!message.is_empty());
        assert!(
            !message.contains("unknown FFmpeg error"),
            "av_strerror did not resolve: {message}"
        );
    }

    #[test]
    fn opening_a_missing_file_reports_not_found() {
        let result = InputContext::open(Path::new("Z:/definitely/not/here.mp4"));
        assert!(matches!(result, Err(MediaError::FileNotFound(_))));
    }

    #[test]
    fn opening_a_non_media_file_is_an_error_not_a_crash() {
        let dir = std::env::temp_dir().join("bettercut-media-tests");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("not-media.txt");
        let _ = std::fs::write(&path, b"this is not a media file");

        let result = InputContext::open(&path);
        assert!(result.is_err(), "a text file was accepted as media");

        let _ = std::fs::remove_file(&path);
    }
}

/// Turn an RGBA picture `width` by `height` clockwise by `degrees` (0, 90, 180
/// or 270; anything else is treated as 0). Returns the pixels and the turned
/// size.
pub(crate) fn rotate_rgba(
    pixels: &[u8],
    width: u32,
    height: u32,
    degrees: u16,
) -> (Vec<u8>, u32, u32) {
    let (w, h) = (width as usize, height as usize);
    if pixels.len() < w * h * 4 {
        return (pixels.to_vec(), width, height);
    }
    let pixel = |x: usize, y: usize| {
        let at = (y * w + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    let (out_w, out_h) = match degrees {
        90 | 270 => (h, w),
        180 => (w, h),
        _ => return (pixels[..w * h * 4].to_vec(), width, height),
    };
    let mut out = Vec::with_capacity(out_w * out_h * 4);
    for y in 0..out_h {
        for x in 0..out_w {
            // Where the shown pixel (x, y) was in the stored frame.
            let from = match degrees {
                90 => pixel(y, h - 1 - x),
                180 => pixel(w - 1 - x, h - 1 - y),
                _ => pixel(w - 1 - y, x),
            };
            out.extend_from_slice(&from);
        }
    }
    (out, out_w as u32, out_h as u32)
}

/// The clockwise turn a display matrix asks for, snapped to a quarter.
///
/// # Safety
/// `matrix` must point at nine 32-bit values.
unsafe fn matrix_rotation(matrix: *const i32) -> u16 {
    // SAFETY: the caller's promise.
    let angle = unsafe { ffi::av_display_rotation_get(matrix) };
    if !angle.is_finite() {
        return 0;
    }
    // FFmpeg's angle is anticlockwise; the turn to show it is the opposite.
    let quarters = (-angle / 90.0).round() as i64;
    (quarters.rem_euclid(4) * 90) as u16
}

/// The clockwise turn a decoded frame's own display matrix asks for — how
/// FFmpeg reports a photo's EXIF orientation.
pub(crate) fn frame_rotation(frame: &raii::Frame) -> u16 {
    // SAFETY: the frame is valid; FFmpeg returns null without a matrix, and a
    // display matrix is always nine 32-bit values.
    unsafe {
        let side = ffi::av_frame_get_side_data(frame.as_ptr(), ffi::AV_FRAME_DATA_DISPLAYMATRIX);
        if side.is_null() || (*side).data.is_null() || (*side).size < 36 {
            return 0;
        }
        matrix_rotation((*side).data as *const i32)
    }
}

/// The clockwise turn a stream's display matrix asks for, snapped to a
/// quarter: 0, 90, 180 or 270.
pub(crate) fn display_rotation(par: &ffi::AVCodecParameters) -> u16 {
    // SAFETY: the side-data array and its count come from the same
    // parameters; FFmpeg returns null when there is no matrix, and a display
    // matrix is always nine 32-bit values.
    unsafe {
        let side = ffi::av_packet_side_data_get(
            par.coded_side_data,
            par.nb_coded_side_data,
            ffi::AV_PKT_DATA_DISPLAYMATRIX,
        );
        if side.is_null() || (*side).data.is_null() || (*side).size < 36 {
            return 0;
        }
        matrix_rotation((*side).data as *const i32)
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::rotate_rgba;

    /// A 2x1 picture: red then green.
    fn two() -> Vec<u8> {
        vec![255, 0, 0, 255, 0, 255, 0, 255]
    }

    #[test]
    fn a_quarter_turn_clockwise_puts_the_left_on_top() {
        let (out, w, h) = rotate_rgba(&two(), 2, 1, 90);
        assert_eq!((w, h), (1, 2));
        assert_eq!(&out[..4], &[255, 0, 0, 255], "the left pixel is not on top");
    }

    #[test]
    fn three_quarters_puts_the_right_on_top() {
        let (out, w, h) = rotate_rgba(&two(), 2, 1, 270);
        assert_eq!((w, h), (1, 2));
        assert_eq!(&out[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn a_half_turn_swaps_the_ends_and_none_changes_nothing() {
        let (out, _, _) = rotate_rgba(&two(), 2, 1, 180);
        assert_eq!(&out[..4], &[0, 255, 0, 255]);
        assert_eq!(rotate_rgba(&two(), 2, 1, 0).0, two());
    }
}

#[cfg(test)]
mod eagain_tests {
    /// FFmpeg itself reads the code as "try again" on the platform running,
    /// which is what the loops that wait on it depend on.
    #[test]
    fn eagain_is_this_platforms_try_again() {
        let text = super::error_string(super::AVERROR_EAGAIN).to_lowercase();
        assert!(
            text.contains("temporarily unavailable") || text.contains("try again"),
            "{text}"
        );
    }
}
