//! A minimal libavfilter graph, for tone-mapping HDR to SDR (§21a.1, §21a.2).
//!
//! swscale converts pixel formats and colour *matrices*, but it cannot convert
//! a transfer function. PQ and HLG encode light on a curve nothing else in this
//! pipeline understands, so a PQ frame interpreted as BT.709 comes out around
//! half as bright — measured on the test fixture: mean luma 122 against 227
//! once tone-mapped.
//!
//! Doing the conversion by hand would mean a transcendental curve per channel
//! per pixel on the CPU. `zscale` (libzimg) plus `tonemap` already do it, are
//! in the pinned LGPL build, and are what every other tool uses.
//!
//! This wrapper is deliberately small: one input, one output, one linear chain.
//! Anything more elaborate belongs in the renderer (§46), not here.

use rusty_ffmpeg::ffi;

use super::raii::Frame;
use super::{error_string, path_to_cstring};
use crate::error::MediaError;

/// Whether a stream's transfer function is HDR (§21a.1).
pub(crate) fn is_hdr_transfer(trc: ffi::AVColorTransferCharacteristic) -> bool {
    matches!(trc, ffi::AVCOL_TRC_SMPTE2084 | ffi::AVCOL_TRC_ARIB_STD_B67)
}

/// The chain that turns an HDR stream into BT.709 SDR, limited-range yuv420p,
/// at `width`×`height` (§21a.1).
///
/// **One description, two users.** The proxy encoder runs it to build a proxy,
/// and the decoder runs it on originals — which is the path export takes, since
/// §14 exports from the original media. They used to disagree: only the proxy
/// was tone-mapped, so an HDR clip looked right while editing and came out
/// around half as bright in the export. Built in one place, the two cannot
/// drift, and an original decodes to the same picture its proxy does.
///
/// The input side is stated explicitly rather than left to the frame's own
/// tags. `zscale` reads those tags when they are there and silently assumes
/// SDR when they are not — which produces exactly the untone-mapped picture
/// this exists to prevent. The stream was probed, so its colour is known.
///
/// `tonemap` is left on its default `desat`. Setting `desat=0` looks harmless —
/// "do not desaturate highlights" — and roughly halves the result: measured 116
/// against 227 on the fixture. The default is what every reference chain uses.
pub(crate) fn hdr_to_sdr_spec(
    trc: ffi::AVColorTransferCharacteristic,
    primaries: ffi::AVColorPrimaries,
    space: ffi::AVColorSpace,
    range: ffi::AVColorRange,
    width: i32,
    height: i32,
) -> Option<String> {
    if !is_hdr_transfer(trc) {
        return None;
    }
    let tin = match trc {
        ffi::AVCOL_TRC_SMPTE2084 => "smpte2084",
        _ => "arib-std-b67",
    };
    let pin = match primaries {
        ffi::AVCOL_PRI_BT2020 => "bt2020",
        ffi::AVCOL_PRI_BT709 => "bt709",
        _ => "bt2020", // HDR is overwhelmingly BT.2020
    };
    let min = match space {
        ffi::AVCOL_SPC_BT2020_NCL => "bt2020nc",
        ffi::AVCOL_SPC_BT709 => "bt709",
        _ => "bt2020nc",
    };
    let rin = if range == ffi::AVCOL_RANGE_JPEG {
        "full"
    } else {
        "limited"
    };
    Some(format!(
        "zscale=tin={tin}:pin={pin}:min={min}:rin={rin}:t=linear:npl=100,\
         tonemap=hable,\
         zscale=w={width}:h={height}:p=bt709:t=bt709:m=bt709:r=tv,\
         format=yuv420p"
    ))
}

/// An owned filter graph with a `buffer` source and a `buffersink`.
pub(crate) struct FilterGraph {
    graph: *mut ffi::AVFilterGraph,
    source: *mut ffi::AVFilterContext,
    sink: *mut ffi::AVFilterContext,
}

