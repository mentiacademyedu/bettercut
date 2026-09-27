//! The FFmpeg decoder (§3, §47a).
//!
//! # Which path this is
//!
//! §5 names two paths. The target path keeps decoded frames on the GPU:
//!
//! ```text
//! Hardware decode -> GPU texture -> wgpu compositor -> egui -> screen
//! ```
//!
//! and the fallback downloads to RAM and uploads:
//!
//! ```text
//! Decode -> RAM -> upload -> composite -> screen
//! ```
//!
//! **This is the fallback.** §5 requires it to exist so unsupported hardware
//! degrades rather than fails, and it is what Milestone 4 ships. Hardware
//! decode to texture interop is the harder half and is still open; ADR 005
//! records that the Milestone 0 spike did not validate it either.
//!
//! # Colour
//!
//! §21a.2: every frame is converted into the working space exactly once, at
//! the boundary, and nothing downstream inspects source colour metadata. Here
//! that boundary is `Scaler`, which is told the source's matrix and range
//! explicitly - including the limited-range expansion that §21a calls out as
//! "the one that bites".

use bettercut_foundation::{MediaTime, Rational};
use rusty_ffmpeg::ffi;

use super::raii::{CodecContext, Frame, Packet, Resampler, Scaler};
use super::{INTERNAL_SAMPLE_RATE, InputContext, error_string};
use crate::asset::MediaAsset;
use crate::decoder::{
    AudioBuffer, CancellationToken, FrameStorage, MediaDecoder, SeekMode, VideoFrame,
};
use crate::error::MediaError;

/// The deinterlacer: the top field alone, line-doubled back to full height
/// (a "bob"). Not yadif — that holds every frame back until it has seen the
/// next, which breaks the one-in-one-out chain below and never hands out a
/// lone still. Half the vertical detail, no combing, and every frame the
/// instant it is asked for; applied to every frame, because the flag is the
/// user's word that the file is interlaced, whatever its headers claim.
const DEINTERLACE: &str = "separatefields,select=eq(mod(n\\,2)\\,0),scale=iw:ih*2:flags=bicubic";

/// One decoded stream's plumbing.
struct StreamDecoder {
    /// The container's own stream index, as it appears on every packet.
    ///
    /// **Not** the position in our filtered list. Comparing a packet's
    /// `stream_index` against a list position works right up until a container
    /// has a stream we skipped, and then silently decodes the wrong one.
    index: i32,
    codec: CodecContext,
    /// The stream's own timebase, for converting timestamps (§9).
    timebase: Rational,
    /// Source pixel format and colour, captured at open so the scaler can be
    /// built without re-walking the stream list.
    format: i32,
    full_range: bool,
    colorspace: i32,
    /// How long one frame of this stream lasts, used to decide which frame
    /// contains a given instant. Zero when the container declares no rate.
    frame_duration: MediaTime,
}

pub struct FfmpegDecoder {
    /// Threads FFmpeg may use internally (§15.1). Set once at construction.
    threads: u32,

    input: Option<InputContext>,
    video: Option<StreamDecoder>,
    audio: Option<StreamDecoder>,

    scaler: Option<Scaler>,
    /// §21a.1's tone-mapping chain, when the source is HDR. Its output is
    /// SDR yuv420p, which `scaler` then turns into RGBA exactly as it would a
    /// proxy — so an original and its proxy decode to the same picture.
    tonemap: Option<super::filter::FilterGraph>,
    /// Where the chain's output lands between the two steps. Reused, like
    /// `rgba`, so steady-state playback allocates nothing (§68).
    tonemapped: Frame,
    resampler: Option<Resampler>,

    packet: Packet,
    frame: Frame,

    /// Reused between frames so steady-state playback allocates nothing (§68).
    rgba: Vec<u8>,

    width: u32,
    height: u32,
    /// The clockwise turn applied to every frame, from the asset: a phone's
    /// portrait clip, or its proxy, which is encoded from the stored frames.
    rotation: u16,
    duration: MediaTime,
    /// Resolved once at open, then attached to every frame (§21a.2).
    color: crate::color::ColorMetadata,

    /// End of stream reached; further reads return `Ok(None)` without asking
    /// FFmpeg again.
    drained: bool,

    /// Set by a `Precise` seek: discard frames until the one containing this
    /// instant (§47a.2).
    ///
    /// A container seek only reaches the preceding **keyframe**, which for
    /// long-GOP media can be seconds early. §47a.1 spells out the consequence
    /// and why all-intra proxies (§13.1) matter: with GOP=1 this skips nothing,
    /// with a 250-frame GOP it skips up to 249 decodes.
    skip_until: Option<MediaTime>,

    /// Timestamp of the last frame handed out, which is where the demuxer now
    /// stands. `None` until something has been decoded.
    ///
    /// This is what lets a caller tell "the next frame in the file" from "a
    /// real jump", and therefore what lets ordinary playback avoid seeking at
    /// all (§47a.2).
    position: Option<MediaTime>,
}

// SAFETY: every field is an FFmpeg context that FFmpeg permits one thread to
// use at a time. Moving that ownership between threads is fine; *sharing* it is
// not, which is why `Sync` is deliberately not implemented.
//
// §20a.2's diagram puts decoding on threads other than the one consuming the
// results ("decode threads -> resample -> mixer thread"), so `Send` is required
// rather than incidental. The media README states the same ownership rule.
unsafe impl Send for FfmpegDecoder {}

