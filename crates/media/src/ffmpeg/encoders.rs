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

use rusty_ffmpeg::ffi;

use crate::error::MediaError;

use super::raii::CodecContext;

/// An H.264 encoder that was found and successfully opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderChoice {
    /// The FFmpeg encoder name, e.g. `h264_nvenc`.
    pub name: &'static str,
    /// What to call it in the interface.
    pub label: &'static str,
    pub kind: EncoderKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderKind {
    /// Provided by the GPU driver or the OS. Distributed by neither us nor the
    /// user (§0.1).
    Hardware,
    /// `libopenh264`: BSD, linked into our binary, runs on the CPU.
    Software,
}

impl EncoderKind {
    pub fn is_hardware(self) -> bool {
        matches!(self, Self::Hardware)
    }
}

/// Candidates in preference order.
///
/// Hardware first, and within hardware the vendor-specific encoders ahead of
/// the generic Media Foundation wrapper: `h264_mf` reaches the same silicon
/// through another layer, and exposes fewer controls doing it. The software
/// encoder is last and is the only entry guaranteed to exist.
const CANDIDATES: &[EncoderChoice] = &[
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_qsv",
        label: "Intel Quick Sync",
        kind: EncoderKind::Hardware,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_amf",
        label: "AMD AMF",
        kind: EncoderKind::Hardware,
    },
    #[cfg(target_os = "windows")]
    EncoderChoice {
        name: "h264_mf",
        label: "Windows Media Foundation",
        kind: EncoderKind::Hardware,
    },
    #[cfg(target_os = "macos")]
    EncoderChoice {
        name: "h264_videotoolbox",
        label: "VideoToolbox",
        kind: EncoderKind::Hardware,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "h264_nvenc",
        label: "NVIDIA NVENC",
        kind: EncoderKind::Hardware,
    },
    #[cfg(target_os = "linux")]
    EncoderChoice {
        name: "h264_vaapi",
        label: "VAAPI",
        kind: EncoderKind::Hardware,
    },
    // §0.1's fallback, and §13.1's proxy encoder. BSD, so shipping it is
    // settled; ADR 002 records the reasoning.
    EncoderChoice {
        name: "libopenh264",
        label: "openh264 (software)",
        kind: EncoderKind::Software,
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
    pub frame_rate: ffi::AVRational,
    /// §15.1: FFmpeg must never be allowed every core.
    pub threads: u32,
    /// MP4 keeps SPS/PPS in the container. Must be known before opening.
    pub global_header: bool,
}

/// Try every candidate and report what happened to each.
///
/// Used by the export dialog, which shows the user what their machine offers
/// rather than silently picking — §41: the interface explains itself, and
/// "why is my export slow" has an answer here.
pub fn probe_all(target: EncodeTarget) -> Vec<EncoderProbe> {
    CANDIDATES
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
    for choice in CANDIDATES {
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
    Err(MediaError::DecodeFailed(format!(
        "no usable H.264 encoder: {}",
        refusals.join("; ")
    )))
}

/// Open one candidate at the real target format.
fn open(choice: EncoderChoice, target: EncodeTarget) -> Result<CodecContext, MediaError> {
    let context = CodecContext::encoder(choice.name, target.threads)?;

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
            num: target.frame_rate.den,
            den: target.frame_rate.num,
        };
        ctx.framerate = target.frame_rate;

        // Two seconds between keyframes: long enough not to cost bitrate,
        // short enough that a player can seek. The opposite of §13.1's
        // all-intra proxies, which exist to be seeked rather than watched.
        ctx.gop_size = keyframe_interval(target.frame_rate);

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

        ctx.bit_rate = bitrate_for(target.width, target.height, target.frame_rate);

        if target.global_header {
            ctx.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    context.open_encoder()?;
    Ok(context)
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
fn bitrate_for(width: u32, height: u32, frame_rate: ffi::AVRational) -> i64 {
    let pixels = i64::from(width) * i64::from(height);
    let fps = if frame_rate.den > 0 {
        i64::from(frame_rate.num) / i64::from(frame_rate.den.max(1))
    } else {
        30
    };
    // 0.2 bits per pixel at 30 fps, scaled linearly with rate.
    (pixels * fps.clamp(1, 120) / 5).clamp(1_000_000, 120_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(num: i32, den: i32) -> ffi::AVRational {
        ffi::AVRational { num, den }
    }

    fn target() -> EncodeTarget {
        EncodeTarget {
            width: 1920,
            height: 1080,
            frame_rate: rate(30, 1),
            threads: 2,
            global_header: true,
        }
    }

    /// §0.1's ordering, asserted rather than assumed: every hardware candidate
    /// must be tried before the software one, or a machine with NVENC would
    /// quietly export on the CPU.
    #[test]
    fn hardware_encoders_are_all_tried_before_the_software_one() {
        let software = CANDIDATES
            .iter()
            .position(|c| c.kind == EncoderKind::Software)
            .expect("there must be a software fallback");
        assert!(
            CANDIDATES[..software]
                .iter()
                .all(|c| c.kind == EncoderKind::Hardware),
            "a software encoder is ordered before a hardware one"
        );
        assert_eq!(
            software,
            CANDIDATES.len() - 1,
            "the software fallback must be last"
        );
    }

    /// §0.1 point 2 and §74: linking x264 makes the whole product GPL. This is
    /// the kind of thing that gets added by someone reaching for the encoder
    /// they know, so it is asserted rather than left to review.
    #[test]
    fn no_gpl_encoder_is_a_candidate() {
        for choice in CANDIDATES {
            assert!(
                !choice.name.contains("x264") && !choice.name.contains("x265"),
                "{} is GPL and must never be linked (§0.1, §74)",
                choice.name
            );
        }
    }

    /// There must always be something to fall back to, on any platform.
    #[test]
    fn every_platform_has_a_fallback() {
        assert!(
            CANDIDATES
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
        let hd30 = bitrate_for(1920, 1080, rate(30, 1));
        let hd60 = bitrate_for(1920, 1080, rate(60, 1));
        let uhd30 = bitrate_for(3840, 2160, rate(30, 1));

        assert!((10_000_000..=16_000_000).contains(&hd30), "1080p30: {hd30}");
        assert_eq!(hd60, hd30 * 2, "doubling the rate should double the rate");
        assert_eq!(
            uhd30,
            hd30 * 4,
            "four times the pixels, four times the bits"
        );
        // Even a tiny sequence must get a usable bitrate.
        assert!(bitrate_for(64, 64, rate(24, 1)) >= 1_000_000);
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
        assert_eq!(probes.len(), CANDIDATES.len());
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
