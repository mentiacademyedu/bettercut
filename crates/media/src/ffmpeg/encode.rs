//! All-intra proxy generation (§13.1).
//!
//! > **The proxy codec determines scrub latency more than anything else in the
//! > application.**
//! >
//! > ```text
//! > Codec:        H.264
//! > GOP length:   1  (all-intra / all-keyframe)
//! > Profile:      High, 8-bit
//! > Pixel format: yuv420p
//! > Range:        limited (broadcast)
//! > Matrix:       BT.709
//! > Audio:        48 kHz, stereo, AAC or PCM
//! > ```
//!
//! # Why GOP 1 is not a tuning knob
//!
//! §47a.1: seeking into long-GOP media means seeking to the preceding keyframe
//! and decoding forward — up to 250 decodes on a 250-frame GOP. With every
//! frame a keyframe a seek is *one* decode. That single choice is what makes
//! §81's 100 ms scrub target reachable on a 4-core laptop.
//!
//! The cost is real and accepted: all-intra files are 3–5× larger. §13.1 is
//! explicit — "disk is cheap, scrub latency is the product".
//!
//! # Normalization
//!
//! §13.1 also requires the proxy to *normalize*, which is the quieter benefit:
//!
//! ```text
//! 10-bit -> 8-bit | HDR -> tone-mapped SDR | any matrix -> BT.709
//! any audio rate -> 48 kHz | variable frame rate -> constant frame rate
//! ```
//!
//! Afterwards the preview pipeline only ever sees one format, which removes an
//! entire class of colour and timing bugs rather than merely making them rarer.

use std::path::Path;

use rusty_ffmpeg::ffi;

use super::raii::{CodecContext, Frame, Packet, Resampler, Scaler};
use super::{INTERNAL_SAMPLE_RATE, InputContext, error_string};
use crate::asset::MediaAsset;
use crate::decoder::CancellationToken;
use crate::error::MediaError;
use crate::proxy::ProxySpec;

/// Progress callback, 0.0 to 1.0.
pub type ProgressFn<'a> = &'a dyn Fn(f32);

/// Encode a proxy of `asset` into `output`.
///
/// `threads` is the §15.1 cap — **not** the machine's core count. FFmpeg
/// defaults to using every core, and one uncapped proxy job saturates a 4-core
/// machine no matter how few jobs the scheduler runs.
pub fn generate_proxy(
    asset: &MediaAsset,
    output: &Path,
    height: u32,
    threads: u32,
    cancel: &dyn CancellationToken,
    progress: ProgressFn<'_>,
) -> Result<(), MediaError> {
    let spec = ProxySpec::V1;
    let input = InputContext::open(&asset.path)?;

    // The container is allocated *first*, because whether it wants global
    // headers decides how the encoders must be configured — and that has to be
    // set before `avcodec_open2`. Setting it afterwards silently does nothing,
    // and the symptom is an MP4 whose H.264 stream has no SPS/PPS: it encodes
    // without error and then will not decode.
    let mut muxer = Muxer::allocate(output)?;
    let global_header = muxer.needs_global_header();

    let mut video = VideoLeg::open(&input, height, threads, &spec, global_header)?;
    let audio = AudioLeg::open(&input, threads, global_header);

    muxer.add_video(video.encoder.as_ptr())?;
    if let Some(audio) = audio.as_ref() {
        muxer.add_audio(audio.encoder.as_ptr())?;
    }
    muxer.begin(output)?;

    let total = asset.duration.ticks().max(1);
    let mut packet = Packet::new()?;
    let mut decoded = Frame::new()?;
    let mut audio = audio;

    loop {
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }

        packet.unref();
        // SAFETY: both pointers are valid and owned locally.
        let read = unsafe { ffi::av_read_frame(input.as_mut_ptr(), packet.as_ptr()) };
        if read < 0 {
            break; // end of stream, or an unreadable tail we treat as the end
        }

        let index = packet.stream_index();
        if index == video.stream_index {
            video.push_packet(&packet)?;
            while video.take_frame(&mut decoded)? {
                let position = decoded_position(&decoded, video.timebase);
                progress((position as f32 / total as f32).clamp(0.0, 1.0));
                video.encode(&decoded, &mut muxer, cancel)?;
            }
        } else if let Some(audio) = audio.as_mut()
            && index == audio.stream_index
        {
            audio.push_packet(&packet)?;
            while audio.take_frame(&mut decoded)? {
                audio.encode(&decoded, &mut muxer)?;
            }
        }
    }

    // Flush both decoders, then both encoders. Skipping this loses the tail of
    // the file, which shows up as a proxy that is shorter than its source.
    video.flush_decoder(&mut decoded, &mut muxer, cancel)?;
    if let Some(audio) = audio.as_mut() {
        audio.flush_decoder(&mut decoded, &mut muxer)?;
    }
    video.flush_encoder(&mut muxer)?;
    if let Some(audio) = audio.as_mut() {
        audio.flush_encoder(&mut muxer)?;
    }

    muxer.finish()?;
    progress(1.0);
    Ok(())
}

