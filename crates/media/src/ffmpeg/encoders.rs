//! Choosing an H.264 encoder (§0.1).
//!
//! §0.1 states the posture and this module implements it:
//!
//! ```text
//! Preferred:
//!   Windows  → Media Foundation (NVENC / QuickSync / AMF)
//!   macOS    → VideoToolbox
//!   Linux    → VAAPI
//! Fallback:
//!   openh264 (BSD)
//! ```
//!
//! ## Why the OS encoders come first, beyond speed
//!
//! They ship with the driver or the operating system. We distribute no encoder
//! at all when one is used, which keeps the patent-pool question in §0.1 point
//! 3 away from our binary, and the GPL question in point 2 away from the whole
//! product — §74's "never link x264 or any GPL component" is not a close call
//! here, x264 simply never appears.
//!
//! `libopenh264` is the fallback for machines with no hardware encoder. It is
//! BSD-licensed and already linked, because §13.1's proxies encode with it.
//!
//! ## Availability is not usability
//!
//! `avcodec_find_encoder_by_name` answers "was FFmpeg built with this?", which
//! is the wrong question. A generic Windows build contains `h264_nvenc` on a
//! machine with an AMD card, and `h264_qsv` on one with no Intel graphics.
//! Both are found. Both fail at `avcodec_open2`, and on a laptop with switchable
//! graphics they can fail only sometimes.
//!
//! So a candidate is accepted only after it has actually been **opened** at the
//! resolution and frame rate it will be asked to encode. That costs a few
//! milliseconds per candidate, once, and it is the difference between choosing
//! an encoder and discovering at the end of a long export that it never worked.

use bettercut_foundation::FrameRate;
use rusty_ffmpeg::ffi;

use crate::error::MediaError;

use super::error_string;
use super::raii::CodecContext;

/// The video codec to write (§0.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoCodec {
    /// Plays everywhere. The safe default, and the one §59 names.
    #[default]
    H264,
    /// Roughly half the bitrate for the same picture, at the cost of decode
    /// support on older devices — and, for us, of having no software fallback.
    /// See [`CANDIDATES_H265`].
    H265,
    /// Smaller again than H.265, and what YouTube prefers to be sent. Written
    /// only by recent GPUs — see [`CANDIDATES_AV1`].
    Av1,
}

impl VideoCodec {
    pub const ALL: [Self; 3] = [Self::H264, Self::H265, Self::Av1];

    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
            Self::Av1 => "AV1",
        }
    }

    /// Longer form, for the hover text that has to justify the choice.
    pub fn description(self) -> &'static str {
        match self {
            Self::H264 => "Plays on everything. Larger files.",
            Self::H265 => {
                "About half the size for the same quality. Needs a recent device \
                 to play it, and a GPU encoder to write it."
            }
            Self::Av1 => {
                "The smallest files for the quality, and YouTube's preferred \
                 upload. Needs a recent GPU to write it and a recent device to \
                 play it."
            }
        }
    }

    /// Bitrate multiplier against H.264 for the same picture.
    ///
    /// HEVC is roughly twice as efficient, so the automatic bitrate is halved
    /// rather than left at a number chosen for the older codec — otherwise
    /// picking H.265 would produce a file the same size for no reason.
    fn bitrate_scale(self) -> f64 {
        match self {
            Self::H264 => 1.0,
            Self::H265 => 0.55,
            Self::Av1 => 0.45,
        }
    }

    fn candidates(self) -> &'static [EncoderChoice] {
        match self {
            Self::H264 => CANDIDATES_H264,
            Self::H265 => CANDIDATES_H265,
            Self::Av1 => CANDIDATES_AV1,
        }
    }
}

/// How strictly the encoder must hold the requested bitrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RateControl {
    /// Spend up to the bitrate where the picture needs it, less where it does
    /// not. The right default: it puts the bits where they show.
    #[default]
    Variable,
    /// Hold the bitrate whatever the picture is doing, padding if necessary.
    /// Larger files for the same quality, and what some delivery specs and
    /// streaming ingests require.
    Constant,
}

