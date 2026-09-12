//! Sequences: a canvas, a frame rate, and stacks of tracks (§8).

use bettercut_foundation::{ClipId, FrameRate, SequenceId, TimelineTime, TrackId, ticks_per_frame};
use serde::{Deserialize, Serialize};

use crate::error::TimelineError;
use crate::text::TextTrack;
use crate::track::{AudioTrack, VideoTrack};

/// Output canvas size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const HD_1080: Self = Self {
        width: 1920,
        height: 1080,
    };
    pub const HD_720: Self = Self {
        width: 1280,
        height: 720,
    };
    /// 9:16 for §36's Shorts/TikTok/Reels presets.
    pub const VERTICAL_1080: Self = Self {
        width: 1080,
        height: 1920,
    };

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn aspect_ratio(self) -> f32 {
        if self.height == 0 {
            return 0.0;
        }
        self.width as f32 / self.height as f32
    }
}

/// Where a track sits: video tracks composite bottom-up, audio tracks sum, and
/// text tracks composite over everything (§22, §26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,

    pub resolution: Resolution,
    pub frame_rate: FrameRate,

    /// Adjustments applied to the finished picture rather than to any one clip.
    ///
    /// Lives on the sequence because that is what it adjusts: the assembled
    /// video, not a clip in it. Defaulted in serde, so projects written before
    /// it existed load with no adjustment at all.
    #[serde(default)]
    pub master: crate::clip::MasterLook,

    /// §20a.4's master gain: the whole video's volume, as a linear gain.
    ///
    /// Project data rather than a monitoring level, because §20a.4 puts it
    /// inside the mix graph whose order "must be defined, because preview and
    /// export must match" (§46). A volume that changed what the editor played
    /// and not what it exported would be exactly that mismatch. How loud the
    /// speakers are is the operating system's business.
    ///
    /// Beside `master` rather than inside it, because `MasterLook` decides
    /// whether the compositor runs a whole-picture pass at all, and turning the
    /// sound down is no reason to re-render the picture.
    #[serde(default = "unity")]
    pub master_volume: f32,

    /// Index 0 is the bottom layer. §22: "Track order determines compositing
    /// order." Later tracks composite over earlier ones.
    pub video_tracks: Vec<VideoTrack>,
    pub audio_tracks: Vec<AudioTrack>,

    /// Text overlays (§26), composited over every video track.
    ///
    /// A separate list rather than another entry in `video_tracks` because a
    /// text clip has no media behind it and none of the machinery that goes
    /// with media — proxies, filmstrips, decode-ahead — applies to it.
    /// Defaulted in serde, so projects written before §26 load unchanged.
    #[serde(default)]
    pub text_tracks: Vec<TextTrack>,

    /// Named instants, sorted by time, one per instant (`crate::marker`).
    /// Defaulted, so projects written before markers load with none.
    #[serde(default)]
    pub markers: Vec<crate::marker::Marker>,
}

fn unity() -> f32 {
    1.0
}

/// The loudest the whole video may be made: four times, about +12 dB. Past
/// that the limiter is doing all the work and the control has stopped meaning
/// anything.
pub const MAX_MASTER_VOLUME: f32 = 4.0;

impl Sequence {
    /// Create a sequence, rejecting a frame rate the timebase cannot represent.
    ///
    /// Refusing here rather than rounding is the §9 rule: if a rate does not
    /// divide 960,000 exactly, every frame boundary in the sequence would drift.
    pub fn new(
        name: impl Into<String>,
        resolution: Resolution,
        frame_rate: FrameRate,
    ) -> Result<Self, TimelineError> {
        if ticks_per_frame(frame_rate).is_none() {
            return Err(TimelineError::UnrepresentableFrameRate {
                rate: frame_rate.to_string(),
            });
        }
        Ok(Self {
            id: SequenceId::new(),
            name: name.into(),
            resolution,
            frame_rate,
            master: crate::clip::MasterLook::default(),
            master_volume: 1.0,
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            text_tracks: Vec::new(),
            markers: Vec::new(),
        })
    }

    /// A 1080p30 sequence with one video and one audio track — what "New
    /// Project" produces so the timeline is never empty on first launch.
    pub fn default_hd() -> Self {
        let mut sequence = Self::new("Sequence 1", Resolution::HD_1080, FrameRate::FPS_30)
            .unwrap_or_else(|_| unreachable!("30 fps divides the timebase"));
        sequence.video_tracks.push(VideoTrack::new("V1"));
        sequence.audio_tracks.push(AudioTrack::new("A1"));
        // One text lane from the start, for the same reason V1 and A1 are
        // there: something to drop a title onto without first having to work
        // out that a track is what is missing.
        sequence.text_tracks.push(TextTrack::new("T1"));
        sequence
    }

    /// Ticks in one frame of this sequence. Always `Some` for a constructed
    /// `Sequence`, because `new` rejects rates that cannot divide the timebase.
    pub fn ticks_per_frame(&self) -> i64 {
        ticks_per_frame(self.frame_rate).unwrap_or(1)
    }

    /// Snap a position to a frame boundary. §76 requires this before building a
    /// split command.
    pub fn snap_to_frame(&self, t: TimelineTime) -> TimelineTime {
        t.snap_to_frame(self.frame_rate).unwrap_or(t)
    }