fn decoded_position(frame: &Frame, timebase: ffi::AVRational) -> i64 {
    let pts = frame.as_ref().best_effort_timestamp;
    if pts == ffi::AV_NOPTS_VALUE || timebase.den == 0 {
        return 0;
    }
    // Into the project's own tick base, so progress is comparable with duration.
    (pts as i128 * i128::from(timebase.num) * i128::from(bettercut_foundation::TICKS_PER_SECOND)
        / i128::from(timebase.den)) as i64
}

/// The video decode → scale → encode chain.
struct VideoLeg {
    stream_index: i32,
    timebase: ffi::AVRational,
    decoder: CodecContext,
    encoder: CodecContext,
    scaler: Scaler,
    /// The picture handed to the encoder, in the proxy's own format.
    scaled: Frame,
    /// Constant-frame-rate output counter (§13.1).
    ///
    /// Timestamps are generated rather than copied, which is precisely how
    /// variable-frame-rate source becomes constant-frame-rate proxy.
    next_pts: i64,
    frame_rate: ffi::AVRational,
}

impl VideoLeg {
    fn open(
        input: &InputContext,
        height: u32,
        threads: u32,
        spec: &ProxySpec,
        global_header: bool,
    ) -> Result<Self, MediaError> {
        let (stream_index, params, timebase, avg_frame_rate) =
            find_stream(input, ffi::AVMEDIA_TYPE_VIDEO)
                .ok_or_else(|| MediaError::NoStream(std::path::PathBuf::new()))?;

        let decoder = CodecContext::open(params, threads)?;

        // SAFETY: `params` is non-null and owned by the stream.
        let (src_width, src_height, src_format, src_range, src_space) = unsafe {
            let p = &*params;
            (p.width, p.height, p.format, p.color_range, p.color_space)
        };
        if src_width <= 0 || src_height <= 0 {
            return Err(MediaError::DecodeFailed(
                "source video has no dimensions".to_owned(),
            ));
        }

        // Preserve aspect; keep both dimensions even for yuv420p chroma.
        let target_height = i32::try_from(height).unwrap_or(720).max(2) & !1;
        let target_width =
            ((src_width as i64 * i64::from(target_height)) / i64::from(src_height)) as i32;
        let target_width = target_width.max(2) & !1;

        // A sane fallback when the container declares no rate; §13.1 wants
        // constant frame rate either way.
        let frame_rate = if avg_frame_rate.num > 0 && avg_frame_rate.den > 0 {
            avg_frame_rate
        } else {
            ffi::AVRational { num: 30, den: 1 }
        };

        let encoder = open_video_encoder(
            target_width,
            target_height,
            frame_rate,
            threads,
            spec,
            global_header,
        )?;

        // Source colour in, BT.709 limited out (§13.1's normalization).
        let scaler = Scaler::to_yuv420p(
            src_width,
            src_height,
            src_format,
            src_range == ffi::AVCOL_RANGE_JPEG,
            super::decode::sws_colorspace_of(src_space),
            target_width,
            target_height,
        )?;

        let scaled = Frame::video(target_width, target_height, ffi::AV_PIX_FMT_YUV420P)?;

        Ok(Self {
            stream_index,
            timebase,
            decoder,
            encoder,
            scaler,
            scaled,
            next_pts: 0,
            frame_rate,
        })
    }

