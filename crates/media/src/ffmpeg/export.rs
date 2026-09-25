//! Writing the finished video (Milestone 6, §0.1).
//!
//! Takes composited RGBA frames and mixed audio and produces an MP4. It is the
//! back half of export; the front half — rendering the timeline at full quality
//! from the original media (§14, §46) — belongs to the export crate, which
//! knows about projects.
//!
//! ## The encoder is chosen by opening it
//!
//! [`super::encoders`] tries the OS encoders in §0.1's stated order and falls
//! back to openh264. That choice is made here, once, before the first frame,
//! and reported so the interface can say which one ran — "why was that fast"
//! and "why was that slow" have the same answer and the user should be able to
//! see it.
//!
//! ## What arrives here
//!
//! Straight RGBA in the compositor's working space (§21a.1): sRGB-encoded,
//! 8-bit, opaque. It is converted to YUV 4:2:0 limited-range BT.709 on the way
//! into the encoder, which is what the tags on the output stream promise —
//! §21a's whole argument is that an untagged or mistagged file reintroduces the
//! colour bug at the player.

use std::path::Path;

use bettercut_foundation::MediaTime;

use rusty_ffmpeg::ffi;

use crate::error::MediaError;

use bettercut_foundation::FrameRate;

use super::INTERNAL_SAMPLE_RATE;
use super::encode::{Muxer, drain_encoder};
use super::encoders::{
    EncodeTarget, EncoderChoice, RateControl, VideoCodec, av_rational, open_best, open_prores,
    open_vp9_alpha,
};
use super::raii::{CodecContext, Frame, Scaler};

/// How the output is to be written.
/// A chapter to write into the file: what players list, and what a viewer
/// jumps between. Times are in the file's own timeline, from its first
/// frame; a mark past the end of the file is dropped by the muxer, not by us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMark {
    pub title: String,
    pub start: MediaTime,
    pub end: MediaTime,
}

/// The sound's bitrate when none is asked for: 192 kb/s is the deliverable,
/// not a proxy — the difference from 128 is audible on music and costs half
/// a megabyte a minute.
pub const DEFAULT_AUDIO_BITRATE: i64 = 192_000;
/// What AAC is worth encoding at: below 64 kb/s it is a phone line, above
/// 320 it is bits the format cannot use.
pub const MIN_AUDIO_BITRATE: i64 = 64_000;
pub const MAX_AUDIO_BITRATE: i64 = 320_000;
/// The choices the export dialog offers, in kb/s.
pub const AUDIO_BITRATES_KBPS: [i64; 5] = [96, 128, 192, 256, 320];

#[derive(Debug, Clone, Copy)]
pub struct ExportFormat {
    pub width: u32,
    pub height: u32,
    /// The exact ratio §9 stores — 30000/1001, not 29.97. Rounding it puts a
    /// long export's audio adrift by the end.
    pub frame_rate: FrameRate,
    pub codec: VideoCodec,
    /// Video bits per second, or `None` to derive one from the format.
    pub bitrate: Option<i64>,
    pub rate_control: RateControl,
    /// Channels of audio, or zero for a silent file.
    pub channels: usize,
    /// The sound's bits per second, or `None` for [`DEFAULT_AUDIO_BITRATE`].
    pub audio_bitrate: Option<i64>,
    /// §15.1: FFmpeg never gets every core, not even for the last job running.
    pub threads: u32,
    /// A see-through background: VP9 with an alpha plane (write to a `.webm`
    /// path), no sound — WebM carries no AAC. The frames pushed must then be
    /// straight, not premultiplied, alpha.
    pub transparent: bool,
    /// A ProRes 422 HQ master (write to a `.mov` path): 10-bit 4:2:2, the
    /// codec's own quality, sound as AAC. Ignored with `transparent`.
    pub prores: bool,
}