impl RateControl {
    pub fn label(self) -> &'static str {
        match self {
            Self::Variable => "VBR (variable)",
            Self::Constant => "CBR (constant)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Variable => {
                "Spends the bitrate where the picture is complicated. Smaller \
                 files, same quality. Use this unless something requires otherwise."
            }
            Self::Constant => {
                "Holds the same bitrate throughout, padding simple shots. \
                 Required by some streaming ingests and broadcast specs."
            }
        }
    }
}

/// An encoder that was found and successfully opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderChoice {
    /// The FFmpeg encoder name, e.g. `h264_nvenc`.
    pub name: &'static str,
    /// What to call it in the interface.
    pub label: &'static str,
    pub kind: EncoderKind,
    pub codec: VideoCodec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderKind {
    /// Provided by the GPU driver or the OS. Distributed by neither us nor the
    /// user (§0.1).
    Hardware,
    /// Linked into our binary and running on the CPU. Only ever a
    /// BSD-or-similar encoder; §0.1 point 2 forbids anything GPL.
    Software,
}

impl EncoderKind {
    pub fn is_hardware(self) -> bool {
        matches!(self, Self::Hardware)
    }
}

/// H.264 candidates in preference order.
///
/// Hardware first, and within hardware the vendor-specific encoders ahead of
/// the generic Media Foundation wrapper: `h264_mf` reaches the same silicon
/// through another layer, and exposes fewer controls doing it. The software
/// encoder is last and is the only entry guaranteed to exist.
const CANDIDATES_H264: &[EncoderChoice] = &[
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_qsv",
        label: "Intel Quick Sync",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_amf",
        label: "AMD AMF",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_mf",
        label: "Windows Media Foundation",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    #[cfg(target_os = "macos")]
    EncoderChoice {
        name: "h264_videotoolbox",
        label: "VideoToolbox",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "h264_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "h264_vaapi",
        label: "VAAPI",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H264,
    },
    // §0.1's fallback, and §13.1's proxy encoder. BSD, so shipping it is
    // settled; ADR 002 records the reasoning.
    EncoderChoice {
        name: "libopenh264",
        label: "openh264 (software)",
        kind: EncoderKind::Software,
        codec: VideoCodec::H264,
    },
];

/// H.265 candidates in preference order.
///
/// **There is deliberately no software fallback here.** The obvious one is
/// x265, and §0.1 point 2 plus §74 rule it out absolutely: linking it makes the
/// whole product GPL. So H.265 is offered exactly when the machine has a GPU
/// encoder for it, and the interface disables the choice otherwise rather than
/// accepting it and failing at the last moment.
const CANDIDATES_H265: &[EncoderChoice] = &[
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "hevc_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "hevc_qsv",
        label: "Intel Quick Sync",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "hevc_amf",
        label: "AMD AMF",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "hevc_mf",
        label: "Windows Media Foundation",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
    #[cfg(target_os = "macos")]
    EncoderChoice {
        name: "hevc_videotoolbox",
        label: "VideoToolbox",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "hevc_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "hevc_vaapi",
        label: "VAAPI",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::H265,
    },
];

/// AV1 candidates in preference order.
///
/// Hardware only, like H.265: offered when the machine has an encoder for it
/// — NVIDIA RTX 40 and later, Intel Arc, AMD RX 7000 — and not otherwise.
/// Media Foundation last, as the generic route to the same silicon.
const CANDIDATES_AV1: &[EncoderChoice] = &[
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    EncoderChoice {
        name: "av1_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::Av1,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "av1_qsv",
        label: "Intel Quick Sync",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::Av1,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "av1_amf",
        label: "AMD AMF",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::Av1,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "av1_mf",
        label: "Windows Media Foundation",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::Av1,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "av1_vaapi",
        label: "VAAPI",
        kind: EncoderKind::Hardware,
        codec: VideoCodec::Av1,
    },
];

