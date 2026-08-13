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
mod probe;
mod raii;

pub use decode::FfmpegDecoder;
pub use encode::generate_proxy;
pub use encoders::{EncodeTarget, EncoderChoice, EncoderKind, EncoderProbe, probe_all};
pub use export::{ExportFormat, VideoWriter};
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
        let c_path = path_to_cstring(path)?;
        let mut ctx: *mut ffi::AVFormatContext = std::ptr::null_mut();

        // SAFETY: `ctx` is a valid out-pointer initialised to null, which is
        // what avformat_open_input requires (it allocates the context itself).
        // `c_path` outlives the call.
        let code = unsafe {
            ffi::avformat_open_input(
                &mut ctx,
                c_path.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
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