/// An open output file, accepting frames until [`Self::finish`].
pub struct VideoWriter {
    muxer: Muxer,
    video: CodecContext,
    audio: Option<AudioTrack>,
    scaler: Scaler,
    /// The RGBA input, wrapped as an `AVFrame` so sws can read it.
    source: Frame,
    /// The YUV picture handed to the encoder.
    picture: Frame,
    /// Frames written so far, which is also the next PTS: the encoder timebase
    /// is the reciprocal of the frame rate, so PTS counts frames (§9 — integer
    /// throughout, no accumulated float drift over a long export).
    frames: i64,
    frame_rate: ffi::AVRational,
    encoder: EncoderChoice,
    width: u32,
    height: u32,
}

struct AudioTrack {
    encoder: CodecContext,
    frame: Frame,
    /// Samples the encoder wants per call.
    frame_size: usize,
    /// Samples not yet handed over, one `Vec` per channel.
    pending: Vec<Vec<f32>>,
    channels: usize,
    /// Samples written, which is the next PTS at 48 kHz.
    samples: i64,
}

impl VideoWriter {
    /// Open `path` and pick an encoder.
    ///
    /// Fails before anything is written if no encoder will open, so a doomed
    /// export is refused up front rather than after the user has waited for it.
    pub fn create(path: &Path, format: ExportFormat) -> Result<Self, MediaError> {
        Self::create_with_chapters(path, format, &[])
    }

    /// The same, with `chapters` written into the file's header so players
    /// list them. Only the containers that carry chapters keep them (MP4 and
    /// MOV do; a bare stream cannot), which is the muxer's decision, not ours.
    pub fn create_with_chapters(
        path: &Path,
        format: ExportFormat,
        chapters: &[ChapterMark],
    ) -> Result<Self, MediaError> {
        if format.width == 0 || format.height == 0 {
            return Err(MediaError::DecodeFailed(
                "export resolution must not be zero".to_owned(),
            ));
        }
        // H.264 4:2:0 stores chroma at half resolution in both directions, so
        // an odd dimension has no representation. Refusing beats silently
        // exporting something a pixel smaller than the sequence.
        if !format.width.is_multiple_of(2) || !format.height.is_multiple_of(2) {
            return Err(MediaError::DecodeFailed(format!(
                "export resolution {}x{} must be even for H.264 4:2:0",
                format.width, format.height
            )));
        }

        let mut muxer = Muxer::allocate(path)?;
        // MP4 keeps SPS/PPS in the container, and that has to be known before
        // any encoder opens: setting the flag afterwards silently does nothing
        // and the file does not decode.
        let global_header = muxer.needs_global_header();

        let target = EncodeTarget {
            width: format.width,
            height: format.height,
            frame_rate: format.frame_rate,
            codec: format.codec,
            bitrate: format.bitrate,
            rate_control: format.rate_control,
            threads: format.threads,
            global_header,
        };
        let (encoder, video) = if format.transparent {
            open_vp9_alpha(target)?
        } else if format.prores {
            open_prores(target)?
        } else {
            open_best(target)?
        };
        let picture_format = if format.transparent {
            ffi::AV_PIX_FMT_YUVA420P
        } else if format.prores {
            ffi::AV_PIX_FMT_YUV422P10LE
        } else {
            ffi::AV_PIX_FMT_YUV420P
        };

        let audio = if format.channels > 0 && !format.transparent {
            Some(AudioTrack::open(
                format.channels,
                format.audio_bitrate.unwrap_or(DEFAULT_AUDIO_BITRATE),
                format.threads,
                global_header,
            )?)
        } else {
            None
        };

        muxer.add_video(video.as_ptr())?;
        if format.transparent {
            muxer.mark_video_alpha()?;
        }
        if let Some(track) = &audio {
            muxer.add_audio(track.encoder.as_ptr())?;
        }
        // Before the header is written: chapters live in it.
        muxer.set_chapters(chapters)?;
        muxer.begin(path)?;

        let width = format.width as i32;
        let height = format.height as i32;
        let scaler = Scaler::to_yuv(
            width,
            height,
            ffi::AV_PIX_FMT_RGBA,
            // The compositor's output is full-range sRGB; the encoder wants
            // limited-range BT.709, which is what the stream is tagged as.
            true,
            ffi::SWS_CS_ITU709 as i32,
            width,
            height,
            picture_format,
        )?;

        Ok(Self {
            muxer,
            video,
            audio,
            scaler,
            source: Frame::video(width, height, ffi::AV_PIX_FMT_RGBA)?,
            picture: Frame::video(width, height, picture_format)?,
            frames: 0,
            frame_rate: av_rational(format.frame_rate),
            encoder,
            width: format.width,
            height: format.height,
        })
    }