/// What one candidate turned out to be, for the report the user sees.
#[derive(Debug, Clone)]
pub struct EncoderProbe {
    pub choice: EncoderChoice,
    /// `None` when it worked; otherwise why it did not.
    pub rejected: Option<String>,
}

impl EncoderProbe {
    pub fn is_usable(&self) -> bool {
        self.rejected.is_none()
    }
}

/// The video format an encoder has to accept.
#[derive(Debug, Clone, Copy)]
pub struct EncodeTarget {
    pub width: u32,
    pub height: u32,
    /// The exact ratio §9 stores, never a rounded decimal.
    pub frame_rate: FrameRate,
    pub codec: VideoCodec,
    /// Bits per second, or `None` to derive one from the resolution and rate.
    pub bitrate: Option<i64>,
    pub rate_control: RateControl,
    /// §15.1: FFmpeg must never be allowed every core.
    pub threads: u32,
    /// MP4 keeps SPS/PPS in the container. Must be known before opening.
    pub global_header: bool,
}

/// Try every candidate and report what happened to each.
///
/// Used by the export dialog, which shows the user what their machine offers
/// rather than silently picking — the interface explains itself, and
/// "why is my export slow" has an answer here.
pub fn probe_all(target: EncodeTarget) -> Vec<EncoderProbe> {
    target
        .codec
        .candidates()
        .iter()
        .map(|choice| EncoderProbe {
            choice: *choice,
            rejected: open(*choice, target).err().map(|err| err.to_string()),
        })
        .collect()
}

/// The best encoder this machine will actually open, with it already open.
///
/// Returns the context ready to encode, because opening is the test: handing
/// back a name and opening it again would leave room for the second attempt to
/// fail differently, which on switchable graphics it can.
pub fn open_best(target: EncodeTarget) -> Result<(EncoderChoice, CodecContext), MediaError> {
    let mut refusals = Vec::new();

    // Diagnostic override. Encoders differ in ways that only surface in the
    // finished file — B-frame reordering cost a frame on NVENC and nothing on
    // openh264 — so being able to pin one down and run the export tests
    // against it is what turns "it broke somewhere" into a diagnosis.
    let only = std::env::var("BETTERCUT_FORCE_ENCODER").ok();
    for choice in target.codec.candidates() {
        if let Some(name) = &only
            && choice.name != name
        {
            continue;
        }
        match open(*choice, target) {
            Ok(context) => {
                tracing::info!(
                    encoder = choice.name,
                    hardware = choice.kind.is_hardware(),
                    "export encoder selected"
                );
                return Ok((*choice, context));
            }
            Err(err) => {
                tracing::debug!(encoder = choice.name, %err, "encoder unavailable");
                refusals.push(format!("{}: {err}", choice.name));
            }
        }
    }

    // Every candidate failing means the fallback failed too, which is a broken
    // FFmpeg build rather than a hardware question. Say so with the whole list,
    // because the first refusal alone is misleading.
    if refusals.is_empty() {
        // Only reachable for H.265, which has no software fallback by design.
        return Err(MediaError::DecodeFailed(format!(
            "this machine has no {} encoder",
            target.codec.label()
        )));
    }
    Err(MediaError::DecodeFailed(format!(
        "no usable {} encoder: {}",
        target.codec.label(),
        refusals.join("; ")
    )))
}

/// The encoder for a video with a transparent background: VP9 with an alpha
/// plane, which WebM carries and browsers and editors read. `libvpx-vp9` is
/// BSD-licensed, so §0.1's rule holds.
pub const VP9_ALPHA: EncoderChoice = EncoderChoice {
    name: "libvpx-vp9",
    label: "VP9 with transparency",
    kind: EncoderKind::Software,
    codec: VideoCodec::H264,
};