    /// End of the last clip on any track.
    pub fn duration(&self) -> TimelineTime {
        let video = self.video_tracks.iter().map(|t| t.duration());
        let audio = self.audio_tracks.iter().map(|t| t.duration());
        let text = self.text_tracks.iter().map(|t| t.duration());
        video
            .chain(audio)
            .chain(text)
            .fold(TimelineTime::ZERO, TimelineTime::max)
    }

    pub fn text_track(&self, id: TrackId) -> Option<&TextTrack> {
        self.text_tracks.iter().find(|t| t.id == id)
    }

    pub fn text_track_mut(&mut self, id: TrackId) -> Option<&mut TextTrack> {
        self.text_tracks.iter_mut().find(|t| t.id == id)
    }

    /// The text clip with this id, wherever it is.
    pub fn text_clip(&self, id: ClipId) -> Option<&crate::text::TextClip> {
        self.text_tracks.iter().find_map(|t| t.get(id))
    }

    pub fn text_clip_mut(&mut self, id: ClipId) -> Option<&mut crate::text::TextClip> {
        self.text_tracks.iter_mut().find_map(|t| t.get_mut(id))
    }

    /// Which text track a clip is on.
    pub fn text_track_of(&self, clip: ClipId) -> Option<TrackId> {
        self.text_tracks
            .iter()
            .find(|t| t.get(clip).is_some())
            .map(|t| t.id)
    }

    pub fn video_track(&self, id: TrackId) -> Option<&VideoTrack> {
        self.video_tracks.iter().find(|t| t.id == id)
    }

    pub fn video_track_mut(&mut self, id: TrackId) -> Option<&mut VideoTrack> {
        self.video_tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn audio_track(&self, id: TrackId) -> Option<&AudioTrack> {
        self.audio_tracks.iter().find(|t| t.id == id)
    }

    pub fn audio_track_mut(&mut self, id: TrackId) -> Option<&mut AudioTrack> {
        self.audio_tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn track_kind(&self, id: TrackId) -> Option<TrackKind> {
        if self.video_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Video)
        } else if self.audio_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Audio)
        } else if self.text_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Text)
        } else {
            None
        }
    }

    pub fn track_count(&self) -> usize {
        self.video_tracks.len() + self.audio_tracks.len() + self.text_tracks.len()
    }

    pub fn clip_count(&self) -> usize {
        let video: usize = self.video_tracks.iter().map(|t| t.len()).sum();
        let audio: usize = self.audio_tracks.iter().map(|t| t.len()).sum();
        let text: usize = self.text_tracks.iter().map(|t| t.len()).sum();
        video + audio + text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::Rational;

    #[test]
    fn default_sequence_has_one_track_of_each_kind() {
        let s = Sequence::default_hd();
        assert_eq!(s.video_tracks.len(), 1);
        assert_eq!(s.audio_tracks.len(), 1);
        assert_eq!(s.resolution, Resolution::HD_1080);
        assert_eq!(s.ticks_per_frame(), 32_000);
    }

    #[test]
    fn every_supported_frame_rate_is_accepted() {
        for rate in FrameRate::SUPPORTED {
            assert!(
                Sequence::new("s", Resolution::HD_1080, rate).is_ok(),
                "rate {rate} rejected"
            );
        }
    }

    /// 44.1 kHz-style rates and other non-dividing rates must be refused rather
    /// than silently rounded (§9).
    #[test]
    fn a_rate_that_cannot_divide_the_timebase_is_refused() {
        let odd = FrameRate::new(7, 1).expect("positive");
        assert!(ticks_per_frame(odd).is_none());
        assert!(matches!(
            Sequence::new("s", Resolution::HD_1080, odd),
            Err(TimelineError::UnrepresentableFrameRate { .. })
        ));
    }

    #[test]
    fn snapping_uses_the_sequence_frame_rate() {
        let s = Sequence::new("s", Resolution::HD_1080, FrameRate::NTSC_29_97).expect("valid");
        let inside = TimelineTime::from_ticks(32_032 * 3 + 5);
        assert_eq!(s.snap_to_frame(inside).ticks(), 32_032 * 3);
    }

    #[test]
    fn empty_sequence_has_zero_duration() {
        assert_eq!(Sequence::default_hd().duration(), TimelineTime::ZERO);
    }

    #[test]
    fn resolution_aspect_ratio_is_safe_at_zero_height() {
        assert_eq!(Resolution::new(100, 0).aspect_ratio(), 0.0);
        assert!((Resolution::HD_1080.aspect_ratio() - 16.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn frame_rate_serializes_as_a_rational_not_a_float() {
        let s = Sequence::new("s", Resolution::HD_1080, FrameRate::NTSC_29_97).expect("valid");
        let json = serde_json::to_string(&s.frame_rate).expect("serialize");
        assert!(
            json.contains("30000"),
            "expected exact rational, got {json}"
        );
        assert!(json.contains("1001"), "expected exact rational, got {json}");

        let back: FrameRate = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.as_rational(),
            Rational::new(30_000, 1001).expect("valid")
        );
    }
}