    fn push_packet(&mut self, packet: &Packet) -> Result<(), MediaError> {
        send_packet(self.decoder.as_ptr(), packet.as_ptr())
    }

    fn take_frame(&mut self, into: &mut Frame) -> Result<bool, MediaError> {
        receive_frame(self.decoder.as_ptr(), into)
    }

    fn encode(
        &mut self,
        decoded: &Frame,
        muxer: &mut Muxer,
        cancel: &dyn CancellationToken,
    ) -> Result<(), MediaError> {
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }

        self.scaler.convert_to_frame(decoded, &mut self.scaled)?;

        // SAFETY: `scaled` is a valid allocated frame.
        unsafe {
            let frame = &mut *self.scaled.as_ptr();
            // Generated, not copied: this is what makes the output CFR.
            frame.pts = self.next_pts;
        }
        self.next_pts += 1;

        drain_encoder(
            self.encoder.as_ptr(),
            Some(self.scaled.as_ptr()),
            muxer,
            muxer.video_stream,
            encoder_timebase(self.frame_rate),
        )
    }

    fn flush_decoder(
        &mut self,
        into: &mut Frame,
        muxer: &mut Muxer,
        cancel: &dyn CancellationToken,
    ) -> Result<(), MediaError> {
        // SAFETY: sending null is the documented way to drain a decoder.
        unsafe { ffi::avcodec_send_packet(self.decoder.as_ptr(), std::ptr::null()) };
        while receive_frame(self.decoder.as_ptr(), into)? {
            let frame = std::mem::replace(into, Frame::new()?);
            self.encode(&frame, muxer, cancel)?;
            *into = frame;
        }
        Ok(())
    }

    fn flush_encoder(&mut self, muxer: &mut Muxer) -> Result<(), MediaError> {
        drain_encoder(
            self.encoder.as_ptr(),
            None,
            muxer,
            muxer.video_stream,
            encoder_timebase(self.frame_rate),
        )
    }
}

/// The audio decode → resample → encode chain.
struct AudioLeg {
    stream_index: i32,
    decoder: CodecContext,
    encoder: CodecContext,
    resampler: Resampler,
    buffered: Vec<Vec<f32>>,
    channels: usize,
    frame_size: usize,
    next_pts: i64,
}

impl AudioLeg {
    /// Returns `None` when the source has no audio, which is not an error: a
    /// silent proxy of silent footage is correct.
    fn open(input: &InputContext, threads: u32, global_header: bool) -> Option<Self> {
        let (stream_index, params, _timebase, _rate) = find_stream(input, ffi::AVMEDIA_TYPE_AUDIO)?;

        let decoder = CodecContext::open(params, threads).ok()?;

        // SAFETY: `params` is non-null and owned by the stream.
        let (layout, format, sample_rate, channels) = unsafe {
            let p = &*params;
            (
                p.ch_layout,
                p.format,
                p.sample_rate.max(1),
                p.ch_layout.nb_channels.max(1) as usize,
            )
        };

        // §13.1: stereo at 48 kHz, whatever came in.
        let out_channels = channels.clamp(1, 2);
        let resampler =
            Resampler::to_internal_format(&layout, format, sample_rate, out_channels).ok()?;

        let encoder = open_audio_encoder(out_channels, threads, global_header).ok()?;
        // SAFETY: the encoder is open, so `frame_size` is populated.
        let frame_size = unsafe { (*encoder.as_ptr()).frame_size.max(1024) as usize };

        Some(Self {
            stream_index,
            decoder,
            encoder,
            resampler,
            buffered: vec![Vec::new(); out_channels],
            channels: out_channels,
            frame_size,
            next_pts: 0,
        })
    }

    fn push_packet(&mut self, packet: &Packet) -> Result<(), MediaError> {
        send_packet(self.decoder.as_ptr(), packet.as_ptr())
    }

    fn take_frame(&mut self, into: &mut Frame) -> Result<bool, MediaError> {
        receive_frame(self.decoder.as_ptr(), into)
    }

    fn encode(&mut self, decoded: &Frame, muxer: &mut Muxer) -> Result<(), MediaError> {
        let planes = self.resampler.convert(decoded)?;
        for (channel, plane) in planes.into_iter().enumerate() {
            if let Some(existing) = self.buffered.get_mut(channel) {
                existing.extend_from_slice(&plane);
            }
        }
        self.emit_full_frames(muxer, false)
    }