impl FfmpegDecoder {
    /// The turn the first frame of `asset` asks for in its own side data:
    /// a photo's EXIF orientation, which FFmpeg reports only on the decoded
    /// frame. 0 when it asks for none, or cannot be read.
    pub(crate) fn first_frame_rotation(asset: &MediaAsset) -> u16 {
        let Ok(mut decoder) = Self::new(1) else {
            return 0;
        };
        if decoder.open(asset).is_err() {
            return 0;
        }
        match decoder.pump(true, &crate::decoder::NeverCancelled) {
            Ok(true) => super::frame_rotation(&decoder.frame),
            _ => 0,
        }
    }

    /// Create a decoder capped to `threads` FFmpeg threads.
    ///
    /// §15.1: pass 1 on a 4-core machine. `HardwareProfile` computes it.
    pub fn new(threads: u32) -> Result<Self, MediaError> {
        Ok(Self {
            threads: threads.max(1),
            input: None,
            video: None,
            audio: None,
            scaler: None,
            tonemap: None,
            tonemapped: Frame::new()?,
            resampler: None,
            packet: Packet::new()?,
            frame: Frame::new()?,
            rgba: Vec::new(),
            width: 0,
            height: 0,
            rotation: 0,
            duration: MediaTime::ZERO,
            color: crate::color::ColorMetadata::default(),
            drained: false,
            skip_until: None,
            position: None,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    fn input(&self) -> Result<&InputContext, MediaError> {
        self.input.as_ref().ok_or(MediaError::NotOpen)
    }

    /// Read packets until the requested decoder produces a frame.
    ///
    /// Returns `false` at end of stream. Checks `cancel` **between packets**,
    /// not just on entry: §48 is explicit that cancellation has to be
    /// responsive inside decode loops, because a seek from 00:30 to 12:00 must
    /// abandon the old work immediately (§47a.5).
    fn pump(
        &mut self,
        want_video: bool,
        cancel: &dyn CancellationToken,
    ) -> Result<bool, MediaError> {
        loop {
            if cancel.is_cancelled() {
                return Err(MediaError::Cancelled);
            }

            let stream = match (want_video, &self.video, &self.audio) {
                (true, Some(v), _) => v,
                (false, _, Some(a)) => a,
                _ => return Ok(false),
            };
            let (stream_index, codec_ptr) = (stream.index, stream.codec.as_ptr());

            // Take a frame the decoder is already holding, if there is one.
            // SAFETY: both pointers are valid and owned by `self`.
            let code = unsafe { ffi::avcodec_receive_frame(codec_ptr, self.frame.as_ptr()) };
            if code == 0 {
                return Ok(true);
            }
            if code == averror_eof() {
                self.drained = true;
                return Ok(false);
            }
            if code != averror_again() {
                return Err(MediaError::DecodeFailed(format!(
                    "avcodec_receive_frame: {}",
                    error_string(code)
                )));
            }

            // The decoder needs more input. Resolve the container pointer
            // before touching the packet, so the immutable borrow of `self`
            // ends before the mutable one begins.
            let container = self.input()?.as_mut_ptr();
            self.packet.unref();
            // SAFETY: both pointers are valid; av_read_frame fills the packet.
            let read = unsafe { ffi::av_read_frame(container, self.packet.as_ptr()) };

            if read == averror_eof() {
                // A null packet tells the decoder to emit whatever it still
                // holds. Without this the last few frames of every file are
                // never produced.
                // SAFETY: sending null is the documented way to drain.
                let code = unsafe { ffi::avcodec_send_packet(codec_ptr, std::ptr::null()) };
                if code < 0 && code != averror_eof() {
                    return Err(MediaError::DecodeFailed(format!(
                        "avcodec_send_packet(flush): {}",
                        error_string(code)
                    )));
                }
                continue;
            }
            if read < 0 {
                return Err(MediaError::DecodeFailed(format!(
                    "av_read_frame: {}",
                    error_string(read)
                )));
            }

            if self.packet.stream_index() != stream_index {
                continue; // a packet for some other stream
            }

            // SAFETY: both pointers are valid and the packet holds data.
            let code = unsafe { ffi::avcodec_send_packet(codec_ptr, self.packet.as_ptr()) };
            if code < 0 && code != averror_again() {
                return Err(MediaError::DecodeFailed(format!(
                    "avcodec_send_packet: {}",
                    error_string(code)
                )));
            }
        }
    }

    /// Presentation timestamp of the frame currently held, as `MediaTime`.
    fn frame_timestamp(&self, timebase: Rational) -> MediaTime {
        let pts = self.frame.as_ref().best_effort_timestamp;
        if pts == ffi::AV_NOPTS_VALUE {
            return MediaTime::ZERO;
        }
        MediaTime::from_timebase(pts, timebase).unwrap_or(MediaTime::ZERO)
    }
}

fn unit_timebase() -> Rational {
    Rational::new(1, 1).unwrap_or_else(|| unreachable!("1/1 is valid"))
}

/// How long one frame lasts, from a stream's average frame rate.
///
/// Zero when the container declares no usable rate, which makes the
/// "does this frame contain the target?" test degenerate to "does it start at
/// or after the target" — the safe reading when the rate is unknown.
fn frame_duration(rate: ffi::AVRational) -> MediaTime {
    if rate.num <= 0 || rate.den <= 0 {
        return MediaTime::ZERO;
    }
    // Duration is the reciprocal of the rate, so numerator and denominator swap.
    let inverted = match Rational::new(i64::from(rate.den), i64::from(rate.num)) {
        Some(r) => r,
        None => return MediaTime::ZERO,
    };
    MediaTime::from_timebase(1, inverted).unwrap_or(MediaTime::ZERO)
}

impl MediaDecoder for FfmpegDecoder {
    fn open(&mut self, asset: &MediaAsset) -> Result<(), MediaError> {
        let input = InputContext::open_asset(asset)?;

        let mut video = None;
        let mut audio = None;
        let mut width = 0;
        let mut height = 0;
        let mut color = asset.color;
        let mut hdr_spec: Option<String> = None;
        let mut hdr_input = (0, ffi::AVRational { num: 1, den: 1 });

        for stream in input.streams() {
            // SAFETY: `streams()` filtered nulls; the stream outlives this loop.
            let (params, time_base, index, avg_frame_rate) = unsafe {
                let s = &*stream;
                (s.codecpar, s.time_base, s.index, s.avg_frame_rate)
            };
            if params.is_null() {
                continue;
            }
            let timebase = Rational::new(i64::from(time_base.num), i64::from(time_base.den))
                .unwrap_or_else(unit_timebase);

            // SAFETY: non-null, owned by the stream.
            let par = unsafe { &*params };

            if par.codec_type == ffi::AVMEDIA_TYPE_VIDEO && video.is_none() {
                let codec = CodecContext::open(params, self.threads)?;
                width = par.width.max(0) as u32;
                height = par.height.max(0) as u32;
                hdr_spec = super::filter::hdr_to_sdr_spec(
                    par.color_trc,
                    par.color_primaries,
                    par.color_space,
                    par.color_range,
                    par.width,
                    par.height,
                );
                hdr_input = (par.format, time_base);
                color = super::probe::resolve_color(par, height);
                video = Some(StreamDecoder {
                    index,
                    codec,
                    timebase,
                    format: par.format,
                    full_range: par.color_range == ffi::AVCOL_RANGE_JPEG,
                    colorspace: sws_colorspace(par.color_space),
                    frame_duration: frame_duration(avg_frame_rate),
                });
            } else if par.codec_type == ffi::AVMEDIA_TYPE_AUDIO && audio.is_none() {
                let codec = CodecContext::open(params, self.threads)?;
                let channels = par.ch_layout.nb_channels.max(1) as usize;
                self.resampler = Some(Resampler::to_internal_format(
                    &par.ch_layout,
                    par.format,
                    par.sample_rate.max(1),
                    channels,
                )?);
                audio = Some(StreamDecoder {
                    index,
                    codec,
                    timebase,
                    format: par.format,
                    full_range: false,
                    colorspace: 0,
                    frame_duration: MediaTime::ZERO,
                });
            }
        }

        if video.is_none() && audio.is_none() {
            return Err(MediaError::NoStream(asset.path.clone()));
        }

        // A decoder can be reopened on another file; nothing from the last one
        // may carry over, least of all a tone-mapper built for an HDR source.
        self.tonemap = None;
        // An interlaced original is woven into whole frames first, before any
        // scaling: what the scaler then sees is progressive, the way every
        // other stage expects. The HDR chain below takes the same filter in
        // front of its own.
        if asset.deinterlace && hdr_spec.is_none() && video.is_some() && width > 0 && height > 0 {
            let (format, timebase) = hdr_input;
            self.tonemap = Some(super::filter::FilterGraph::new(
                DEINTERLACE,
                width as i32,
                height as i32,
                format,
                timebase,
                ffi::AVRational { num: 1, den: 1 },
            )?);
            tracing::info!(file = %asset.file_name, "deinterlacing");
        }
        self.scaler = None;

        // A large still comes out scaled to fit `MAX_STILL_EDGE` (see there).
        // Video is left alone: a large video gets a proxy instead (§13).
        let (out_width, out_height) = if asset.is_still() {
            crate::fit_within(width, height, crate::MAX_STILL_EDGE)
        } else {
            (width, height)
        };

        if let Some(v) = &video
            && width > 0
            && height > 0
        {
            self.scaler = Some(match hdr_spec {
                // §21a.1: HDR is tone-mapped to SDR first, by the same chain
                // the proxy encoder uses; the scaler then sees what a proxy
                // would give it — BT.709, limited range, 4:2:0 — and converts
                // that to RGBA the ordinary way.
                Some(spec) => {
                    let (format, timebase) = hdr_input;
                    let spec = if asset.deinterlace {
                        format!("{DEINTERLACE},{spec}")
                    } else {
                        spec
                    };
                    self.tonemap = Some(super::filter::FilterGraph::new(
                        &spec,
                        width as i32,
                        height as i32,
                        format,
                        timebase,
                        ffi::AVRational { num: 1, den: 1 },
                    )?);
                    tracing::info!(file = %asset.file_name, "tone-mapping an HDR original");
                    Scaler::to_rgba(
                        width as i32,
                        height as i32,
                        ffi::AV_PIX_FMT_YUV420P,
                        false,
                        sws_colorspace(ffi::AVCOL_SPC_BT709),
                    )?
                }
                None => Scaler::to_rgba_sized(
                    (width as i32, height as i32),
                    (out_width as i32, out_height as i32),
                    v.format,
                    v.full_range,
                    v.colorspace,
                )?,
            });
        }

        // The HDR path above converts at the source size.
        let (width, height) = self
            .scaler
            .as_ref()
            .map_or((width, height), |s| (s.width as u32, s.height as u32));
        self.rgba = vec![0; (width * height * 4) as usize];
        self.width = width;
        self.height = height;
        self.rotation = asset.rotation;
        self.duration = asset.duration;
        self.color = color;
        self.input = Some(input);
        self.video = video;
        self.audio = audio;
        self.drained = false;
        // Reopening reuses the struct, so a position left over from the
        // previous file would be read as this one's.
        self.position = None;
        self.skip_until = None;

        tracing::debug!(
            file = %asset.file_name,
            width,
            height,
            video = self.video.is_some(),
            audio = self.audio.is_some(),
            threads = self.threads,
            "decoder opened"
        );

        Ok(())
    }

    fn seek(&mut self, timestamp: MediaTime, mode: SeekMode) -> Result<(), MediaError> {
        let input = self.input()?;

        // Seek against the video stream where there is one: the container is
        // indexed by its keyframes.
        let (stream_index, timebase) = match (&self.video, &self.audio) {
            (Some(v), _) => (v.index, v.timebase),
            (None, Some(a)) => (a.index, a.timebase),
            (None, None) => return Err(MediaError::NotOpen),
        };

        let target = timestamp
            .to_timebase(timebase)
            .ok_or_else(|| MediaError::SeekFailed {
                timestamp: format!("{} ticks", timestamp.ticks()),
                reason: "outside the stream's timebase".to_owned(),
            })?;

        // Always seek backwards to a keyframe and decode forward from there.
        // Seeking forward lands *after* the target and shows the wrong picture.
        // §47a.1 is why all-intra proxies matter so much: with GOP=1 the
        // decode-forward step is one frame instead of up to 250.
        let flags = ffi::AVSEEK_FLAG_BACKWARD as i32;

        // SAFETY: `input` is open and the index came from its own stream list.
        let code = unsafe { ffi::av_seek_frame(input.as_mut_ptr(), stream_index, target, flags) };
        if code < 0 {
            return Err(MediaError::SeekFailed {
                timestamp: format!("{} ticks", timestamp.ticks()),
                reason: error_string(code),
            });
        }

        // Without flushing, the decoder keeps emitting pre-seek frames and the
        // picture appears to jump backwards before settling.
        if let Some(v) = &mut self.video {
            v.codec.flush();
        }
        if let Some(a) = &mut self.audio {
            a.codec.flush();
        }
        self.drained = false;
        // The demuxer moved and the codec was flushed, so nothing has been
        // decoded from here yet. Leaving a stale position would let the next
        // request mistake a fresh seek for sequential reading.
        self.position = None;

        // §47a.2 distinguishes the three seek kinds by what they optimise for.
        // Only `Precise` pays to decode forward; `Scrub` wants the lowest
        // latency it can get and a nearby frame is fine, and `Playback` never
        // seeks in the first place.
        self.skip_until = match mode {
            SeekMode::Precise => Some(timestamp),
            SeekMode::Scrub | SeekMode::Playback => None,
        };

        tracing::trace!(?mode, target, "seeked");
        Ok(())
    }

    fn decode_frame(
        &mut self,
        cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError> {
        if self.video.is_none() || self.drained {
            return Ok(None);
        }

        let timebase = self
            .video
            .as_ref()
            .map_or_else(unit_timebase, |v| v.timebase);
        let frame_duration = self
            .video
            .as_ref()
            .map_or(MediaTime::ZERO, |v| v.frame_duration);

        // §47a.2: a `Precise` seek must land on the exact frame. The container
        // seek only reached the preceding keyframe, so decode forward and drop
        // everything that ends before the target. The frame we want is the one
        // whose span *contains* it.
        let timestamp = loop {
            self.frame.unref();
            if !self.pump(true, cancel)? {
                // Ran out before reaching the target: the request was past the
                // end of the stream. Clear the skip so the decoder is reusable.
                self.skip_until = None;
                return Ok(None);
            }
            let timestamp = self.frame_timestamp(timebase);

            match self.skip_until {
                Some(target) if timestamp + frame_duration <= target => continue,
                Some(_) => {
                    self.skip_until = None;
                    break timestamp;
                }
                None => break timestamp,
            }
        };

        // §21a.2: the one conversion, at the boundary — preceded by §21a.1's
        // tone-map when the source is HDR.
        let scaler = self
            .scaler
            .as_mut()
            .ok_or_else(|| MediaError::DecodeFailed("no scaler for this stream".to_owned()))?;
        match self.tonemap.as_mut() {
            Some(graph) => {
                graph.push(&self.frame)?;
                // The chain is 1:1, so a frame in yields a frame out. If it
                // does not, converting the untone-mapped frame instead would
                // quietly produce the dark picture this exists to prevent.
                if !graph.pull(&mut self.tonemapped)? {
                    return Err(MediaError::DecodeFailed(
                        "the HDR tone-mapping chain produced no frame".to_owned(),
                    ));
                }
                scaler.convert(&self.tonemapped, &mut self.rgba)?;
            }
            None => scaler.convert(&self.frame, &mut self.rgba)?,
        }

        // Where the demuxer now stands, so the next request can tell whether
        // it is simply the next frame along (§47a.2's `Playback`) or a real
        // seek.
        self.position = Some(timestamp);

        // Turned upright last, after every conversion, so the tone-mapper and
        // the scaler work on the frame as it is stored.
        let (data, width, height) = if self.rotation == 0 {
            (self.rgba.clone(), self.width, self.height)
        } else {
            super::rotate_rgba(&self.rgba, self.width, self.height, self.rotation)
        };

        Ok(Some(VideoFrame {
            timestamp,
            width,
            height,
            color: self.color,
            // The §5 fallback: pixels in system RAM, to be uploaded.
            storage: FrameStorage::System {
                data,
                stride: width * 4,
            },
        }))
    }

    fn decode_frame_at(
        &mut self,
        target: MediaTime,
        cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError> {
        // Exactly the rule a `Precise` seek uses to pick its frame, reused
        // rather than restated: the frame wanted is the one whose span
        // contains `target`. Two copies of that comparison would eventually
        // disagree by one frame, and a preview off by one frame from the
        // decode-ahead ring is the kind of fault that looks like a stutter.
        //
        // The difference from `seek` is everything it does *not* do: no
        // container seek, no codec flush. It just keeps reading.
        self.skip_until = Some(target);
        self.decode_frame(cancel)
    }

    fn position(&self) -> Option<MediaTime> {
        self.position
    }

    fn frame_duration(&self) -> MediaTime {
        self.video
            .as_ref()
            .map_or(MediaTime::ZERO, |v| v.frame_duration)
    }

    fn decode_audio(
        &mut self,
        cancel: &dyn CancellationToken,
    ) -> Result<Option<AudioBuffer>, MediaError> {
        if self.audio.is_none() {
            return Ok(None);
        }
        self.frame.unref();
        if !self.pump(false, cancel)? {
            return Ok(None);
        }

        let timebase = self
            .audio
            .as_ref()
            .map_or_else(unit_timebase, |a| a.timebase);
        let timestamp = self.frame_timestamp(timebase);

        let resampler = self
            .resampler
            .as_mut()
            .ok_or_else(|| MediaError::DecodeFailed("no resampler for this stream".to_owned()))?;
        let planes = resampler.convert(&self.frame)?;

        Ok(Some(AudioBuffer {
            timestamp,
            planes,
            // §20a.3: always, by construction.
            sample_rate: INTERNAL_SAMPLE_RATE as u32,
        }))
    }

    fn duration(&self) -> MediaTime {
        self.duration
    }
}

/// `AVERROR(EAGAIN)` - "send more input".
fn averror_again() -> i32 {
    -(ffi::EAGAIN as i32)
}

/// `AVERROR_EOF`. FFmpeg defines it as a negated FourCC, and the binding does
/// not export the macro.
fn averror_eof() -> i32 {
    -((b'E' as i32) | ((b'O' as i32) << 8) | ((b'F' as i32) << 16) | ((b' ' as i32) << 24))
}

/// `AVERROR_EOF`, for the encoder path in `encode.rs`.
pub(crate) fn averror_eof_code() -> i32 {
    averror_eof()
}

/// Map an `AVColorSpace` to the swscale colour-space id, for `encode.rs`.
pub(crate) fn sws_colorspace_of(space: ffi::AVColorSpace) -> i32 {
    sws_colorspace(space)
}

/// Map an `AVColorSpace` to the swscale colour-space id.
fn sws_colorspace(space: ffi::AVColorSpace) -> i32 {
    if space == ffi::AVCOL_SPC_BT709 {
        ffi::SWS_CS_ITU709 as i32
    } else if space == ffi::AVCOL_SPC_BT470BG || space == ffi::AVCOL_SPC_SMPTE170M {
        ffi::SWS_CS_ITU601 as i32
    } else if space == ffi::AVCOL_SPC_BT2020_NCL {
        ffi::SWS_CS_BT2020 as i32
    } else {
        ffi::SWS_CS_DEFAULT as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{MediaProber, NeverCancelled};
    use crate::ffmpeg::FfmpegProber;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn open(name: &str) -> FfmpegDecoder {
        let asset = FfmpegProber.probe(&fixture(name)).expect("probe");
        let mut decoder = FfmpegDecoder::new(1).expect("allocate");
        decoder.open(&asset).expect("open");
        decoder
    }

    fn mean_luma(frame: &VideoFrame) -> f64 {
        let crate::FrameStorage::System { data, .. } = &frame.storage else {
            panic!("expected a RAM frame");
        };
        data.chunks_exact(4)
            .map(|p| (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) as f64 / 3.0)
            .sum::<f64>()
            / (data.len() / 4) as f64
    }

    /// §21a.1 on the path that decodes *originals*, which is the path export
    /// takes (§14: proxies for editing, originals for export).
    ///
    /// The proxy encoder tone-maps HDR, so the preview looked right — and the
    /// export, reading the original, came out around half as bright, because
    /// swscale cannot convert a transfer function. Measured on this fixture:
    /// mean luma ~122 untone-mapped, ~227 tone-mapped.
    #[test]
    fn an_hdr_original_decodes_tone_mapped() {
        let mut decoder = open("hlg-bt2020.mkv");
        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");

        let mean = mean_luma(&frame);
        assert!(
            mean > 150.0,
            "an HDR original decoded to mean luma {mean:.1}; it was not tone-mapped \
             (untone-mapped measures ~122, tone-mapped ~227)"
        );
    }

    #[test]
    fn a_small_still_decodes_at_its_own_size() {
        let mut decoder = open("still.png");
        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");
        assert_eq!((frame.width, frame.height), (320, 180));
    }

    /// A photo wider than `MAX_STILL_EDGE` comes out scaled to fit, keeping
    /// its shape and its colour.
    #[test]
    fn a_large_still_is_scaled_to_fit() {
        let (width, height) = (5000u32, 100u32);
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        for _ in 0..width * height {
            ppm.extend_from_slice(&[200, 40, 20]);
        }
        let dir = std::env::temp_dir().join(format!("bettercut-still-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("wide.ppm");
        std::fs::write(&path, ppm).expect("write");

        let asset = FfmpegProber.probe(&path).expect("probe");
        assert!(asset.is_still(), "setup: a PPM is a still");
        let mut decoder = FfmpegDecoder::new(1).expect("allocate");
        decoder.open(&asset).expect("open");
        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!((frame.width, frame.height), (4096, 80));
        let crate::FrameStorage::System { data, .. } = &frame.storage else {
            panic!("expected a RAM frame");
        };
        assert_eq!(data.len(), 4096 * 80 * 4);
        let middle = (40 * 4096 + 2048) * 4;
        let pixel = &data[middle..middle + 3];
        for (got, want) in pixel.iter().zip([200u8, 40, 20]) {
            assert!(got.abs_diff(want) <= 2, "colour changed: {pixel:?}");
        }
    }

    /// An SDR source must come out exactly as before — the tone-mapping chain
    /// is for HDR only, and running SDR through it would shift every colour.
    #[test]
    fn an_sdr_source_is_not_sent_through_the_tone_mapper() {
        let decoder = open("ntsc-2997.mp4");
        assert!(
            decoder.tonemap.is_none(),
            "an SDR source was given a tone-mapping chain"
        );
    }

    /// A phone photo with EXIF orientation 6: the left edge (blue) ends up
    /// along the top.
    #[test]
    fn an_exif_rotated_photo_decodes_upright() {
        let asset = FfmpegProber.probe(&fixture("exif-6.jpg")).expect("probe");
        let mut decoder = FfmpegDecoder::new(1).expect("decoder");
        decoder.open(&asset).expect("open");
        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");
        assert_eq!((frame.width, frame.height), (32, 64));
        let FrameStorage::System { data, .. } = &frame.storage else {
            panic!("system memory expected");
        };
        let (r, b) = (data[0], data[2]);
        assert!(
            b > 150 && r < 100,
            "the top left is not the blue edge: r {r} b {b}"
        );
    }

    /// A phone's portrait clip: stored landscape with a note to turn it. The
    /// frames come out turned, pixel for pixel the stored frame rotated.
    #[test]
    fn a_rotated_file_decodes_upright() {
        let mut turned = open("rotated-90.mp4");
        let mut plain = open("ntsc-2997.mp4");
        let upright = turned
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");
        let stored = plain
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");
        assert_eq!((upright.width, upright.height), (360, 640));
        let (FrameStorage::System { data: got, .. }, FrameStorage::System { data: raw, .. }) =
            (&upright.storage, &stored.storage)
        else {
            panic!("frames in system memory expected");
        };
        let (expected, _, _) = crate::ffmpeg::rotate_rgba(raw, 640, 360, 270);
        assert!(
            got == &expected,
            "the turned frame is not the stored one rotated"
        );
    }

    #[test]
    fn decodes_video_frames_in_order() {
        let mut decoder = open("ntsc-2997.mp4");
        assert!(decoder.has_video());
        assert_eq!((decoder.width(), decoder.height()), (640, 360));

        let mut previous = MediaTime::from_ticks(-1);
        let mut count = 0;
        while let Some(frame) = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode should not fail")
        {
            assert_eq!((frame.width, frame.height), (640, 360));
            assert!(
                frame.timestamp >= previous,
                "timestamps went backwards: {:?} then {:?}",
                previous,
                frame.timestamp
            );
            previous = frame.timestamp;

            match &frame.storage {
                FrameStorage::System { data, stride } => {
                    assert_eq!(*stride, 640 * 4);
                    assert_eq!(data.len(), 640 * 360 * 4);
                }
                FrameStorage::Gpu { .. } => panic!("software decode produced a GPU frame"),
            }
            count += 1;
        }

        // 2 seconds at 29.97 fps.
        assert!(
            (58..=62).contains(&count),
            "expected about 60 frames, got {count}"
        );
    }

    /// The decoded picture must not be uniformly black or uniformly grey: that
    /// is what a broken pixel-format conversion produces, and it would sail
    /// past a test that only checks buffer sizes.
    #[test]
    fn decoded_frames_contain_real_picture_data() {
        let mut decoder = open("ntsc-2997.mp4");
        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("at least one frame");

        let FrameStorage::System { data, .. } = &frame.storage else {
            panic!("expected a RAM frame");
        };

        let mut min = u8::MAX;
        let mut max = u8::MIN;
        for pixel in data.chunks_exact(4) {
            for channel in &pixel[..3] {
                min = min.min(*channel);
                max = max.max(*channel);
            }
            // swscale must fill alpha; a zero alpha would make the whole
            // preview invisible once composited.
            assert_eq!(pixel[3], 255, "alpha was not opaque");
        }

        assert!(
            max - min > 40,
            "frame has almost no contrast (min {min}, max {max}) - conversion is probably wrong"
        );
    }

    /// §47a.2: a precise seek lands on the frame *containing* the target, not
    /// merely on the preceding keyframe.
    ///
    /// This fixture is a single-GOP file, so a container seek alone would
    /// always return frame 0 and this test would fail — which is exactly the
    /// long-GOP behaviour §47a.1 describes and all-intra proxies avoid.
    #[test]
    fn a_precise_seek_lands_on_the_frame_containing_the_target() {
        let mut decoder = open("ntsc-2997.mp4");
        // One 29.97 fps frame is 32,032 ticks.
        let frame_ticks = 32_032;

        for frame_index in [0_i64, 5, 17, 30, 44] {
            let target = MediaTime::from_ticks(frame_index * frame_ticks);
            decoder.seek(target, SeekMode::Precise).expect("seek");

            let frame = decoder
                .decode_frame(&NeverCancelled)
                .expect("decode")
                .expect("a frame after seeking");

            let delta = (frame.timestamp.ticks() - target.ticks()).abs();
            assert!(
                delta < frame_ticks,
                "seeking to frame {frame_index} ({} ticks) returned {} ticks, \
                 which is {delta} away - more than one frame",
                target.ticks(),
                frame.timestamp.ticks()
            );
        }
    }

    /// §47a.2's `Playback`: reading forward must land on exactly the frames a
    /// seek would have found.
    ///
    /// This is the property the whole sequential path rests on. If the two
    /// disagree by a frame, playback shows something one frame off from what
    /// stepping and scrubbing show, and the decode-ahead ring fills with
    /// frames the preview will never ask for.
    #[test]
    fn reading_forward_returns_the_same_frames_as_seeking_to_each() {
        let frame_ticks = 32_032; // one 29.97 fps frame
        let frames = 24;

        let mut seeking = open("ntsc-2997.mp4");
        let mut expected = Vec::new();
        for index in 0..frames {
            let target = MediaTime::from_ticks(index * frame_ticks);
            seeking.seek(target, SeekMode::Precise).expect("seek");
            let frame = seeking
                .decode_frame(&NeverCancelled)
                .expect("decode")
                .expect("a frame");
            expected.push(frame.timestamp.ticks());
        }

        let mut sequential = open("ntsc-2997.mp4");
        let mut actual = Vec::new();
        for index in 0..frames {
            let target = MediaTime::from_ticks(index * frame_ticks);
            // Only the first request seeks; from then on the decoder is
            // already sitting on the previous frame.
            let frame = if index == 0 {
                sequential.seek(target, SeekMode::Precise).expect("seek");
                sequential.decode_frame(&NeverCancelled)
            } else {
                sequential.decode_frame_at(target, &NeverCancelled)
            }
            .expect("decode")
            .expect("a frame");
            actual.push(frame.timestamp.ticks());
        }

        assert_eq!(
            actual, expected,
            "reading forward disagreed with seeking to each frame"
        );
    }

    /// The position is what tells a caller "the next frame" from "a jump", so
    /// it has to be absent exactly when there is nothing to read forward from.
    #[test]
    fn position_is_only_set_once_something_has_been_decoded() {
        let mut decoder = open("ntsc-2997.mp4");
        assert_eq!(decoder.position(), None, "nothing decoded yet");
        assert!(decoder.frame_duration().ticks() > 0, "29.97 fps is known");

        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");
        assert_eq!(decoder.position(), Some(frame.timestamp));

        // A seek moves the demuxer and flushes the codec, so the old position
        // no longer describes where the next read will start.
        decoder
            .seek(MediaTime::from_millis(500), SeekMode::Precise)
            .expect("seek");
        assert_eq!(decoder.position(), None, "a seek must clear the position");
    }

    /// §47a.1, made concrete: this fixture is single-GOP, so seeking to frame
    /// N decodes N frames to get there. Playing it that way is quadratic, and
    /// that is exactly what made playback crawl on media without a proxy.
    ///
    /// Timing in a test is normally a bad idea. It earns its place here because
    /// the difference is asymptotic rather than constant — 60 frames costs 60
    /// decodes one way and about 1,800 the other — so the margin below is met
    /// by a wide margin on any machine that can run the suite at all. Measured
    /// at 27 ms against 377 ms - 14x - on a 640x360 fixture; the gap widens
    /// with resolution and with GOP length.
    #[test]
    fn reading_forward_is_dramatically_cheaper_than_seeking_each_frame() {
        let frame_ticks = 32_032;
        let frames = 50;

        let mut sequential = open("ntsc-2997.mp4");
        let start = std::time::Instant::now();
        sequential
            .seek(MediaTime::ZERO, SeekMode::Precise)
            .expect("seek");
        sequential.decode_frame(&NeverCancelled).expect("decode");
        for index in 1..frames {
            sequential
                .decode_frame_at(MediaTime::from_ticks(index * frame_ticks), &NeverCancelled)
                .expect("decode");
        }
        let forward = start.elapsed();

        let mut seeking = open("ntsc-2997.mp4");
        let start = std::time::Instant::now();
        for index in 0..frames {
            seeking
                .seek(
                    MediaTime::from_ticks(index * frame_ticks),
                    SeekMode::Precise,
                )
                .expect("seek");
            seeking.decode_frame(&NeverCancelled).expect("decode");
        }
        let seeked = start.elapsed();

        assert!(
            forward * 3 < seeked,
            "reading forward took {forward:?} and seeking each frame took {seeked:?}; \
             the sequential path is not paying off"
        );
    }

    /// A scrub seek does not pay to decode forward: §47a.2 says lowest latency
    /// wins there, and a nearby frame is acceptable while dragging.
    #[test]
    fn a_scrub_seek_does_not_decode_forward() {
        let mut decoder = open("ntsc-2997.mp4");
        let target = MediaTime::from_millis(1_000);

        decoder.seek(target, SeekMode::Scrub).expect("seek");
        let frame = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("a frame");

        // It must not overshoot; landing early is the accepted trade.
        assert!(frame.timestamp <= target, "scrub seek overshot the target");
    }

    #[test]
    fn seeking_past_the_end_yields_no_frame_and_leaves_the_decoder_usable() {
        let mut decoder = open("ntsc-2997.mp4");

        decoder
            .seek(MediaTime::from_seconds(30), SeekMode::Precise)
            .expect("seek should be accepted even past the end");
        // Either no frame, or one at most a frame from the end; both are fine.
        let _ = decoder.decode_frame(&NeverCancelled).expect("no error");

        // The decoder must still work afterwards.
        decoder
            .seek(MediaTime::ZERO, SeekMode::Precise)
            .expect("seek");
        assert!(
            decoder
                .decode_frame(&NeverCancelled)
                .expect("decode")
                .is_some(),
            "decoder was left unusable after seeking past the end"
        );
    }

    #[test]
    fn seeking_back_to_zero_replays_the_file() {
        let mut decoder = open("ntsc-2997.mp4");

        let first = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("frame");

        // Drain a few frames, then rewind.
        for _ in 0..10 {
            let _ = decoder.decode_frame(&NeverCancelled).expect("decode");
        }
        decoder
            .seek(MediaTime::ZERO, SeekMode::Precise)
            .expect("seek");

        let again = decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .expect("frame after rewind");
        assert_eq!(
            again.timestamp, first.timestamp,
            "rewinding did not return to the first frame"
        );
    }

    /// §20a.3: audio always arrives 48 kHz, f32, planar - whatever the source.
    #[test]
    fn audio_is_normalized_to_the_internal_format() {
        let mut decoder = open("ntsc-2997.mp4");
        assert!(decoder.has_audio());

        let buffer = decoder
            .decode_audio(&NeverCancelled)
            .expect("decode")
            .expect("an audio buffer");

        assert_eq!(buffer.sample_rate, 48_000);
        assert_eq!(buffer.channels(), 2);
        assert!(buffer.frames() > 0);
        // Planar means one plane per channel, all the same length.
        assert!(buffer.planes.iter().all(|p| p.len() == buffer.frames()));
        // A 440 Hz tone is not silence.
        assert!(
            buffer.planes[0].iter().any(|s| s.abs() > 0.01),
            "decoded audio is silent"
        );
    }

    #[test]
    fn audio_only_files_decode_without_a_video_stream() {
        let mut decoder = open("tone-48k.wav");
        assert!(!decoder.has_video());
        assert!(decoder.has_audio());

        assert!(
            decoder
                .decode_frame(&NeverCancelled)
                .expect("no error")
                .is_none(),
            "a file with no video produced a video frame"
        );
        assert!(
            decoder
                .decode_audio(&NeverCancelled)
                .expect("decode")
                .is_some()
        );
    }

    /// §48: cancellation is checked inside the decode loop, not only between
    /// jobs, so a seek can abandon in-flight work immediately.
    #[test]
    fn decoding_stops_promptly_when_cancelled() {
        struct AlwaysCancelled;
        impl CancellationToken for AlwaysCancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let mut decoder = open("ntsc-2997.mp4");
        let result = decoder.decode_frame(&AlwaysCancelled);
        assert!(matches!(result, Err(MediaError::Cancelled)));
        assert!(
            result.err().is_some_and(|e| e.is_cancellation()),
            "cancellation should be distinguishable from a real failure"
        );
    }

    /// Decoding to the end and then rewinding must work; a decoder that latches
    /// "drained" forever cannot loop playback or be reused from the pool.
    #[test]
    fn a_drained_decoder_recovers_after_seeking() {
        let mut decoder = open("ntsc-2997.mp4");
        while decoder
            .decode_frame(&NeverCancelled)
            .expect("decode")
            .is_some()
        {}

        decoder
            .seek(MediaTime::ZERO, SeekMode::Precise)
            .expect("seek");
        assert!(
            decoder
                .decode_frame(&NeverCancelled)
                .expect("decode")
                .is_some(),
            "decoder stayed drained after a seek"
        );
    }
}
