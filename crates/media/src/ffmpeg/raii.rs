//! Owning wrappers around FFmpeg's C allocations.
//!
//! Every one of these exists for the same reason: FFmpeg's free functions must
//! run on **every** path out of a function, including early returns and
//! panics. Doing that by hand is how decoders leak, and a leaked decoder holds
//! the user's file open — on Windows that means they cannot move or delete
//! their own footage until the app closes.
//!
//! Each type owns exactly one allocation and frees it in `Drop`.

use rusty_ffmpeg::ffi;

use crate::error::MediaError;

use super::error_string;

/// An `AVPacket` — one compressed chunk read from the container.
pub(crate) struct Packet {
    inner: *mut ffi::AVPacket,
}

impl Packet {
    pub(crate) fn new() -> Result<Self, MediaError> {
        // SAFETY: av_packet_alloc takes no arguments and returns null on OOM.
        let inner = unsafe { ffi::av_packet_alloc() };
        if inner.is_null() {
            return Err(MediaError::DecodeFailed(
                "could not allocate a packet".to_owned(),
            ));
        }
        Ok(Self { inner })
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::AVPacket {
        self.inner
    }

    pub(crate) fn stream_index(&self) -> i32 {
        // SAFETY: `inner` is non-null for the lifetime of `self`.
        unsafe { (*self.inner).stream_index }
    }

    /// Release the packet's payload while keeping the allocation for reuse.
    /// Called after every packet, or the decoder leaks one buffer per frame.
    pub(crate) fn unref(&mut self) {
        // SAFETY: `inner` is non-null; av_packet_unref accepts a used or
        // already-unreffed packet.
        unsafe { ffi::av_packet_unref(self.inner) };
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: `inner` came from av_packet_alloc and is freed once.
        unsafe { ffi::av_packet_free(&mut self.inner) };
    }
}

/// An `AVFrame` — one decoded picture or block of samples.
pub(crate) struct Frame {
    inner: *mut ffi::AVFrame,
}

impl Frame {
    pub(crate) fn new() -> Result<Self, MediaError> {
        // SAFETY: av_frame_alloc takes no arguments and returns null on OOM.
        let inner = unsafe { ffi::av_frame_alloc() };
        if inner.is_null() {
            return Err(MediaError::DecodeFailed(
                "could not allocate a frame".to_owned(),
            ));
        }
        Ok(Self { inner })
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::AVFrame {
        self.inner
    }

    /// A frame with its own picture buffer, for handing to an encoder.
    pub(crate) fn video(width: i32, height: i32, format: i32) -> Result<Self, MediaError> {
        let frame = Self::new()?;
        // SAFETY: freshly allocated and not yet given a buffer, which is when
        // these fields may be set.
        unsafe {
            let raw = &mut *frame.inner;
            raw.width = width;
            raw.height = height;
            raw.format = format;
        }
        // SAFETY: dimensions and format are set; 32-byte alignment suits every
        // SIMD path FFmpeg might take.
        let code = unsafe { ffi::av_frame_get_buffer(frame.inner, 32) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "av_frame_get_buffer(video): {}",
                error_string(code)
            )));
        }
        Ok(frame)
    }

    /// A planar `f32` audio frame with its own sample buffer.
    pub(crate) fn audio(
        samples: usize,
        channels: usize,
        sample_rate: i32,
    ) -> Result<Self, MediaError> {
        let frame = Self::new()?;
        // SAFETY: freshly allocated, no buffer yet.
        unsafe {
            let raw = &mut *frame.inner;
            raw.nb_samples = samples as i32;
            raw.format = ffi::AV_SAMPLE_FMT_FLTP;
            raw.sample_rate = sample_rate;
            ffi::av_channel_layout_default(&mut raw.ch_layout, channels as i32);
        }
        // SAFETY: layout and size are set.
        let code = unsafe { ffi::av_frame_get_buffer(frame.inner, 0) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "av_frame_get_buffer(audio): {}",
                error_string(code)
            )));
        }
        Ok(frame)
    }

    /// SAFETY: the caller must not hold the reference past the next `unref`.
    pub(crate) fn as_ref(&self) -> &ffi::AVFrame {
        // SAFETY: `inner` is non-null for the lifetime of `self`.
        unsafe { &*self.inner }
    }

    pub(crate) fn unref(&mut self) {
        // SAFETY: `inner` is non-null; av_frame_unref accepts any state.
        unsafe { ffi::av_frame_unref(self.inner) };
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: `inner` came from av_frame_alloc and is freed once.
        unsafe { ffi::av_frame_free(&mut self.inner) };
    }
}