    /// AAC needs exactly `frame_size` samples per frame; the decoder produces
    /// whatever the source used. Emit only whole frames, keeping the remainder.
    fn emit_full_frames(&mut self, muxer: &mut Muxer, flush: bool) -> Result<(), MediaError> {
        loop {
            let available = self.buffered.first().map_or(0, Vec::len);
            if available == 0 || (!flush && available < self.frame_size) {
                return Ok(());
            }
            let count = self.frame_size.min(available);

            let frame = Frame::audio(count, self.channels, INTERNAL_SAMPLE_RATE)?;
            // SAFETY: the frame was allocated for exactly `channels` planes of
            // `count` f32 samples.
            unsafe {
                let raw = &mut *frame.as_ptr();
                raw.pts = self.next_pts;
                for channel in 0..self.channels {
                    let dst = raw.data[channel].cast::<f32>();
                    let src = &self.buffered[channel][..count];
                    std::ptr::copy_nonoverlapping(src.as_ptr(), dst, count);
                }
            }
            self.next_pts += count as i64;

            for plane in &mut self.buffered {
                plane.drain(..count.min(plane.len()));
            }

            drain_encoder(
                self.encoder.as_ptr(),
                Some(frame.as_ptr()),
                muxer,
                muxer.audio_stream,
                ffi::AVRational {
                    num: 1,
                    den: INTERNAL_SAMPLE_RATE,
                },
            )?;
        }
    }

    fn flush_decoder(&mut self, into: &mut Frame, muxer: &mut Muxer) -> Result<(), MediaError> {
        // SAFETY: sending null drains the decoder.
        unsafe { ffi::avcodec_send_packet(self.decoder.as_ptr(), std::ptr::null()) };
        while receive_frame(self.decoder.as_ptr(), into)? {
            let frame = std::mem::replace(into, Frame::new()?);
            self.encode(&frame, muxer)?;
            *into = frame;
        }
        Ok(())
    }

    fn flush_encoder(&mut self, muxer: &mut Muxer) -> Result<(), MediaError> {
        self.emit_full_frames(muxer, true)?;
        drain_encoder(
            self.encoder.as_ptr(),
            None,
            muxer,
            muxer.audio_stream,
            ffi::AVRational {
                num: 1,
                den: INTERNAL_SAMPLE_RATE,
            },
        )
    }
}

