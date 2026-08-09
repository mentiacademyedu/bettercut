//! Colour metadata (§21a).
//!
//! §46 promises preview and export match. Without a stated colour policy they
//! will not, and the failure mode is the classic "my export looks washed out".
//!
//! The single most important item here is **range**. Most camera and phone H.264
//! is limited-range (16–235). Treating it as full-range crushes blacks and clips
//! whites, and the error is small enough that nobody notices until a user
//! compares against another player.

use serde::{Deserialize, Serialize};

/// Colour primaries. Determines the chromaticity of R, G, B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorPrimaries {
    Bt601Ntsc,
    Bt601Pal,
    #[default]
    Bt709,
    Bt2020,
    /// Present in the file but not one we handle; treated as BT.709.
    Unknown,
}

/// Transfer function (gamma).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferFunction {
    #[default]
    Bt709,
    Srgb,
    /// SMPTE ST 2084. HDR.
    Pq,
    /// Hybrid log-gamma. HDR.
    Hlg,
    Unknown,
}

impl TransferFunction {
    /// HDR transfers need tone-mapping into the SDR working space (§21a.1).
    ///
    /// **Detected, not yet handled.** This currently only decides whether a
    /// proxy is worth generating ([`ColorMetadata::needs_normalization`]).
    /// Nothing in the upload path converts PQ or HLG to the SDR working space,
    /// and the proxy encoder rescales and retags without transforming the
    /// transfer either — `sws_setColorspaceDetails` covers matrix and range,
    /// not gamma. HDR footage therefore reaches the screen with its code
    /// values interpreted as BT.709 and looks dark and flat.
    pub fn is_hdr(self) -> bool {
        matches!(self, Self::Pq | Self::Hlg)
    }
}

/// YCbCr → RGB matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMatrix {
    Bt601,
    #[default]
    Bt709,
    Bt2020Ncl,
    Unknown,
}

/// Luma/chroma value range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorRange {
    /// 16–235 luma, 16–240 chroma. The default for camera and phone H.264.
    #[default]
    Limited,
    /// 0–255.
    Full,
}

impl ColorRange {
    /// True when the upload shader must expand 16–235 to 0–255 (§21a.2).
    pub fn needs_expansion(self) -> bool {
        matches!(self, Self::Limited)
    }
}

/// Everything the upload-boundary conversion needs to know about a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColorMetadata {
    pub primaries: ColorPrimaries,
    pub transfer: TransferFunction,
    pub matrix: ColorMatrix,
    pub range: ColorRange,
    pub bit_depth: u8,
}

impl ColorMetadata {
    /// §21a.2's stated defaults, for files whose metadata is absent — which is
    /// most screen recordings and a good share of phone video.
    ///
    /// ```text
    /// SD  (height <= 576) → BT.601
    /// HD+ (height >  576) → BT.709
    /// Range unspecified   → limited
    /// ```
    pub fn guess_from_height(height: u32) -> Self {
        if height <= 576 {
            Self {
                primaries: ColorPrimaries::Bt601Pal,
                transfer: TransferFunction::Bt709,
                matrix: ColorMatrix::Bt601,
                range: ColorRange::Limited,
                bit_depth: 8,
            }
        } else {
            Self {
                primaries: ColorPrimaries::Bt709,
                transfer: TransferFunction::Bt709,
                matrix: ColorMatrix::Bt709,
                range: ColorRange::Limited,
                bit_depth: 8,
            }
        }
    }

    /// What the §13.1 proxy normalizes everything to, and what §21a.3 tags on
    /// export output.
    pub const fn bt709_limited_8bit() -> Self {
        Self {
            primaries: ColorPrimaries::Bt709,
            transfer: TransferFunction::Bt709,
            matrix: ColorMatrix::Bt709,
            range: ColorRange::Limited,
            bit_depth: 8,
        }
    }

    /// True when this source needs work beyond a straight copy at upload —
    /// used to decide whether a proxy is worth generating (§13).
    pub fn needs_normalization(self) -> bool {
        self.transfer.is_hdr() || self.bit_depth > 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_range_is_the_default_and_needs_expansion() {
        // The §21a.2 bug this whole module exists to prevent.
        assert_eq!(ColorRange::default(), ColorRange::Limited);
        assert!(ColorRange::default().needs_expansion());
        assert!(!ColorRange::Full.needs_expansion());
    }

    #[test]
    fn guessing_follows_the_576_line_split() {
        assert_eq!(
            ColorMetadata::guess_from_height(480).matrix,
            ColorMatrix::Bt601
        );
        assert_eq!(
            ColorMetadata::guess_from_height(576).matrix,
            ColorMatrix::Bt601
        );
        assert_eq!(
            ColorMetadata::guess_from_height(720).matrix,
            ColorMatrix::Bt709
        );
        assert_eq!(
            ColorMetadata::guess_from_height(2160).matrix,
            ColorMatrix::Bt709
        );
    }

    #[test]
    fn guessed_range_is_always_limited() {
        for h in [240, 576, 1080, 2160] {
            assert_eq!(
                ColorMetadata::guess_from_height(h).range,
                ColorRange::Limited
            );
        }
    }

    #[test]
    fn hdr_and_ten_bit_sources_need_normalizing() {
        let hdr = ColorMetadata {
            transfer: TransferFunction::Pq,
            bit_depth: 10,
            ..Default::default()
        };
        assert!(hdr.needs_normalization());
        assert!(!ColorMetadata::bt709_limited_8bit().needs_normalization());
    }

    #[test]
    fn color_metadata_round_trips_through_json() {
        let c = ColorMetadata {
            primaries: ColorPrimaries::Bt2020,
            transfer: TransferFunction::Hlg,
            matrix: ColorMatrix::Bt2020Ncl,
            range: ColorRange::Full,
            bit_depth: 10,
        };
        let json = serde_json::to_string(&c).expect("serialize");
        assert!(json.contains("bt2020"), "expected readable names: {json}");
        assert_eq!(
            serde_json::from_str::<ColorMetadata>(&json).expect("deserialize"),
            c
        );
    }
}
