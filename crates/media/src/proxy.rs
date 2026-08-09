//! Proxy media state (§13, §14).
//!
//! §13.1 is unusually specific about the encoding, and the reason is worth
//! keeping next to the code: **GOP length 1**. Long-GOP media requires seeking
//! to the preceding keyframe and decoding forward — up to 250 decodes per seek
//! on a 250-frame GOP. With every frame a keyframe, a seek is one decode. That
//! single choice is what makes §81's 150 ms seek target reachable on the target
//! hardware.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyStatus {
    Missing,
    Queued,
    Generating,
    Ready,
    Failed,
}

impl ProxyStatus {
    /// Only a ready proxy may be used for preview (§14).
    pub fn is_usable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Proxy resolutions (§13). Default 720p; 540p on weak hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyResolution {
    P360,
    P540,
    P720,
}

impl ProxyResolution {
    pub fn height(self) -> u32 {
        match self {
            Self::P360 => 360,
            Self::P540 => 540,
            Self::P720 => 720,
        }
    }

    /// Width for a given source aspect, rounded to an even number — H.264 with
    /// yuv420p chroma subsampling requires even dimensions.
    pub fn width_for_aspect(self, aspect: f32) -> u32 {
        let w = (self.height() as f32 * aspect).round() as u32;
        w.max(2) & !1
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAsset {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub status: ProxyStatus,
}

impl ProxyAsset {
    pub fn queued(path: impl Into<PathBuf>, width: u32, height: u32) -> Self {
        Self {
            path: path.into(),
            width,
            height,
            status: ProxyStatus::Queued,
        }
    }
}

/// The §13.1 encoding specification, as data rather than prose.
///
/// Held here so the Milestone 7 encoder cannot quietly disagree with the guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxySpec {
    pub gop_length: u32,
    pub bit_depth: u8,
    pub audio_sample_rate: u32,
    pub constant_frame_rate: bool,
}

impl ProxySpec {
    pub const V1: Self = Self {
        // All-intra. Not a tuning knob — see the module docs.
        gop_length: 1,
        bit_depth: 8,
        audio_sample_rate: 48_000,
        constant_frame_rate: true,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// If this ever fails, someone has traded §81's seek target for disk space.
    #[test]
    fn the_proxy_spec_is_all_intra() {
        assert_eq!(ProxySpec::V1.gop_length, 1);
    }

    #[test]
    fn proxy_audio_matches_the_timebase_requirement() {
        // §9: 960,000 does not divide 44,100, so proxies must be 48 kHz.
        assert_eq!(ProxySpec::V1.audio_sample_rate, 48_000);
        assert_eq!(
            bettercut_foundation::TICKS_PER_SECOND % ProxySpec::V1.audio_sample_rate as i64,
            0
        );
    }

    #[test]
    fn proxy_widths_are_even_for_yuv420p() {
        for res in [
            ProxyResolution::P360,
            ProxyResolution::P540,
            ProxyResolution::P720,
        ] {
            for aspect in [16.0 / 9.0, 9.0 / 16.0, 1.0, 4.0 / 5.0, 2.35] {
                let w = res.width_for_aspect(aspect);
                assert_eq!(w % 2, 0, "{res:?} at aspect {aspect} gave odd width {w}");
                assert!(w >= 2);
            }
        }
    }

    #[test]
    fn only_a_ready_proxy_is_usable() {
        assert!(ProxyStatus::Ready.is_usable());
        for s in [
            ProxyStatus::Missing,
            ProxyStatus::Queued,
            ProxyStatus::Generating,
            ProxyStatus::Failed,
        ] {
            assert!(!s.is_usable(), "{s:?} must not be used for preview");
        }
    }
}