    /// Which encoder is running, for the interface to report.
    pub fn encoder(&self) -> EncoderChoice {
        self.encoder
    }

    pub fn frames_written(&self) -> i64 {
        self.frames
    }

    /// Append one composited frame. `rgba` is tightly packed, top row first.
    pub fn push_frame(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        let expected = self.width as usize * self.height as usize * 4;
        if rgba.len() < expected {
            return Err(MediaError::DecodeFailed(format!(
                "frame is {} bytes, expected {expected}",
                rgba.len()
            )));
        }

        // SAFETY: `source` was allocated for exactly these dimensions in RGBA,
        // so plane 0 is the only plane and its stride is at least 4*width. The
        // copy is bounded by the frame's own linesize on each row.
        unsafe {
            let raw = &*self.source.as_ptr();
            let stride = raw.linesize[0] as usize;
            let row_bytes = self.width as usize * 4;
            for y in 0..self.height as usize {
                std::ptr::copy_nonoverlapping(
                    rgba.as_ptr().add(y * row_bytes),
                    raw.data[0].add(y * stride),
                    row_bytes,
                );
            }
        }

        self.scaler
            .convert_to_frame(&self.source, &mut self.picture)?;

        // SAFETY: `picture` is allocated; PTS is the only field being set.
        unsafe {
            (*self.picture.as_ptr()).pts = self.frames;
        }
        self.frames += 1;

        let stream = self.muxer.video_stream();
        drain_encoder(
            self.video.as_ptr(),
            Some(self.picture.as_ptr()),
            &mut self.muxer,
            stream,
            encoder_timebase(self.frame_rate),
        )
    }

    /// Append mixed audio: planar, 48 kHz, one `Vec` per channel (§20a.3).
    ///
    /// Buffered internally, because the encoder takes a fixed number of samples
    /// per call — 1024 for AAC — and the mixer produces whatever a block
    /// happens to be. Anything left over is flushed by [`Self::finish`].
    pub fn push_audio(&mut self, planes: &[Vec<f32>]) -> Result<(), MediaError> {
        let Some(track) = &mut self.audio else {
            return Ok(()); // silent export; the caller need not know
        };
        track.push(planes);

        let stream = self.muxer.audio_stream();
        while let Some(frame) = track.take_frame()? {
            drain_encoder(
                track.encoder.as_ptr(),
                Some(frame),
                &mut self.muxer,
                stream,
                audio_timebase(),
            )?;
        }
        Ok(())
    }

    /// Flush both encoders and close the file.
    ///
    /// **Not optional.** Encoders hold frames back for lookahead; skipping the
    /// flush loses the tail of the video, and skipping the trailer leaves an
    /// MP4 with no index that most players refuse.
    pub fn finish(mut self) -> Result<(), MediaError> {
        let video_stream = self.muxer.video_stream();
        let audio_stream = self.muxer.audio_stream();

        if let Some(track) = &mut self.audio {
            // Pad the last partial frame with silence rather than dropping it:
            // the alternative truncates up to 21 ms off the end of the audio.
            if let Some(frame) = track.take_remainder()? {
                drain_encoder(
                    track.encoder.as_ptr(),
                    Some(frame),
                    &mut self.muxer,
                    audio_stream,
                    audio_timebase(),
                )?;
            }
        }

        drain_encoder(
            self.video.as_ptr(),
            None,
            &mut self.muxer,
            video_stream,
            encoder_timebase(self.frame_rate),
        )?;

        if let Some(track) = &self.audio {
            drain_encoder(
                track.encoder.as_ptr(),
                None,
                &mut self.muxer,
                audio_stream,
                audio_timebase(),
            )?;
        }

        self.muxer.finish()
    }
}