/// An open decoder for one stream.
pub(crate) struct CodecContext {
    inner: *mut ffi::AVCodecContext,
}

impl CodecContext {
    /// Open a decoder for `params`, capped to `threads` internal threads.
    ///
    /// **The thread cap is not optional** (§15.1, §74). FFmpeg defaults to
    /// using every core; one uncapped decode job saturates a 4-core machine no
    /// matter how few jobs the scheduler runs, and playback is what suffers.
    pub(crate) fn open(
        params: *const ffi::AVCodecParameters,
        threads: u32,
    ) -> Result<Self, MediaError> {
        if params.is_null() {
            return Err(MediaError::DecodeFailed(
                "stream has no codec parameters".to_owned(),
            ));
        }

        // SAFETY: `params` is non-null and owned by the caller's stream.
        let codec_id = unsafe { (*params).codec_id };

        // SAFETY: avcodec_find_decoder accepts any id and returns null when
        // there is no decoder for it.
        let codec = unsafe { ffi::avcodec_find_decoder(codec_id) };
        if codec.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "no decoder for codec id {codec_id}"
            )));
        }

        // SAFETY: `codec` is non-null; returns null on OOM.
        let inner = unsafe { ffi::avcodec_alloc_context3(codec) };
        if inner.is_null() {
            return Err(MediaError::DecodeFailed(
                "could not allocate a codec context".to_owned(),
            ));
        }

        // From here on `this` owns `inner`, so every error path frees it.
        let this = Self { inner };

        // SAFETY: both pointers are non-null and the context is freshly
        // allocated, which is what avcodec_parameters_to_context expects.
        let code = unsafe { ffi::avcodec_parameters_to_context(this.inner, params) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "avcodec_parameters_to_context: {}",
                error_string(code)
            )));
        }

        // SAFETY: `inner` is non-null and not yet opened.
        unsafe {
            (*this.inner).thread_count = threads.max(1) as i32;
        }

        // SAFETY: context and codec are non-null; options are optional.
        let code = unsafe { ffi::avcodec_open2(this.inner, codec, std::ptr::null_mut()) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "avcodec_open2: {}",
                error_string(code)
            )));
        }

        Ok(this)
    }

    /// Allocate an *encoder* context, to be configured then opened.
    ///
    /// Named rather than looked up by id, because §0.1 constrains which
    /// encoders may be used at all: `libopenh264` (BSD) rather than `libx264`
    /// (GPL). Asking for a codec id would let FFmpeg pick whichever H.264
    /// encoder happened to be compiled in, which is exactly the decision §0.1
    /// says we must make deliberately.
    pub(crate) fn encoder(name: &str, threads: u32) -> Result<Self, MediaError> {
        let c_name = std::ffi::CString::new(name)
            .map_err(|_| MediaError::DecodeFailed(format!("bad encoder name {name}")))?;

        // SAFETY: `c_name` is a valid NUL-terminated string; the call returns
        // null when there is no such encoder.
        let codec = unsafe { ffi::avcodec_find_encoder_by_name(c_name.as_ptr()) };
        if codec.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "encoder '{name}' is not available in this FFmpeg build"
            )));
        }

        // SAFETY: `codec` is non-null; returns null on OOM.
        let inner = unsafe { ffi::avcodec_alloc_context3(codec) };
        if inner.is_null() {
            return Err(MediaError::DecodeFailed(
                "could not allocate an encoder context".to_owned(),
            ));
        }

        let this = Self { inner };
        // SAFETY: freshly allocated and not yet opened.
        unsafe {
            (*this.inner).thread_count = threads.max(1) as i32;
        }
        Ok(this)
    }

    /// Open a context previously created by [`Self::encoder`].
    pub(crate) fn open_encoder(&self) -> Result<(), MediaError> {
        // SAFETY: `inner` was allocated from a codec and configured but not
        // opened. Passing null re-uses the codec it was allocated with.
        let code =
            unsafe { ffi::avcodec_open2(self.inner, std::ptr::null(), std::ptr::null_mut()) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "avcodec_open2(encoder): {}",
                error_string(code)
            )));
        }
        Ok(())
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::AVCodecContext {
        self.inner
    }

    /// Drop everything buffered inside the decoder.
    ///
    /// Required after a seek: without it the decoder emits frames from before
    /// the seek, which look like the picture jumping backwards (§47a).
    pub(crate) fn flush(&mut self) {
        // SAFETY: `inner` is an opened context.
        unsafe { ffi::avcodec_flush_buffers(self.inner) };
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        // SAFETY: `inner` came from avcodec_alloc_context3 and is freed once.
        unsafe { ffi::avcodec_free_context(&mut self.inner) };
    }
}

