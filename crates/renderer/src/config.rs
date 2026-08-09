//! Render configuration (§46).
//!
//! > **There is exactly one render graph implementation.** Preview and export
//! > are two *configurations* of it.
//!
//! §46 is unusually firm about this, and gives the reason: two separate
//! renderer implementations will diverge, and that is "not a risk, it is a
//! certainty". §74 puts "introduce a second renderer implementation" on the
//! prohibited list.
//!
//! So the differences between preview and export live here, as data, rather
//! than in two code paths:
//!
//! ```text
//! Resolution
//! Source media (proxy vs original)
//! Effect quality tier (§45)
//! Output sink (screen texture vs encoder)
//! ```

use bettercut_timeline::Resolution;

/// Which media the graph reads (§14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSourceMode {
    /// Editing uses proxies (§14). Faster, and the only way §81's seek targets
    /// are reachable on the reference machine.
    Proxy,
    /// Export uses the original media, always.
    Original,
}

/// How expensive an effect implementation may be (§45).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTier {
    /// Simplified implementations are allowed for expensive effects.
    Preview,
    /// Full quality, whatever it costs.
    Full,
}

/// Where the composited frame goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTarget {
    /// A texture the UI paints (§4.1: already a `wgpu::Texture`, so there is
    /// nothing to copy).
    Texture,
    /// An encoder input. Milestone 6.
    Encoder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderConfig {
    pub resolution: Resolution,
    pub media_source: MediaSourceMode,
    pub effect_quality: QualityTier,
    pub output: RenderTarget,
}

impl RenderConfig {
    /// Preview at a given resolution.
    ///
    /// §17 lowers this while playback is struggling and raises it again when
    /// paused, so it is deliberately not tied to the sequence's own size.
    pub fn preview(resolution: Resolution) -> Self {
        Self {
            resolution,
            media_source: MediaSourceMode::Proxy,
            effect_quality: QualityTier::Preview,
            output: RenderTarget::Texture,
        }
    }

    /// Export at the sequence's full resolution (§40).
    pub fn export(resolution: Resolution) -> Self {
        Self {
            resolution,
            media_source: MediaSourceMode::Original,
            effect_quality: QualityTier::Full,
            output: RenderTarget::Encoder,
        }
    }

    /// The same graph as `export`, but rendered to a texture.
    ///
    /// This is what §51.1's golden-frame tests compare against the preview
    /// configuration: the two must agree per pixel, and they can only be
    /// compared if both can be read back as images.
    pub fn export_to_texture(resolution: Resolution) -> Self {
        Self {
            output: RenderTarget::Texture,
            ..Self::export(resolution)
        }
    }
}

/// Preview scale (§16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewQuality {
    Full,
    Half,
    #[default]
    Quarter,
    /// Chosen from measured playback performance (§17).
    Auto,
}

impl PreviewQuality {
    /// Divisor applied to the sequence resolution.
    pub fn divisor(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Half => 2,
            // `Auto` starts conservative and §17's hysteresis moves it.
            Self::Quarter | Self::Auto => 4,
        }
    }

    /// Scale a sequence resolution, keeping both dimensions even and non-zero.
    ///
    /// Even because chroma-subsampled encoders require it, non-zero because a
    /// zero-sized texture is a validation error rather than a small picture.
    pub fn apply(self, resolution: Resolution) -> Resolution {
        let divisor = self.divisor();
        Resolution::new(
            ((resolution.width / divisor).max(2)) & !1,
            ((resolution.height / divisor).max(2)) & !1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §46: preview and export differ only in these four fields. If a fifth
    /// ever appears, it belongs here rather than in a branch somewhere.
    #[test]
    fn preview_and_export_differ_only_in_configuration() {
        let resolution = Resolution::HD_1080;
        let preview = RenderConfig::preview(resolution);
        let export = RenderConfig::export(resolution);

        assert_eq!(preview.resolution, export.resolution);
        assert_eq!(preview.media_source, MediaSourceMode::Proxy);
        assert_eq!(export.media_source, MediaSourceMode::Original);
        assert_eq!(preview.effect_quality, QualityTier::Preview);
        assert_eq!(export.effect_quality, QualityTier::Full);
    }

    /// The golden-frame configuration must match export in everything except
    /// where the pixels land, or §51.1 would be comparing the wrong things.
    #[test]
    fn the_golden_frame_config_matches_export_except_for_the_sink() {
        let resolution = Resolution::HD_1080;
        let export = RenderConfig::export(resolution);
        let golden = RenderConfig::export_to_texture(resolution);

        assert_eq!(golden.media_source, export.media_source);
        assert_eq!(golden.effect_quality, export.effect_quality);
        assert_eq!(golden.resolution, export.resolution);
        assert_eq!(golden.output, RenderTarget::Texture);
    }

    #[test]
    fn preview_scaling_stays_even_and_non_zero() {
        for quality in [
            PreviewQuality::Full,
            PreviewQuality::Half,
            PreviewQuality::Quarter,
            PreviewQuality::Auto,
        ] {
            for resolution in [
                Resolution::HD_1080,
                Resolution::HD_720,
                Resolution::VERTICAL_1080,
                Resolution::new(3, 7),
            ] {
                let scaled = quality.apply(resolution);
                assert!(
                    scaled.width >= 2 && scaled.height >= 2,
                    "{scaled:?} too small"
                );
                assert_eq!(scaled.width % 2, 0, "{scaled:?} has an odd width");
                assert_eq!(scaled.height % 2, 0, "{scaled:?} has an odd height");
            }
        }
    }

    #[test]
    fn quarter_quality_quarters_each_dimension() {
        let scaled = PreviewQuality::Quarter.apply(Resolution::HD_1080);
        assert_eq!((scaled.width, scaled.height), (480, 270));
    }
}