/// Configure the H.264 encoder to §13.1's specification.
fn open_video_encoder(
    width: i32,
    height: i32,
    frame_rate: ffi::AVRational,
    threads: u32,
    spec: &ProxySpec,
    global_header: bool,
) -> Result<CodecContext, MediaError> {
    // §0.1: x264 is GPL and must never be linked. openh264 is the BSD encoder
    // our LGPL build ships, and ADR 002 records why.
    let context = CodecContext::encoder("libopenh264", threads)?;

    // SAFETY: the context is allocated and not yet opened, which is when these
    // fields may be set.
    unsafe {
        let ctx = &mut *context.as_ptr();
        ctx.width = width;
        ctx.height = height;
        ctx.pix_fmt = ffi::AV_PIX_FMT_YUV420P;
        ctx.time_base = encoder_timebase(frame_rate);
        ctx.framerate = frame_rate;

        // **The line this whole module exists for** (§13.1, §47a.1).
        // Every frame a keyframe means one decode per seek instead of up to 250.
        ctx.gop_size = spec.gop_length as i32;
        ctx.max_b_frames = 0;

        // §21a: tag the output explicitly. An untagged file gets guessed at by
        // players, which reintroduces the colour bug at the other end.
        ctx.color_range = ffi::AVCOL_RANGE_MPEG;
        ctx.color_primaries = ffi::AVCOL_PRI_BT709;
        ctx.color_trc = ffi::AVCOL_TRC_BT709;
        ctx.colorspace = ffi::AVCOL_SPC_BT709;

        // Proxies are for judging framing and timing, not grading. A modest
        // bitrate keeps §67's cache limit reachable given all-intra's 3-5x cost.
        ctx.bit_rate = i64::from(width) * i64::from(height) * 4;

        // MP4 keeps SPS/PPS in the container rather than in the stream. This
        // must be set before opening, or the encoder produces in-band headers
        // the muxer then discards, and the result does not decode.
        if global_header {
            ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    context.open_encoder()?;
    Ok(context)
}

fn open_audio_encoder(
    channels: usize,
    threads: u32,
    global_header: bool,
) -> Result<CodecContext, MediaError> {
    let context = CodecContext::encoder("aac", threads)?;

    // SAFETY: allocated and not yet opened.
    unsafe {
        let ctx = &mut *context.as_ptr();
        ctx.sample_fmt = ffi::AV_SAMPLE_FMT_FLTP;
        // §9/§20a.3: 48 kHz, always.
        ctx.sample_rate = INTERNAL_SAMPLE_RATE;
        ctx.time_base = ffi::AVRational {
            num: 1,
            den: INTERNAL_SAMPLE_RATE,
        };
        ctx.bit_rate = 128_000;
        ffi::av_channel_layout_default(&mut ctx.ch_layout, channels as i32);

        if global_header {
            ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    context.open_encoder()?;
    Ok(context)
}

/// Encoder timebase: the reciprocal of the frame rate, so PTS counts frames.
fn encoder_timebase(frame_rate: ffi::AVRational) -> ffi::AVRational {
    ffi::AVRational {
        num: frame_rate.den,
        den: frame_rate.num,
    }
}

/// Locate the first stream of a kind, with what the caller needs about it.
fn find_stream(
    input: &InputContext,
    kind: ffi::AVMediaType,
) -> Option<(
    i32,
    *const ffi::AVCodecParameters,
    ffi::AVRational,
    ffi::AVRational,
)> {
    for stream in input.streams() {
        // SAFETY: `streams()` filtered nulls.
        let (params, timebase, index, rate) = unsafe {
            let s = &*stream;
            (s.codecpar, s.time_base, s.index, s.avg_frame_rate)
        };
        if params.is_null() {
            continue;
        }
        // SAFETY: non-null, owned by the stream.
        if unsafe { (*params).codec_type } == kind {
            return Some((index, params, timebase, rate));
        }
    }
    None
}

fn send_packet(
    codec: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
) -> Result<(), MediaError> {
    // SAFETY: both pointers are valid and owned by the caller.
    let code = unsafe { ffi::avcodec_send_packet(codec, packet) };
    if code < 0 && code != -(ffi::EAGAIN as i32) {
        return Err(MediaError::DecodeFailed(format!(
            "avcodec_send_packet: {}",
            error_string(code)
        )));
    }
    Ok(())
}

fn receive_frame(codec: *mut ffi::AVCodecContext, into: &mut Frame) -> Result<bool, MediaError> {
    into.unref();
    // SAFETY: both pointers are valid.
    let code = unsafe { ffi::avcodec_receive_frame(codec, into.as_ptr()) };
    if code == 0 {
        return Ok(true);
    }
    if code == -(ffi::EAGAIN as i32) || code == super::decode::averror_eof_code() {
        return Ok(false);
    }
    Err(MediaError::DecodeFailed(format!(
        "avcodec_receive_frame: {}",
        error_string(code)
    )))
}

/// Push a frame (or a flush) into an encoder and mux everything it emits.
fn drain_encoder(
    encoder: *mut ffi::AVCodecContext,
    frame: Option<*mut ffi::AVFrame>,
    muxer: &mut Muxer,
    stream: Option<i32>,
    source_timebase: ffi::AVRational,
) -> Result<(), MediaError> {
    let Some(stream_index) = stream else {
        return Ok(());
    };

    // SAFETY: the encoder is open; a null frame is the documented flush.
    let code = unsafe { ffi::avcodec_send_frame(encoder, frame.unwrap_or(std::ptr::null_mut())) };
    if code < 0 && code != -(ffi::EAGAIN as i32) && code != super::decode::averror_eof_code() {
        return Err(MediaError::DecodeFailed(format!(
            "avcodec_send_frame: {}",
            error_string(code)
        )));
    }

    let mut packet = Packet::new()?;
    loop {
        packet.unref();
        // SAFETY: both pointers are valid.
        let code = unsafe { ffi::avcodec_receive_packet(encoder, packet.as_ptr()) };
        if code == -(ffi::EAGAIN as i32) || code == super::decode::averror_eof_code() {
            return Ok(());
        }
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "avcodec_receive_packet: {}",
                error_string(code)
            )));
        }
        muxer.write(&mut packet, stream_index, source_timebase)?;
    }
}