/// A `SwsContext` — pixel format and colour conversion.
pub(crate) struct Scaler {
    inner: *mut ffi::SwsContext,
    /// Output size.
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// The size of picture it reads.
    src_width: i32,
    src_height: i32,
}

impl Scaler {
    /// Build a converter from `src_format` to RGBA at the same size.
    ///
    /// §21a.2 requires colour to be resolved exactly once, at the boundary
    /// between decode and the working space. That is what this is: swscale is
    /// told the source's matrix and range explicitly, so limited-range YUV is
    /// expanded correctly rather than being guessed at downstream.
    pub(crate) fn to_rgba(
        width: i32,
        height: i32,
        src_format: i32,
        full_range: bool,
        colorspace: i32,
    ) -> Result<Self, MediaError> {
        Self::to_rgba_sized(
            (width, height),
            (width, height),
            src_format,
            full_range,
            colorspace,
        )
    }

    /// [`Self::to_rgba`], scaling to `output` on the way — how a still larger
    /// than [`crate::MAX_STILL_EDGE`] is brought down to size.
    pub(crate) fn to_rgba_sized(
        (src_width, src_height): (i32, i32),
        (width, height): (i32, i32),
        src_format: i32,
        full_range: bool,
        colorspace: i32,
    ) -> Result<Self, MediaError> {
        // Bicubic when shrinking, for the proxy path's reason: bilinear drops
        // detail on a large downscale. Same-size conversion stays bilinear,
        // where the two are identical and bilinear is cheaper.
        let flags = if (width, height) == (src_width, src_height) {
            ffi::SWS_BILINEAR
        } else {
            ffi::SWS_BICUBIC
        };
        // SAFETY: sws_getContext validates its own arguments and returns null
        // when the conversion is unsupported.
        let inner = unsafe {
            ffi::sws_getContext(
                src_width,
                src_height,
                src_format,
                width,
                height,
                ffi::AV_PIX_FMT_RGBA,
                flags as i32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if inner.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "no conversion from pixel format {src_format} to RGBA"
            )));
        }

        let this = Self {
            inner,
            width,
            height,
            src_width,
            src_height,
        };

        // Tell swscale the source's colour space and range explicitly.
        //
        // **This is §21a.2's "range is the one that bites", one layer down.**
        // Left alone, swscale assumes BT.601 limited. Feeding it BT.709
        // limited-range H.264 — which is most camera and phone footage — then
        // shifts every colour slightly, and feeding it full-range footage
        // crushes blacks and clips whites. Neither is obvious on its own; both
        // show up the moment a user compares against another player.
        //
        // SAFETY: `inner` is a valid context and the coefficient tables are
        // FFmpeg's own statics, which are only read.
        unsafe {
            // FFmpeg's neutral values: no brightness shift, unity contrast and
            // saturation, all in 16.16 fixed point.
            const NEUTRAL: i32 = 1 << 16;
            let _ = ffi::sws_setColorspaceDetails(
                inner,
                ffi::sws_getCoefficients(colorspace),
                i32::from(full_range),
                // RGBA output is always full range.
                ffi::sws_getCoefficients(ffi::SWS_CS_DEFAULT as i32),
                1,
                0,
                NEUTRAL,
                NEUTRAL,
            );
        }

        Ok(this)
    }

    /// Build a converter to yuv420p at a (usually smaller) size.
    ///
    /// This is the proxy path (§13.1): it both rescales and normalizes colour,
    /// so a 10-bit BT.2020 source becomes 8-bit BT.709 limited in one step.
    pub(crate) fn to_yuv420p(
        src_width: i32,
        src_height: i32,
        src_format: i32,
        src_full_range: bool,
        src_colorspace: i32,
        dst_width: i32,
        dst_height: i32,
    ) -> Result<Self, MediaError> {
        Self::to_yuv(
            src_width,
            src_height,
            src_format,
            src_full_range,
            src_colorspace,
            dst_width,
            dst_height,
            ffi::AV_PIX_FMT_YUV420P,
        )
    }

    /// [`Self::to_yuv420p`] into another 4:2:0 layout — `YUVA420P` keeps an
    /// alpha plane, which sws fills from the source's.
    #[expect(
        clippy::too_many_arguments,
        reason = "sws_getContext's own arguments, one each"
    )]
    pub(crate) fn to_yuv(
        src_width: i32,
        src_height: i32,
        src_format: i32,
        src_full_range: bool,
        src_colorspace: i32,
        dst_width: i32,
        dst_height: i32,
        dst_format: i32,
    ) -> Result<Self, MediaError> {
        // Bicubic rather than bilinear: a proxy is usually a large downscale,
        // and bilinear downscaling loses detail the user needs to judge focus.
        // SAFETY: sws_getContext validates its arguments and returns null when
        // the conversion is unsupported.
        let inner = unsafe {
            ffi::sws_getContext(
                src_width,
                src_height,
                src_format,
                dst_width,
                dst_height,
                dst_format,
                ffi::SWS_BICUBIC as i32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if inner.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "no conversion from pixel format {src_format} to yuv420p"
            )));
        }

        // SAFETY: `inner` is valid; the coefficient tables are FFmpeg statics.
        unsafe {
            const NEUTRAL: i32 = 1 << 16;
            let _ = ffi::sws_setColorspaceDetails(
                inner,
                ffi::sws_getCoefficients(src_colorspace),
                i32::from(src_full_range),
                // §13.1: the proxy is always BT.709 limited.
                ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as i32),
                0,
                0,
                NEUTRAL,
                NEUTRAL,
            );
        }

        Ok(Self {
            inner,
            width: dst_width,
            height: dst_height,
            src_width,
            src_height,
        })
    }

    /// Convert into an allocated `AVFrame` rather than a byte buffer.
    pub(crate) fn convert_to_frame(
        &mut self,
        source: &Frame,
        target: &mut Frame,
    ) -> Result<(), MediaError> {
        // SAFETY: both frames are allocated; `target` was created for exactly
        // the dimensions this scaler outputs.
        let produced = unsafe {
            let src = source.as_ref();
            let dst = &*target.as_ptr();
            ffi::sws_scale(
                self.inner,
                src.data.as_ptr() as *const *const u8,
                src.linesize.as_ptr(),
                0,
                src.height,
                dst.data.as_ptr(),
                dst.linesize.as_ptr(),
            )
        };

        if produced != self.height {
            return Err(MediaError::DecodeFailed(format!(
                "sws_scale produced {produced} of {} rows",
                self.height
            )));
        }
        Ok(())
    }

    /// Convert one frame into a tightly packed RGBA buffer.
    pub(crate) fn convert(&mut self, frame: &Frame, out: &mut [u8]) -> Result<(), MediaError> {
        let stride = self.width * 4;
        let needed = (stride * self.height) as usize;
        if out.len() < needed {
            return Err(MediaError::DecodeFailed(format!(
                "output buffer is {} bytes, needs {needed}",
                out.len()
            )));
        }

        let dst_slice = [
            out.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ];
        let dst_stride = [stride, 0, 0, 0];

        let (src_width, src_height) = {
            let src = frame.as_ref();
            (src.width, src.height)
        };
        // swscale reads as many source rows as it is told and trusts the frame
        // to have them. A stream that changes size mid-file would otherwise
        // hand it a smaller picture than it was built for.
        if (src_width, src_height) != (self.src_width, self.src_height) {
            return Err(MediaError::DecodeFailed(format!(
                "frame is {src_width}x{src_height}; the converter was built for {}x{}",
                self.src_width, self.src_height
            )));
        }

        // SAFETY: `frame` holds a picture of exactly the source dimensions this
        // scaler was built for, checked above, and `out` is large enough for
        // one tightly packed RGBA image of the output size, also checked.
        let produced = unsafe {
            let src = frame.as_ref();
            ffi::sws_scale(
                self.inner,
                src.data.as_ptr() as *const *const u8,
                src.linesize.as_ptr(),
                0,
                src_height,
                dst_slice.as_ptr(),
                dst_stride.as_ptr(),
            )
        };

        if produced != self.height {
            return Err(MediaError::DecodeFailed(format!(
                "sws_scale produced {produced} of {} rows",
                self.height
            )));
        }
        Ok(())
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: `inner` came from sws_getContext and is freed once.
        unsafe { ffi::sws_freeContext(self.inner) };
    }
}