impl FilterGraph {
    /// Build a graph from a filter description, e.g. `"format=yuv420p"`.
    ///
    /// `spec` is spliced between the built-in `[in]` and `[out]` pads, so it
    /// must be a plain chain with no labels of its own.
    pub(crate) fn new(
        spec: &str,
        width: i32,
        height: i32,
        pix_fmt: i32,
        timebase: ffi::AVRational,
        sample_aspect: ffi::AVRational,
    ) -> Result<Self, MediaError> {
        // SAFETY: allocation; null is the documented failure.
        let graph = unsafe { ffi::avfilter_graph_alloc() };
        if graph.is_null() {
            return Err(MediaError::DecodeFailed(
                "could not allocate a filter graph".to_owned(),
            ));
        }

        let mut this = Self {
            graph,
            source: std::ptr::null_mut(),
            sink: std::ptr::null_mut(),
        };

        let args = format!(
            "video_size={width}x{height}:pix_fmt={pix_fmt}:time_base={}/{}:pixel_aspect={}/{}",
            timebase.num,
            timebase.den.max(1),
            sample_aspect.num.max(1),
            sample_aspect.den.max(1),
        );

        this.create("buffer", "in", Some(&args), true)?;
        this.create("buffersink", "out", None, false)?;
        this.parse(spec)?;

        // SAFETY: both endpoints exist and the chain has been parsed.
        let code = unsafe { ffi::avfilter_graph_config(this.graph, std::ptr::null_mut()) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "filter graph '{spec}' is invalid: {}",
                error_string(code)
            )));
        }

        Ok(this)
    }

    /// Create one endpoint filter and record its context.
    fn create(
        &mut self,
        filter: &str,
        name: &str,
        args: Option<&str>,
        is_source: bool,
    ) -> Result<(), MediaError> {
        let c_filter = path_to_cstring(std::path::Path::new(filter))?;
        let c_name = path_to_cstring(std::path::Path::new(name))?;
        let c_args = args
            .map(|a| path_to_cstring(std::path::Path::new(a)))
            .transpose()?;

        // SAFETY: the name is a valid NUL-terminated string; null means the
        // filter is not compiled into this build.
        let definition = unsafe { ffi::avfilter_get_by_name(c_filter.as_ptr()) };
        if definition.is_null() {
            return Err(MediaError::DecodeFailed(format!(
                "filter '{filter}' is not available in this FFmpeg build"
            )));
        }

        let mut context: *mut ffi::AVFilterContext = std::ptr::null_mut();
        // SAFETY: `graph` is valid; the call fills `context` or returns < 0.
        let code = unsafe {
            ffi::avfilter_graph_create_filter(
                &mut context,
                definition,
                c_name.as_ptr(),
                c_args.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
                std::ptr::null_mut(),
                self.graph,
            )
        };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "could not create filter '{filter}': {}",
                error_string(code)
            )));
        }

        if is_source {
            self.source = context;
        } else {
            self.sink = context;
        }
        Ok(())
    }

    /// Splice `spec` between the source and the sink.
    fn parse(&mut self, spec: &str) -> Result<(), MediaError> {
        // SAFETY: allocations; null on failure.
        let (inputs, outputs) =
            unsafe { (ffi::avfilter_inout_alloc(), ffi::avfilter_inout_alloc()) };
        if inputs.is_null() || outputs.is_null() {
            // SAFETY: freeing possibly-null pointers is defined for these.
            unsafe {
                ffi::avfilter_inout_free(&mut { inputs });
                ffi::avfilter_inout_free(&mut { outputs });
            }
            return Err(MediaError::DecodeFailed(
                "could not allocate filter endpoints".to_owned(),
            ));
        }

        let in_name = path_to_cstring(std::path::Path::new("in"))?;
        let out_name = path_to_cstring(std::path::Path::new("out"))?;

        // SAFETY: both are freshly allocated and exclusively owned here. The
        // names are duplicated with av_strdup because avfilter frees them.
        //
        // The naming reads backwards on purpose: `outputs` describes what feeds
        // *into* the parsed chain (our buffer source), and `inputs` describes
        // what the chain feeds (our sink). That is libavfilter's convention.
        unsafe {
            (*outputs).name = ffi::av_strdup(in_name.as_ptr());
            (*outputs).filter_ctx = self.source;
            (*outputs).pad_idx = 0;
            (*outputs).next = std::ptr::null_mut();

            (*inputs).name = ffi::av_strdup(out_name.as_ptr());
            (*inputs).filter_ctx = self.sink;
            (*inputs).pad_idx = 0;
            (*inputs).next = std::ptr::null_mut();
        }

        let c_spec = path_to_cstring(std::path::Path::new(spec))?;
        let mut inputs = inputs;
        let mut outputs = outputs;

        // SAFETY: parse_ptr takes ownership of both lists and frees them.
        let code = unsafe {
            ffi::avfilter_graph_parse_ptr(
                self.graph,
                c_spec.as_ptr(),
                &mut inputs,
                &mut outputs,
                std::ptr::null_mut(),
            )
        };

        // SAFETY: whatever parse_ptr left behind is ours to free.
        unsafe {
            ffi::avfilter_inout_free(&mut inputs);
            ffi::avfilter_inout_free(&mut outputs);
        }

        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "could not parse filter chain '{spec}': {}",
                error_string(code)
            )));
        }
        Ok(())
    }

    /// Push a decoded frame in.
    pub(crate) fn push(&mut self, frame: &Frame) -> Result<(), MediaError> {
        // SAFETY: both pointers are valid; the filter takes a reference.
        let code = unsafe { ffi::av_buffersrc_add_frame(self.source, frame.as_ptr()) };
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "av_buffersrc_add_frame: {}",
                error_string(code)
            )));
        }
        Ok(())
    }

    /// Pull a filtered frame out, or `false` when the graph needs more input.
    pub(crate) fn pull(&mut self, out: &mut Frame) -> Result<bool, MediaError> {
        out.unref();
        // SAFETY: both pointers are valid; the frame is filled or an error
        // code explains why not.
        let code = unsafe { ffi::av_buffersink_get_frame(self.sink, out.as_ptr()) };
        if code == -(ffi::EAGAIN as i32) || code == super::decode::averror_eof_code() {
            return Ok(false);
        }
        if code < 0 {
            return Err(MediaError::DecodeFailed(format!(
                "av_buffersink_get_frame: {}",
                error_string(code)
            )));
        }
        Ok(true)
    }
}

impl Drop for FilterGraph {
    fn drop(&mut self) {
        // SAFETY: freeing the graph frees every filter context in it, so the
        // endpoint pointers must not be touched afterwards.
        unsafe { ffi::avfilter_graph_free(&mut self.graph) };
    }
}

// SAFETY: a graph is owned by one thread at a time; nothing here is shared.
unsafe impl Send for FilterGraph {}
