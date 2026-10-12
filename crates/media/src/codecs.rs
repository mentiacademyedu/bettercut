//! What an export asks for: the codec, how its bitrate is spent, the
//! encoder that ran, the file's format and chapters.
//!
//! Plain data, kept apart from the FFmpeg code that acts on it
//! (`crate::ffmpeg`) so it exists where FFmpeg does not: the web build,
//! which encodes with the browser's own codecs, offers the same choices.

// The encoder lists are FFmpeg's to try; the web build never does.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use bettercut_foundation::{FrameRate, MediaTime};

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
    pub(crate) fn bitrate_scale(self) -> f64 {
        match self {
            Self::H264 => 1.0,
            Self::H265 => 0.55,
            Self::Av1 => 0.45,
        }
    }

    pub(crate) fn candidates(self) -> &'static [EncoderChoice] {
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
pub(crate) const CANDIDATES_H264: &[EncoderChoice] = &[
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
pub(crate) const CANDIDATES_H265: &[EncoderChoice] = &[
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
pub(crate) const CANDIDATES_AV1: &[EncoderChoice] = &[
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

/// How the output is to be written.
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