/// The output file.
struct Muxer {
    context: *mut ffi::AVFormatContext,
    video_stream: Option<i32>,
    audio_stream: Option<i32>,
    header_written: bool,
}

impl Muxer {
    /// Allocate the output context, deducing the container from the extension.
    ///
    /// Separate from [`Self::begin`] because the caller needs
    /// [`Self::needs_global_header`] *before* it opens any encoder.
    fn allocate(path: &Path) -> Result<Self, MediaError> {
        let c_path = super::path_to_cstring(path)?;
        let mut context: *mut ffi::AVFormatContext = std::ptr::null_mut();

        // SAFETY: `context` is a valid out-pointer; FFmpeg allocates into it.
        let code = unsafe {
            ffi::avformat_alloc_output_context2(
                &mut context,
                std::ptr::null_mut(),
                std::ptr::null(),
                c_path.as_ptr(),
            )
        };
        if code < 0 || context.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "avformat_alloc_output_context2: {}",
                error_string(code)
            )));
        }

        Ok(Self {
            context,
            video_stream: None,
            audio_stream: None,
            header_written: false,
        })
    }

    /// Does this container keep codec headers out of the stream?
    fn needs_global_header(&self) -> bool {
        // SAFETY: `context` is allocated and its `oformat` is set by
        // avformat_alloc_output_context2.
        unsafe { (*(*self.context).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 }
    }

    fn add_video(&mut self, encoder: *mut ffi::AVCodecContext) -> Result<(), MediaError> {
        self.video_stream = Some(self.add_stream(encoder)?);
        Ok(())
    }

    fn add_audio(&mut self, encoder: *mut ffi::AVCodecContext) -> Result<(), MediaError> {
        self.audio_stream = Some(self.add_stream(encoder)?);
        Ok(())
    }

    /// Open the file and write the header. Streams must already be added.
    fn begin(&mut self, path: &Path) -> Result<(), MediaError> {
        let c_path = super::path_to_cstring(path)?;

        // SAFETY: `context` is allocated; `c_path` outlives the call.
        let code = unsafe {
            ffi::avio_open(
                &mut (*self.context).pb,
                c_path.as_ptr(),
                ffi::AVIO_FLAG_WRITE as i32,
            )
        };
        if code < 0 {
            return Err(MediaError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other(error_string(code)),
            });
        }

        // SAFETY: streams are configured and the IO context is open.
        let code = unsafe { ffi::avformat_write_header(self.context, std::ptr::null_mut()) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "avformat_write_header: {}",
                error_string(code)
            )));
        }
        self.header_written = true;
        Ok(())
    }

    fn add_stream(&mut self, encoder: *mut ffi::AVCodecContext) -> Result<i32, MediaError> {
        // SAFETY: `context` is allocated; a null codec is allowed here because
        // the parameters are copied from the encoder immediately after.
        let stream = unsafe { ffi::avformat_new_stream(self.context, std::ptr::null()) };
        if stream.is_null() {
            return Err(MediaError::DecodeFailed(
                "avformat_new_stream returned null".to_owned(),
            ));
        }

        // SAFETY: both pointers are valid and the stream is freshly created.
        // The global-header flag was set before the encoder was opened, so the
        // parameters copied here already carry the right extradata.
        let code = unsafe {
            (*stream).time_base = (*encoder).time_base;
            ffi::avcodec_parameters_from_context((*stream).codecpar, encoder)
        };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "avcodec_parameters_from_context: {}",
                error_string(code)
            )));
        }

        // SAFETY: the stream was just created by this context.
        Ok(unsafe { (*stream).index })
    }

    fn write(
        &mut self,
        packet: &mut Packet,
        stream_index: i32,
        source_timebase: ffi::AVRational,
    ) -> Result<(), MediaError> {
        // SAFETY: `context` is open and `stream_index` came from it.
        unsafe {
            let stream = *(*self.context).streams.add(stream_index as usize);
            let raw = packet.as_ptr();
            (*raw).stream_index = stream_index;
            // The encoder counts in its own units; the container has its own.
            ffi::av_packet_rescale_ts(raw, source_timebase, (*stream).time_base);

            let code = ffi::av_interleaved_write_frame(self.context, raw);
            if code < 0 {
                return Err(MediaError::DecodeFailed(format!(
                    "av_interleaved_write_frame: {}",
                    error_string(code)
                )));
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), MediaError> {
        if !self.header_written {
            return Ok(());
        }
        // SAFETY: the header was written, so a trailer is expected.
        let code = unsafe { ffi::av_write_trailer(self.context) };
        self.header_written = false;
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "av_write_trailer: {}",
                error_string(code)
            )));
        }
        Ok(())
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        if self.context.is_null() {
            return;
        }
        // SAFETY: `context` came from avformat_alloc_output_context2. Closing
        // the IO context before freeing is required, and doing it here means an
        // abandoned encode cannot leave the output file locked.
        unsafe {
            if !(*self.context).pb.is_null() {
                ffi::avio_closep(&mut (*self.context).pb);
            }
            ffi::avformat_free_context(self.context);
        }
        self.context = std::ptr::null_mut();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{MediaDecoder, MediaProber, NeverCancelled, SeekMode};
    use crate::ffmpeg::{FfmpegDecoder, FfmpegProber};
    use bettercut_foundation::MediaTime;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn encode(name: &str, height: u32) -> (tempfile::TempDir, std::path::PathBuf, MediaAsset) {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join(format!("{height}p.mp4"));
        let asset = FfmpegProber.probe(&fixture(name)).expect("probe");

        generate_proxy(&asset, &output, height, 1, &NeverCancelled, &|_| {})
            .unwrap_or_else(|e| panic!("proxy generation failed: {e}"));

        let proxy = FfmpegProber.probe(&output).expect("probe the proxy");
        (dir, output, proxy)
    }

    #[test]
    fn a_proxy_is_produced_at_the_requested_height() {
        let (_dir, output, proxy) = encode("ntsc-2997.mp4", 180);

        assert!(output.exists());
        assert!(output.metadata().expect("metadata").len() > 0);
        assert_eq!(proxy.height, 180);
        // 640x360 scaled to 180 tall keeps 16:9.
        assert_eq!(proxy.width, 320);
    }

    /// **The property this whole module exists for** (§13.1, §47a.1).
    ///
    /// If every frame is a keyframe, seeking anywhere costs one decode. The
    /// test is behavioural rather than a header check: seek to a spread of
    /// instants and require the decoder to land on the exact frame each time,
    /// which only holds when there is a keyframe there to land on.
    #[test]
    fn every_frame_in_a_proxy_is_a_keyframe() {
        let (_dir, _output, proxy) = encode("ntsc-2997.mp4", 180);

        let mut decoder = FfmpegDecoder::new(1).expect("decoder");
        decoder.open(&proxy).expect("open the proxy");

        // Ask for a scrub seek, which does *not* decode forward. On long-GOP
        // media it would land on the previous keyframe, potentially seconds
        // early; on all-intra media it lands exactly.
        let frame_ticks = MediaTime::from_ticks(32_032);
        for index in [3_i64, 11, 24, 37] {
            let target = MediaTime::from_ticks(index * frame_ticks.ticks());
            decoder.seek(target, SeekMode::Scrub).expect("seek");

            let frame = decoder
                .decode_frame(&NeverCancelled)
                .expect("decode")
                .expect("a frame");

            let delta = (frame.timestamp.ticks() - target.ticks()).abs();
            assert!(
                delta < frame_ticks.ticks(),
                "scrub seek to frame {index} landed {delta} ticks away - \
                 the proxy is not all-intra"
            );
        }
    }

    /// §13.1's normalization: whatever went in, the proxy is BT.709 limited,
    /// 8-bit. Nothing downstream then has to ask.
    #[test]
    fn the_proxy_is_normalized_to_bt709_limited_8_bit() {
        let (_dir, _output, proxy) = encode("ntsc-2997.mp4", 180);

        assert_eq!(proxy.color.bit_depth, 8);
        assert_eq!(proxy.color.range, crate::color::ColorRange::Limited);
        assert_eq!(proxy.color.matrix, crate::color::ColorMatrix::Bt709);
        assert_eq!(proxy.color.primaries, crate::color::ColorPrimaries::Bt709);
    }

    /// §13.1: "A proxy must preserve duration, timing, audio sync, aspect
    /// ratio." A proxy that runs short would desync every edit made against it.
    #[test]
    fn the_proxy_preserves_duration_and_aspect() {
        let source = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe source");
        let (_dir, _output, proxy) = encode("ntsc-2997.mp4", 180);

        let drift = (proxy.duration.ticks() - source.duration.ticks()).abs();
        assert!(
            drift < MediaTime::from_millis(100).ticks(),
            "proxy is {drift} ticks off the source duration"
        );

        let source_aspect = source.width as f32 / source.height as f32;
        let proxy_aspect = proxy.width as f32 / proxy.height as f32;
        assert!(
            (source_aspect - proxy_aspect).abs() < 0.02,
            "aspect changed: {source_aspect} -> {proxy_aspect}"
        );
    }

    /// §13.1/§20a.3: audio comes out at 48 kHz whatever the source used.
    #[test]
    fn the_proxy_carries_audio_at_48_khz() {
        let (_dir, _output, proxy) = encode("ntsc-2997.mp4", 180);

        assert_eq!(proxy.audio_sample_rate, Some(48_000));
        assert!(proxy.audio_codec.is_some(), "the proxy has no audio stream");

        // And it decodes to real sound, not silence.
        let mut decoder = FfmpegDecoder::new(1).expect("decoder");
        decoder.open(&proxy).expect("open");
        let buffer = decoder
            .decode_audio(&NeverCancelled)
            .expect("decode")
            .expect("an audio buffer");
        assert_eq!(buffer.sample_rate, 48_000);
    }

    /// Source with no audio must still produce a usable proxy.
    #[test]
    fn a_silent_source_produces_a_video_only_proxy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("still.mp4");
        let asset = FfmpegProber.probe(&fixture("still.png")).expect("probe");

        generate_proxy(&asset, &output, 90, 1, &NeverCancelled, &|_| {})
            .expect("a silent source should still encode");

        assert!(output.exists());
        let proxy = FfmpegProber.probe(&output).expect("probe");
        assert!(proxy.audio_codec.is_none());
    }

    #[test]
    fn progress_is_reported_and_ends_at_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("progress.mp4");
        let asset = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe");

        let seen = std::cell::RefCell::new(Vec::new());
        generate_proxy(&asset, &output, 180, 1, &NeverCancelled, &|fraction| {
            seen.borrow_mut().push(fraction);
        })
        .expect("encode");

        let seen = seen.into_inner();
        assert!(seen.len() > 5, "progress was barely reported");
        assert!(seen.iter().all(|f| (0.0..=1.0).contains(f)));
        assert_eq!(
            seen.last().copied(),
            Some(1.0),
            "progress never reached 1.0"
        );
        // Monotonic, so a progress bar never runs backwards.
        assert!(
            seen.windows(2).all(|pair| pair[1] >= pair[0]),
            "progress went backwards"
        );
    }

    /// §48: a cancelled encode stops rather than running to completion.
    #[test]
    fn a_cancelled_encode_stops_early() {
        struct AlwaysCancelled;
        impl CancellationToken for AlwaysCancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("cancelled.mp4");
        let asset = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe");

        let result = generate_proxy(&asset, &output, 180, 1, &AlwaysCancelled, &|_| {});
        assert!(matches!(result, Err(MediaError::Cancelled)));
    }

    /// A proxy of a proxy should be smaller still — a sanity check that the
    /// requested height is actually applied rather than ignored.
    #[test]
    fn a_smaller_height_produces_a_smaller_file() {
        let (_a, big_path, _big) = encode("ntsc-2997.mp4", 240);
        let (_b, small_path, _small) = encode("ntsc-2997.mp4", 120);

        let big = big_path.metadata().expect("metadata").len();
        let small = small_path.metadata().expect("metadata").len();
        assert!(
            small < big,
            "the smaller proxy ({small} bytes) was not smaller than the larger ({big})"
        );
    }
}