/// A `SwrContext` — audio resampling and layout conversion.
///
/// Everything internal is 48 kHz `f32` planar (§20a.3), which the §9 timebase
/// requires and the mixer assumes. This is where that becomes true.
pub(crate) struct Resampler {
    inner: *mut ffi::SwrContext,
    pub(crate) channels: usize,
}

impl Resampler {
    pub(crate) fn to_internal_format(
        src_layout: &ffi::AVChannelLayout,
        src_format: i32,
        src_rate: i32,
        channels: usize,
    ) -> Result<Self, MediaError> {
        let mut inner: *mut ffi::SwrContext = std::ptr::null_mut();

        // Preserve the source's channel count, so a mono file stays mono and a
        // 5.1 file is downmixed by FFmpeg's own matrix rather than by us.
        let mut out_layout = unsafe { std::mem::zeroed::<ffi::AVChannelLayout>() };
        // SAFETY: `out_layout` is zeroed, which is the documented precondition
        // for av_channel_layout_default.
        unsafe { ffi::av_channel_layout_default(&mut out_layout, channels as i32) };

        // SAFETY: all pointers are valid; swr_alloc_set_opts2 allocates into
        // `inner` and returns < 0 without allocating on failure.
        let code = unsafe {
            ffi::swr_alloc_set_opts2(
                &mut inner,
                &out_layout,
                ffi::AV_SAMPLE_FMT_FLTP,
                crate::ffmpeg::INTERNAL_SAMPLE_RATE,
                src_layout,
                src_format,
                src_rate,
                0,
                std::ptr::null_mut(),
            )
        };
        if code < 0 || inner.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "swr_alloc_set_opts2: {}",
                error_string(code)
            )));
        }

        let this = Self { inner, channels };

        // SAFETY: `inner` is allocated and configured.
        let code = unsafe { ffi::swr_init(this.inner) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "swr_init: {}",
                error_string(code)
            )));
        }

        Ok(this)
    }

    /// Resample one frame into per-channel planes.
    pub(crate) fn convert(&mut self, frame: &Frame) -> Result<Vec<Vec<f32>>, MediaError> {
        let src = frame.as_ref();

        // Worst case output length, including whatever swr still holds.
        // SAFETY: `inner` is initialised.
        let delay = unsafe {
            ffi::swr_get_delay(self.inner, i64::from(crate::ffmpeg::INTERNAL_SAMPLE_RATE))
        };
        let max_out = delay + i64::from(src.nb_samples) + 256;
        let max_out = max_out.max(0) as usize;

        let mut planes: Vec<Vec<f32>> = (0..self.channels).map(|_| vec![0.0; max_out]).collect();
        let mut plane_ptrs: Vec<*mut u8> = planes
            .iter_mut()
            .map(|p| p.as_mut_ptr().cast::<u8>())
            .collect();

        // SAFETY: `plane_ptrs` has one entry per output channel, each backed by
        // `max_out` f32 samples; swr_convert writes at most that many.
        let produced = unsafe {
            ffi::swr_convert(
                self.inner,
                plane_ptrs.as_mut_ptr(),
                max_out as i32,
                src.data.as_ptr() as *const *const u8,
                src.nb_samples,
            )
        };

        if produced < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "swr_convert: {}",
                error_string(produced)
            )));
        }

        for plane in &mut planes {
            plane.truncate(produced as usize);
        }
        Ok(planes)
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        // SAFETY: `inner` came from swr_alloc_set_opts2 and is freed once.
        unsafe { ffi::swr_free(&mut self.inner) };
    }
}
