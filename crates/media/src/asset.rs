//! Media assets: what the project knows about an imported file (§12).

use std::path::{Path, PathBuf};

use bettercut_foundation::{FrameRate, MediaId, MediaTime};
use serde::{Deserialize, Serialize};

use crate::color::ColorMetadata;
use crate::proxy::ProxyAsset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

impl MediaKind {
    pub fn has_video(self) -> bool {
        matches!(self, Self::Video | Self::Image)
    }

    pub fn has_audio(self) -> bool {
        matches!(self, Self::Video | Self::Audio)
    }
}

/// How long a photo runs when it is first placed. Long enough to register,
/// short enough that a handful make a montage without trimming each one.
pub const STILL_DURATION: MediaTime = MediaTime::from_seconds(5);

/// The longest edge a decoded still keeps, in pixels.
///
/// A phone photo is 4000–8000 pixels across, and decoded to RGBA at full size
/// it is 60–250 MB for one frame, past what many GPUs accept as one texture.
/// Nothing is exported larger than 4K, so detail past this edge could never
/// reach the output anyway.
pub const MAX_STILL_EDGE: u32 = 4096;

/// A size scaled down, keeping its shape, until neither edge is over `limit`.
///
/// Even dimensions, like every size the pipeline hands to an encoder, and
/// never zero.
pub fn fit_within(width: u32, height: u32, limit: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= limit || long == 0 {
        return (width, height);
    }
    let scale = |edge: u32| {
        let scaled = (u64::from(edge) * u64::from(limit) / u64::from(long)) as u32;
        (scaled & !1).max(2)
    };
    (scale(width), scale(height))
}

/// An imported source file, referenced by clips but never copied (§2).
///
/// `path` is stored alongside `file_name` and `file_size` so §66's relink can
/// find a moved file by name and confirm it by size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: MediaId,
    pub kind: MediaKind,

    pub path: PathBuf,
    /// Kept separately so relinking survives the path being wrong (§66).
    pub file_name: String,
    pub file_size: u64,

    pub width: u32,
    pub height: u32,
    pub duration: MediaTime,
    pub frame_rate: Option<FrameRate>,

    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// Source rate. Everything is resampled to 48 kHz internally (§20a.3).
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u16>,

    pub color: ColorMetadata,

    #[serde(default)]
    pub proxy: Option<ProxyAsset>,

    /// Set when the file cannot be found on disk. The clip renders as
    /// unavailable and the session continues (§50, §66).
    #[serde(default)]
    pub missing: bool,
}

impl MediaAsset {
    /// Build an asset from already-probed metadata.
    ///
    /// Milestone 2 fills this in from FFmpeg; until then it is how tests and
    /// fixtures construct assets.
    pub fn new(kind: MediaKind, path: impl Into<PathBuf>, duration: MediaTime) -> Self {
        let path = path.into();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        Self {
            id: MediaId::new(),
            kind,
            path,
            file_name,
            file_size: 0,
            width: 0,
            height: 0,
            duration,
            frame_rate: None,
            video_codec: None,
            audio_codec: None,
            audio_sample_rate: None,
            audio_channels: None,
            color: ColorMetadata::default(),
            proxy: None,
            missing: false,
        }
    }

    pub fn with_video(mut self, width: u32, height: u32, frame_rate: FrameRate) -> Self {
        self.width = width;
        self.height = height;
        self.frame_rate = Some(frame_rate);
        // §21a.2: resolve colour at import so nothing downstream has to guess.
        self.color = ColorMetadata::guess_from_height(height);
        self
    }

    pub fn with_audio(mut self, sample_rate: u32, channels: u16) -> Self {
        self.audio_sample_rate = Some(sample_rate);
        self.audio_channels = Some(channels);
        self
    }

    /// A still image: one picture, with no length of its own.
    pub fn is_still(&self) -> bool {
        self.kind == MediaKind::Image
    }

    /// The instant in the file that holds the picture for `time`.
    ///
    /// A still has one picture and every instant shows it. Mapping them all
    /// to zero here, rather than asking the decoder for a time the file does
    /// not have, is also what makes a frame cache keyed by time hit on every
    /// frame of a still clip — otherwise the photo would be decoded again for
    /// each frame of playback.
    pub fn frame_time(&self, time: MediaTime) -> MediaTime {
        if self.is_still() {
            MediaTime::ZERO
        } else {
            time
        }
    }

    /// How far into the file a clip may read: `None` for a still, which runs
    /// as long as it is dragged out to.
    pub fn source_limit(&self) -> Option<MediaTime> {
        (!self.is_still()).then_some(self.duration)
    }

    /// How long a clip of this runs when first put on the timeline: the whole
    /// file, or [`STILL_DURATION`] for a picture.
    pub fn placement_duration(&self) -> MediaTime {
        if self.is_still() {
            STILL_DURATION
        } else {
            self.duration
        }
    }