/// The encoder for a ProRes master: FFmpeg's own `prores_ks`, part of the
/// LGPL build (no GPL component), on the CPU.
pub const PRORES: EncoderChoice = EncoderChoice {
    name: "prores_ks",
    label: "ProRes 422 HQ",
    kind: EncoderKind::Software,
    codec: VideoCodec::H264,
};

/// Open [`PRORES`] at the target's size and rate: 422 HQ, 10-bit 4:2:2,
/// every frame a keyframe as ProRes always is. Quality is set by the
/// profile, not a bitrate.
pub fn open_prores(target: EncodeTarget) -> Result<(EncoderChoice, CodecContext), MediaError> {
    let context = CodecContext::encoder(PRORES.name, target.threads)?;
    let rate = av_rational(target.frame_rate);

    // SAFETY: allocated and not yet opened, which is when these may be set.
    unsafe {
        let ctx = &mut *context.as_ptr();
        ctx.width = target.width as i32;
        ctx.height = target.height as i32;
        ctx.pix_fmt = ffi::AV_PIX_FMT_YUV422P10LE;
        ctx.time_base = ffi::AVRational {
            num: rate.den,
            den: rate.num,
        };
        ctx.framerate = rate;
        ctx.color_range = ffi::AVCOL_RANGE_MPEG;
        ctx.color_primaries = ffi::AVCOL_PRI_BT709;
        ctx.color_trc = ffi::AVCOL_TRC_BT709;
        ctx.colorspace = ffi::AVCOL_SPC_BT709;
        if target.global_header {
            ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    // Profile 3 is 422 HQ: what "a ProRes master" usually means.
    for (name, value) in [("profile", "3"), ("vendor", "apl0")] {
        let (Ok(name_c), Ok(value_c)) =
            (std::ffi::CString::new(name), std::ffi::CString::new(value))
        else {
            continue;
        };
        // SAFETY: allocated and not yet opened; a null `priv_data` is skipped.
        unsafe {
            let priv_data = (*context.as_ptr()).priv_data;
            if !priv_data.is_null() {
                let _ = ffi::av_opt_set(priv_data, name_c.as_ptr(), value_c.as_ptr(), 0);
            }
        }
    }
    context.open_encoder()?;
    Ok((PRORES, context))
}

/// Open [`VP9_ALPHA`] at the target's size and rate, for 4:2:0 with alpha.
pub fn open_vp9_alpha(target: EncodeTarget) -> Result<(EncoderChoice, CodecContext), MediaError> {
    let context = CodecContext::encoder(VP9_ALPHA.name, target.threads)?;
    let rate = av_rational(target.frame_rate);

    // SAFETY: allocated and not yet opened, which is when these may be set.
    unsafe {
        let ctx = &mut *context.as_ptr();
        ctx.width = target.width as i32;
        ctx.height = target.height as i32;
        ctx.pix_fmt = ffi::AV_PIX_FMT_YUVA420P;
        ctx.time_base = ffi::AVRational {
            num: rate.den,
            den: rate.num,
        };
        ctx.framerate = rate;
        ctx.gop_size = keyframe_interval(rate);
        ctx.max_b_frames = 0;
        ctx.color_range = ffi::AVCOL_RANGE_MPEG;
        ctx.color_primaries = ffi::AVCOL_PRI_BT709;
        ctx.color_trc = ffi::AVCOL_TRC_BT709;
        ctx.colorspace = ffi::AVCOL_SPC_BT709;
        ctx.bit_rate = target
            .bitrate
            .filter(|rate| *rate > 0)
            .unwrap_or_else(|| bitrate_for(target.width, target.height, rate, VideoCodec::H264));
        if target.global_header {
            ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    // Speed over the last few percent of size: VP9's defaults are slow enough
    // to make a short export feel stuck.
    for (name, value) in [("deadline", "good"), ("cpu-used", "4"), ("row-mt", "1")] {
        let (Ok(name_c), Ok(value_c)) =
            (std::ffi::CString::new(name), std::ffi::CString::new(value))
        else {
            continue;
        };
        // SAFETY: allocated and not yet opened; a null `priv_data` is skipped.
        unsafe {
            let priv_data = (*context.as_ptr()).priv_data;
            if !priv_data.is_null() {
                let _ = ffi::av_opt_set(priv_data, name_c.as_ptr(), value_c.as_ptr(), 0);
            }
        }
    }
    context.open_encoder()?;
    Ok((VP9_ALPHA, context))
}

/// Open one candidate at the real target format.
///
/// Opening is not a pure FFmpeg operation: it initialises a vendor runtime.
/// `h264_mf` starts Media Foundation, which is process-global; `h264_amf`
/// loads `amfrt64.dll`, which on a machine without AMD hardware is a failed
/// `LoadLibrary` every time. So the open itself (`CodecContext::open_encoder`)
/// waits its turn behind every other codec and device (`crate::gpu_opening`).
fn open(choice: EncoderChoice, target: EncodeTarget) -> Result<CodecContext, MediaError> {
    let context = CodecContext::encoder(choice.name, target.threads)?;

    let rate = av_rational(target.frame_rate);

    // SAFETY: allocated and not yet opened, which is when these may be set.
    unsafe {
        let ctx = &mut *context.as_ptr();
        ctx.width = target.width as i32;
        ctx.height = target.height as i32;
        // Every candidate here takes planar 8-bit 4:2:0. The hardware encoders
        // would also take their own surface formats, which is the zero-copy
        // path §5 describes; that needs the frame to already be on the GPU, and
        // export reads it back to system memory today.
        ctx.pix_fmt = ffi::AV_PIX_FMT_YUV420P;
        ctx.time_base = ffi::AVRational {
            num: rate.den,
            den: rate.num,
        };
        ctx.framerate = rate;

        // Two seconds between keyframes: long enough not to cost bitrate,
        // short enough that a player can seek. The opposite of §13.1's
        // all-intra proxies, which exist to be seeked rather than watched.
        ctx.gop_size = keyframe_interval(rate);

        // No B-frames, and this is load-bearing rather than a quality opinion.
        //
        // A B-frame is coded after the frames it references, so the first
        // packet's decode timestamp lands *before* the first presentation
        // timestamp — negative. MP4 has no way to store that without an edit
        // list, and the muxer resolves it by dropping the packet: a 48-frame
        // export came back as 47, silently, only on the hardware encoder.
        // Measured, not theorised — openh264 defaults to none and wrote all 48.
        //
        // The compression B-frames buy is small at the bitrates above, and
        // uniform behaviour across four encoders is worth more than a few
        // percent on the file size.
        ctx.max_b_frames = 0;

        // §21a: tag the output. An untagged file gets guessed at by players,
        // which is the colour bug arriving at the other end.
        ctx.color_range = ffi::AVCOL_RANGE_MPEG;
        ctx.color_primaries = ffi::AVCOL_PRI_BT709;
        ctx.color_trc = ffi::AVCOL_TRC_BT709;
        ctx.colorspace = ffi::AVCOL_SPC_BT709;

        let bits = target
            .bitrate
            .filter(|rate| *rate > 0)
            .unwrap_or_else(|| bitrate_for(target.width, target.height, rate, target.codec));
        ctx.bit_rate = bits;

        // `bit_rate` alone is advisory, and several encoders treat it that way —
        // NVENC's default rate control spends a fraction of it on easy footage.
        // A ceiling and a buffer are what turn the number into something the
        // encoder honours.
        ctx.rc_max_rate = bits;
        ctx.rc_buffer_size = (bits * 2).clamp(1, i64::from(i32::MAX)) as i32;
        // A floor as well as a ceiling. Necessary but **not sufficient**: the
        // hardware encoders ignore it and read their own private option
        // instead, which `set_rate_control` sets below. Measured — two exports
        // that differed only in this field came out byte-identical on NVENC.
        ctx.rc_min_rate = match target.rate_control {
            RateControl::Constant => bits,
            RateControl::Variable => 0,
        };

        if target.global_header {
            ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    set_rate_control(&context, choice, target.rate_control);

    context.open_encoder()?;
    Ok(context)
}

/// A `FrameRate` as FFmpeg's rational, exactly.
pub(crate) fn av_rational(rate: FrameRate) -> ffi::AVRational {
    let ratio = rate.as_rational();
    ffi::AVRational {
        num: ratio.num() as i32,
        den: ratio.den() as i32,
    }
}

/// Tell the encoder how strictly to hold the bitrate.
///
/// Each vendor spells this differently in its own private options, and the
/// generic `rc_min_rate`/`rc_max_rate` fields do not reach them — two exports
/// differing only in those came out byte-identical on NVENC. So the mode has to
/// be set as the private option the encoder actually reads, before it opens.
///
/// An encoder with no matching option gets nothing and keeps its default, which
/// is the right outcome: `libopenh264` has no constant-rate mode worth the name,
/// and pretending otherwise would be a worse answer than leaving it alone.
fn set_rate_control(context: &CodecContext, choice: EncoderChoice, mode: RateControl) {
    use RateControl::{Constant, Variable};

    let options: &[(&str, &str)] = match (choice.name, mode) {
        ("h264_nvenc" | "hevc_nvenc" | "av1_nvenc", Constant) => &[("rc", "cbr")],
        ("h264_nvenc" | "hevc_nvenc" | "av1_nvenc", Variable) => &[("rc", "vbr")],
        ("h264_amf" | "hevc_amf" | "av1_amf", Constant) => &[("rc", "cbr")],
        ("h264_amf" | "hevc_amf" | "av1_amf", Variable) => &[("rc", "vbr_peak")],
        ("h264_mf" | "hevc_mf" | "av1_mf", Constant) => &[("rate_control", "cbr")],
        ("h264_mf" | "hevc_mf" | "av1_mf", Variable) => &[("rate_control", "u_vbr")],
        // A Mac without the media engine (and GitHub's Mac machines) has no
        // hardware session to open; `allow_sw` lets Apple's own software
        // encoder stand in rather than dropping to openh264. Constant rate is
        // its own switch, honoured on macOS 13 and later.
        ("h264_videotoolbox" | "hevc_videotoolbox", Constant) => {
            &[("allow_sw", "1"), ("constant_bit_rate", "1")]
        }
        ("h264_videotoolbox" | "hevc_videotoolbox", Variable) => &[("allow_sw", "1")],
        // Quick Sync picks its mode from which rate fields are set, which the
        // caller has already done; there is no name to set here.
        _ => &[],
    };

    for (name, value) in options {
        let (Ok(name_c), Ok(value_c)) = (
            std::ffi::CString::new(*name),
            std::ffi::CString::new(*value),
        ) else {
            continue;
        };

        // SAFETY: the context is allocated and not yet opened, which is when
        // private options may be set. `priv_data` is null for encoders with no
        // private options; `av_opt_set` handles that by returning an error.
        let code = unsafe {
            let priv_data = (*context.as_ptr()).priv_data;
            if priv_data.is_null() {
                continue;
            }
            ffi::av_opt_set(priv_data, name_c.as_ptr(), value_c.as_ptr(), 0)
        };

        if code < 0 {
            // Not a §74 "silently ignored failure": the export still runs, at
            // the encoder's default rate control. Worth a line in the log
            // because it means the user's choice did not take effect.
            tracing::debug!(
                encoder = choice.name,
                option = name,
                value,
                reason = %error_string(code),
                "could not set rate control; using the encoder's default"
            );
        }
    }
}

/// Frames between keyframes, from a target of two seconds.
fn keyframe_interval(frame_rate: ffi::AVRational) -> i32 {
    if frame_rate.den <= 0 {
        return 60;
    }
    let per_second = frame_rate.num / frame_rate.den.max(1);
    (per_second * 2).clamp(12, 300)
}

/// A bitrate that looks good without being wasteful.
///
/// Bits per pixel per second, scaled by frame rate: 1080p30 lands near 12 Mb/s
/// and 4K30 near 48, which is in the range consumer platforms re-encode from
/// without visibly degrading. Deliberately generous rather than clever —
/// guessing low produces an export the user has to redo, and disk is cheap next
/// to their time.
fn bitrate_for(width: u32, height: u32, frame_rate: ffi::AVRational, codec: VideoCodec) -> i64 {
    let pixels = i64::from(width) * i64::from(height);
    let fps = if frame_rate.den > 0 {
        i64::from(frame_rate.num) / i64::from(frame_rate.den.max(1))
    } else {
        30
    };
    // 0.2 bits per pixel at 30 fps, scaled linearly with rate.
    let h264 = (pixels * fps.clamp(1, 120) / 5) as f64;
    ((h264 * codec.bitrate_scale()) as i64).clamp(1_000_000, 120_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(num: i32, den: i32) -> ffi::AVRational {
        ffi::AVRational { num, den }
    }

    fn target() -> EncodeTarget {
        target_for(VideoCodec::H264)
    }

    fn target_for(codec: VideoCodec) -> EncodeTarget {
        EncodeTarget {
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::FPS_30,
            codec,
            bitrate: None,
            rate_control: RateControl::Variable,
            threads: 2,
            global_header: true,
        }
    }

    /// What this machine actually offers, printed rather than asserted: the
    /// answer differs per machine and the useful thing is to be able to see it.
    #[test]
    fn report_what_this_machine_can_encode() {
        for codec in VideoCodec::ALL {
            eprintln!("--- {}", codec.label());
            for probe in probe_all(target_for(codec)) {
                match &probe.rejected {
                    None => eprintln!("  OK      {}", probe.choice.name),
                    Some(why) => eprintln!("  no      {} ({why})", probe.choice.name),
                }
            }
        }
    }

    /// §0.1's ordering, asserted rather than assumed: every hardware candidate
    /// must be tried before the software one, or a machine with NVENC would
    /// quietly export on the CPU.
    #[test]
    fn hardware_encoders_are_all_tried_before_the_software_one() {
        let software = CANDIDATES_H264
            .iter()
            .position(|c| c.kind == EncoderKind::Software)
            .expect("there must be a software fallback");
        assert!(
            CANDIDATES_H264[..software]
                .iter()
                .all(|c| c.kind == EncoderKind::Hardware),
            "a software encoder is ordered before a hardware one"
        );
        assert_eq!(
            software,
            CANDIDATES_H264.len() - 1,
            "the software fallback must be last"
        );
    }

    /// §0.1 point 2 and §74: linking x264 makes the whole product GPL. This is
    /// the kind of thing that gets added by someone reaching for the encoder
    /// they know, so it is asserted rather than left to review.
    #[test]
    fn no_gpl_encoder_is_a_candidate() {
        for choice in CANDIDATES_H264
            .iter()
            .chain(CANDIDATES_H265)
            .chain(CANDIDATES_AV1)
        {
            assert!(
                !choice.name.contains("x264") && !choice.name.contains("x265"),
                "{} is GPL and must never be linked (§0.1, §74)",
                choice.name
            );
        }
    }

    /// H.265 has no software fallback, on purpose: x265 is GPL. This asserts
    /// the absence, so nobody 'fixes' the gap by reaching for the obvious
    /// encoder and taking the whole product GPL with it.
    #[test]
    fn h265_has_no_software_encoder() {
        assert!(
            CANDIDATES_H265
                .iter()
                .all(|c| c.kind == EncoderKind::Hardware),
            "a software H.265 encoder appeared; check its licence against §0.1"
        );
    }

    /// AV1, like H.265, is offered only where a GPU can write it: no software
    /// AV1 encoder is linked, so the choice never means a slow CPU export the
    /// person did not ask for.
    #[test]
    fn av1_has_no_software_encoder() {
        assert!(
            CANDIDATES_AV1
                .iter()
                .all(|c| c.kind == EncoderKind::Hardware && c.codec == VideoCodec::Av1),
            "a software or mislabelled AV1 encoder appeared"
        );
        assert!(VideoCodec::Av1.bitrate_scale() < VideoCodec::H265.bitrate_scale());
    }

    /// There must always be something to fall back to, on any platform.
    #[test]
    fn every_platform_has_a_fallback() {
        assert!(
            CANDIDATES_H264
                .iter()
                .any(|c| c.name == "libopenh264" && c.kind == EncoderKind::Software)
        );
    }

    /// Two seconds of frames, whatever the rate — including the NTSC rates,
    /// whose ratios are not integers.
    #[test]
    fn the_keyframe_interval_is_about_two_seconds() {
        assert_eq!(keyframe_interval(rate(30, 1)), 60);
        assert_eq!(keyframe_interval(rate(60, 1)), 120);
        assert_eq!(keyframe_interval(rate(24, 1)), 48);
        // 29.97 and 23.976: integer division lands on 29 and 23 per second.
        assert_eq!(keyframe_interval(rate(30_000, 1001)), 58);
        assert_eq!(keyframe_interval(rate(24_000, 1001)), 46);
        // Nonsense must not produce a zero GOP, which encodes every frame as a
        // keyframe and quadruples the file.
        assert_eq!(keyframe_interval(rate(30, 0)), 60);
    }

    #[test]
    fn the_bitrate_scales_with_pixels_and_rate() {
        let hd30 = bitrate_for(1920, 1080, rate(30, 1), VideoCodec::H264);
        let hd60 = bitrate_for(1920, 1080, rate(60, 1), VideoCodec::H264);
        let uhd30 = bitrate_for(3840, 2160, rate(30, 1), VideoCodec::H264);

        assert!((10_000_000..=16_000_000).contains(&hd30), "1080p30: {hd30}");
        assert_eq!(hd60, hd30 * 2, "doubling the rate should double the rate");
        assert_eq!(
            uhd30,
            hd30 * 4,
            "four times the pixels, four times the bits"
        );
        // Even a tiny sequence must get a usable bitrate.
        assert!(bitrate_for(64, 64, rate(24, 1), VideoCodec::H264) >= 1_000_000);

        // H.265 is about twice as efficient, so the automatic rate is lower for
        // the same picture rather than the same number for a smaller file.
        let hevc = bitrate_for(1920, 1080, rate(30, 1), VideoCodec::H265);
        assert!(hevc < hd30, "H.265 should ask for less: {hevc} vs {hd30}");
        assert!(hevc > hd30 / 2, "and not so much less that it looks worse");
    }

    /// The real test: this machine must be able to export.
    ///
    /// Not skipped when there is no hardware encoder — the software fallback is
    /// linked into the binary and has no excuse. A failure here means an
    /// FFmpeg build that cannot encode H.264 at all, which would make §60's
    /// last criterion unreachable.
    #[test]
    fn this_machine_can_open_an_h264_encoder() {
        let (choice, _context) = open_best(target()).expect("no usable H.264 encoder");
        eprintln!("selected {} ({})", choice.label, choice.name);
    }

    /// The probe is what the export dialog shows, so it has to describe every
    /// candidate rather than stopping at the first that works.
    #[test]
    fn probing_reports_on_every_candidate() {
        let probes = probe_all(target());
        assert_eq!(probes.len(), CANDIDATES_H264.len());
        assert!(
            probes.iter().any(EncoderProbe::is_usable),
            "nothing on this machine can encode H.264: {:?}",
            probes
                .iter()
                .map(|p| format!("{}: {:?}", p.choice.name, p.rejected))
                .collect::<Vec<_>>()
        );
    }
}