impl AudioTrack {
    fn open(
        channels: usize,
        bitrate: i64,
        threads: u32,
        global_header: bool,
    ) -> Result<Self, MediaError> {
        let encoder = CodecContext::encoder("aac", threads)?;

        // SAFETY: allocated and not yet opened.
        unsafe {
            let ctx = &mut *encoder.as_ptr();
            ctx.sample_fmt = ffi::AV_SAMPLE_FMT_FLTP;
            ctx.sample_rate = INTERNAL_SAMPLE_RATE;
            ctx.time_base = audio_timebase();
            // What was asked for, held to what AAC is worth encoding at.
            ctx.bit_rate = bitrate.clamp(MIN_AUDIO_BITRATE, MAX_AUDIO_BITRATE);
            ffi::av_channel_layout_default(&mut ctx.ch_layout, channels as i32);
            if global_header {
                ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        encoder.open_encoder()?;

        // SAFETY: the encoder is open, so `frame_size` is populated.
        let frame_size = unsafe { (*encoder.as_ptr()).frame_size.max(1024) as usize };
        let frame = Frame::audio(frame_size, channels, INTERNAL_SAMPLE_RATE)?;

        Ok(Self {
            encoder,
            frame,
            frame_size,
            pending: vec![Vec::new(); channels],
            channels,
            samples: 0,
        })
    }

    fn push(&mut self, planes: &[Vec<f32>]) {
        for channel in 0..self.channels {
            // A mix that ran out of one channel is silence on it, not a shorter
            // file: the planes have to stay the same length or the channels
            // drift apart.
            let source = planes.get(channel);
            let wanted = planes.first().map_or(0, Vec::len);
            let plane = &mut self.pending[channel];
            match source {
                Some(samples) => plane.extend_from_slice(samples),
                None => plane.extend(std::iter::repeat_n(0.0, wanted)),
            }
        }
    }

    fn buffered(&self) -> usize {
        self.pending.first().map_or(0, Vec::len)
    }

    /// One full encoder frame, or `None` when there is not enough buffered.
    fn take_frame(&mut self) -> Result<Option<*mut ffi::AVFrame>, MediaError> {
        if self.buffered() < self.frame_size {
            return Ok(None);
        }
        self.fill(self.frame_size)
    }

    /// Whatever is left, padded with silence.
    fn take_remainder(&mut self) -> Result<Option<*mut ffi::AVFrame>, MediaError> {
        let held = self.buffered();
        if held == 0 {
            return Ok(None);
        }
        for plane in &mut self.pending {
            plane.resize(self.frame_size, 0.0);
        }
        self.fill(self.frame_size)
    }

    /// Copy `count` samples per channel into the reusable frame.
    fn fill(&mut self, count: usize) -> Result<Option<*mut ffi::AVFrame>, MediaError> {
        // SAFETY: `frame` was allocated for `frame_size` samples across
        // `channels` planes, and `count` never exceeds `frame_size`.
        unsafe {
            let raw = &*self.frame.as_ptr();
            for channel in 0..self.channels {
                let destination = raw.data[channel].cast::<f32>();
                if destination.is_null() {
                    return Err(MediaError::DecodeFailed(
                        "audio frame has fewer planes than channels".to_owned(),
                    ));
                }
                std::ptr::copy_nonoverlapping(self.pending[channel].as_ptr(), destination, count);
            }
            (*self.frame.as_ptr()).pts = self.samples;
        }

        for plane in &mut self.pending {
            plane.drain(..count);
        }
        self.samples += count as i64;
        Ok(Some(self.frame.as_ptr()))
    }
}

/// PTS counts frames, so the timebase is the reciprocal of the rate.
fn encoder_timebase(frame_rate: ffi::AVRational) -> ffi::AVRational {
    ffi::AVRational {
        num: frame_rate.den,
        den: frame_rate.num,
    }
}

fn audio_timebase() -> ffi::AVRational {
    ffi::AVRational {
        num: 1,
        den: INTERNAL_SAMPLE_RATE,
    }
}