    /// Whether §13's rules say this asset should get a proxy.
    ///
    /// ```text
    /// >= 1440p · HEVC/H.265 · AV1 · 10-bit · high frame rate
    /// ```
    ///
    /// Never for a still: a proxy is a lighter *video* to decode on every
    /// frame, and a still is decoded once. A large photo is scaled down at
    /// decode instead ([`MAX_STILL_EDGE`]).
    pub fn should_generate_proxy(&self) -> bool {
        if self.kind != MediaKind::Video {
            return false;
        }
        let big = self.height >= 1440;
        let hard_codec = self
            .video_codec
            .as_deref()
            .is_some_and(|c| matches!(c, "hevc" | "h265" | "av1" | "vp9"));
        let deep = self.color.bit_depth > 8;
        let fast = self.frame_rate.is_some_and(|r| r.as_f64() > 60.5);

        big || hard_codec || deep || fast || self.color.needs_normalization()
    }

    /// Does the file still exist where we last saw it? (§66)
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// A candidate relink target: same file name, matching size (§66).
    pub fn matches_relink_candidate(&self, candidate: &Path) -> bool {
        let name_matches = candidate
            .file_name()
            .is_some_and(|n| n.to_string_lossy() == self.file_name);
        if !name_matches {
            return false;
        }
        // Size 0 means we never recorded one, so name alone has to do.
        if self.file_size == 0 {
            return true;
        }
        std::fs::metadata(candidate).is_ok_and(|m| m.len() == self.file_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_fitted_keeping_its_shape() {
        assert_eq!(fit_within(1920, 1080, 4096), (1920, 1080), "small enough");
        assert_eq!(fit_within(8000, 6000, 4096), (4096, 3072));
        assert_eq!(fit_within(3000, 9000, 4096), (1364, 4096), "portrait");
        assert_eq!(fit_within(100_000, 1, 4096), (4096, 2), "never zero");
        assert_eq!(fit_within(0, 0, 4096), (0, 0));
    }

    #[test]
    fn a_still_shows_one_picture_for_as_long_as_it_runs() {
        let still = MediaAsset::new(MediaKind::Image, "C:/media/p.jpg", MediaTime::ZERO);
        assert_eq!(
            still.frame_time(MediaTime::from_seconds(7)),
            MediaTime::ZERO
        );
        assert_eq!(still.source_limit(), None);
        assert_eq!(still.placement_duration(), STILL_DURATION);
        assert!(
            !MediaAsset {
                height: 4000,
                ..still
            }
            .should_generate_proxy()
        );

        let clip = video(1080);
        let later = MediaTime::from_seconds(7);
        assert_eq!(clip.frame_time(later), later);
        assert_eq!(clip.source_limit(), Some(clip.duration));
    }

    fn video(height: u32) -> MediaAsset {
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/clip.mp4",
            MediaTime::from_seconds(10),
        )
        .with_video(height * 16 / 9, height, FrameRate::FPS_30)
    }

    #[test]
    fn file_name_is_extracted_from_the_path() {
        let a = video(1080);
        assert_eq!(a.file_name, "clip.mp4");
    }

    #[test]
    fn proxy_rules_follow_section_13() {
        assert!(
            !video(1080).should_generate_proxy(),
            "1080p30 h264 needs none"
        );
        assert!(video(1440).should_generate_proxy(), "1440p qualifies");
        assert!(video(2160).should_generate_proxy(), "4K qualifies");

        let mut hevc = video(1080);
        hevc.video_codec = Some("hevc".to_owned());
        assert!(hevc.should_generate_proxy());

        let mut ten_bit = video(1080);
        ten_bit.color.bit_depth = 10;
        assert!(ten_bit.should_generate_proxy());

        let mut high_fps = video(1080);
        high_fps.frame_rate = Some(FrameRate::FPS_120);
        assert!(high_fps.should_generate_proxy());
    }

    #[test]
    fn audio_never_gets_a_video_proxy() {
        let a = MediaAsset::new(
            MediaKind::Audio,
            "C:/media/music.wav",
            MediaTime::from_seconds(60),
        );
        assert!(!a.should_generate_proxy());
    }

    #[test]
    fn colour_is_resolved_at_import_not_left_unknown() {
        // §21a.2: nothing downstream should have to guess.
        use crate::color::{ColorMatrix, ColorRange};
        assert_eq!(video(480).color.matrix, ColorMatrix::Bt601);
        assert_eq!(video(1080).color.matrix, ColorMatrix::Bt709);
        assert_eq!(video(1080).color.range, ColorRange::Limited);
    }

    #[test]
    fn relink_requires_a_matching_name() {
        let a = video(1080);
        assert!(a.matches_relink_candidate(Path::new("D:/backup/clip.mp4")));
        assert!(!a.matches_relink_candidate(Path::new("D:/backup/other.mp4")));
    }

    #[test]
    fn asset_round_trips_through_json() {
        let a = video(1080).with_audio(44_100, 2);
        let json = serde_json::to_string(&a).expect("serialize");
        assert_eq!(
            serde_json::from_str::<MediaAsset>(&json).expect("deserialize"),
            a
        );
    }
}
