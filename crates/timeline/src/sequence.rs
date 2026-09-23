//! Sequences: a canvas, a frame rate, and stacks of tracks (§8).

use bettercut_foundation::{ClipId, FrameRate, SequenceId, TimelineTime, TrackId, ticks_per_frame};
use serde::{Deserialize, Serialize};

use crate::clip::TimelineRange;
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

/// One clip as [`Sequence::clip_spans`] reports it: enough to find it, cut it
/// and move it, without knowing what kind of clip it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipSpan {
    pub track: TrackId,
    pub kind: TrackKind,
    pub clip: ClipId,
    pub timeline: TimelineRange,
    /// The picture or sound it is tied to (§12). `None` for a title or an
    /// adjustment, which have nothing to be tied to.
    pub link: Option<bettercut_foundation::LinkId>,
}

/// A mark on a clip rather than on the sequence: kept at an offset into
/// the clip's *source*, so it travels when the clip is moved and stays on
/// the same frame of the footage when the clip is trimmed or slipped.
///
/// The sequence's own markers mark the edit — a beat of the music, a place
/// to cut. These mark the footage — the moment the door opens, the take
/// that was good — and the difference shows the instant a clip moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipMark {
    pub clip: ClipId,
    /// Where in the file it falls, not where on the timeline.
    pub source: bettercut_foundation::MediaTime,
    /// What it marks. Empty is a plain mark, as on the ruler.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub color: crate::clip::ColorLabel,
}

/// A note left on a clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipNote {
    pub clip: ClipId,
    pub text: String,
}

/// Where a track sits: video tracks composite bottom-up, audio tracks sum,
/// adjustment lanes grade the pictures beneath them, and text tracks composite
/// over everything (§22, §26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    /// `crate::adjustment`: a grade over a stretch of the edit.
    Adjustment,
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

    /// Adjustment lanes (`crate::adjustment`): grades over a stretch of the
    /// edit, applied to every picture beneath them and none of the titles.
    ///
    /// Composited between the two lists either side of it here — over the
    /// video tracks, under the text — which is where a filter belongs.
    /// Defaulted, so projects written before adjustments load with none.
    #[serde(default)]
    pub adjustment_tracks: Vec<crate::adjustment::AdjustmentTrack>,

    /// Named instants, sorted by time, one per instant (`crate::marker`).
    /// Defaulted, so projects written before markers load with none.
    #[serde(default)]
    pub markers: Vec<crate::marker::Marker>,

    /// Bars that jump with a sound, over the whole picture. Defaulted, so
    /// older projects have none.
    #[serde(default)]
    pub visualizer: Option<crate::visualizer::Visualizer>,

    /// A logo in a corner of every frame. Defaulted, so older projects have
    /// none.
    #[serde(default)]
    pub watermark: Option<crate::watermark::Watermark>,

    /// The frame chosen as the video's cover, saved as a picture beside its
    /// exports. Defaulted, so older projects have none.
    #[serde(default)]
    pub cover_frame: Option<bettercut_foundation::TimelineTime>,

    /// Clips grouped to move together, each group a list of clip ids. A clip
    /// is in at most one. Ids of clips since deleted or split are ignored where
    /// groups are read, not cleaned out here. Defaulted: older projects have
    /// none.
    #[serde(default)]
    pub groups: Vec<Vec<bettercut_foundation::ClipId>>,

    /// Short notes left on clips — "swap for take 3", "check the audio here".
    /// At most one a clip; a note for a clip since deleted is ignored where
    /// notes are read. Defaulted: older projects have none.
    #[serde(default)]
    pub notes: Vec<ClipNote>,

    /// Marks that belong to clips rather than to the edit (`ClipMark`).
    /// Defaulted: older projects have none.
    #[serde(default)]
    pub clip_marks: Vec<ClipMark>,

    /// The in and out marks: a stretch of the sequence picked out to export on
    /// its own. Either may be unset; the range exists when both are, in order.
    /// Stretches of this sequence already rendered to a file
    /// (`crate::render`). Defaulted: older projects have none.
    #[serde(default)]
    pub renders: Vec<crate::render::RenderedRange>,

    #[serde(default)]
    pub mark_in: Option<TimelineTime>,
    #[serde(default)]
    pub mark_out: Option<TimelineTime>,

    /// What the first frame is called: the timecode the ruler, the readouts
    /// and a burn-in count from. Zero for most edits; an hour for a
    /// programme delivered the broadcast way. Positions stay from zero
    /// inside (§9) — this is only what they are *called*. Defaulted: older
    /// projects start at zero.
    #[serde(default)]
    pub start_timecode: TimelineTime,

    /// Clips soloed on their own: while any picture clip is here, only the
    /// picture clips here are seen; while any sound clip is here, only those
    /// are heard. §20a.4's lane solo, for one clip — "let me see just this
    /// one" without muting a lane at a time. The id of a clip since deleted
    /// is ignored rather than tidied, so a solo survives an undo of the
    /// delete. Defaulted: older projects have nothing soloed.
    #[serde(default)]
    pub soloed_clips: Vec<ClipId>,
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
            adjustment_tracks: Vec::new(),
            markers: Vec::new(),
            visualizer: None,
            watermark: None,
            cover_frame: None,
            groups: Vec::new(),
            notes: Vec::new(),
            clip_marks: Vec::new(),
            renders: Vec::new(),
            mark_in: None,
            mark_out: None,
            start_timecode: TimelineTime::ZERO,
            soloed_clips: Vec::new(),
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

    /// The span between the in and out marks, when both are set and the in
    /// comes first.
    pub fn marked_range(&self) -> Option<TimelineRange> {
        match (self.mark_in, self.mark_out) {
            (Some(start), Some(end)) if start < end => Some(TimelineRange { start, end }),
            _ => None,
        }
    }

    /// End of the last clip on any track.
    pub fn duration(&self) -> TimelineTime {
        let video = self.video_tracks.iter().map(|t| t.duration());
        let audio = self.audio_tracks.iter().map(|t| t.duration());
        let text = self.text_tracks.iter().map(|t| t.duration());
        // An adjustment running past the last clip still has to be exported,
        // or a fade to a graded black at the end would be cut off.
        let adjustments = self.adjustment_tracks.iter().map(|t| t.duration());
        video
            .chain(audio)
            .chain(text)
            .chain(adjustments)
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

    pub fn adjustment_track(&self, id: TrackId) -> Option<&crate::adjustment::AdjustmentTrack> {
        self.adjustment_tracks.iter().find(|t| t.id == id)
    }

    pub fn adjustment_track_mut(
        &mut self,
        id: TrackId,
    ) -> Option<&mut crate::adjustment::AdjustmentTrack> {
        self.adjustment_tracks.iter_mut().find(|t| t.id == id)
    }

    /// The adjustment clip with this id, wherever it is.
    pub fn adjustment_clip(&self, id: ClipId) -> Option<&crate::adjustment::AdjustmentClip> {
        self.adjustment_tracks.iter().find_map(|t| t.get(id))
    }

    pub fn adjustment_clip_mut(
        &mut self,
        id: ClipId,
    ) -> Option<&mut crate::adjustment::AdjustmentClip> {
        self.adjustment_tracks
            .iter_mut()
            .find_map(|t| t.get_mut(id))
    }

    /// Which adjustment lane a clip is on.
    pub fn adjustment_track_of(&self, clip: ClipId) -> Option<TrackId> {
        self.adjustment_tracks
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

    /// A track's name, whatever kind of lane it is.
    pub fn track_name(&self, id: TrackId) -> Option<&str> {
        self.video_tracks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.as_str())
            .or_else(|| {
                self.audio_tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.name.as_str())
            })
            .or_else(|| {
                self.text_tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.name.as_str())
            })
            .or_else(|| {
                self.adjustment_tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.name.as_str())
            })
    }

    /// The same, to change.
    pub fn track_name_mut(&mut self, id: TrackId) -> Option<&mut String> {
        if let Some(t) = self.video_tracks.iter_mut().find(|t| t.id == id) {
            return Some(&mut t.name);
        }
        if let Some(t) = self.audio_tracks.iter_mut().find(|t| t.id == id) {
            return Some(&mut t.name);
        }
        if let Some(t) = self.text_tracks.iter_mut().find(|t| t.id == id) {
            return Some(&mut t.name);
        }
        self.adjustment_tracks
            .iter_mut()
            .find(|t| t.id == id)
            .map(|t| &mut t.name)
    }

    pub fn track_kind(&self, id: TrackId) -> Option<TrackKind> {
        if self.video_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Video)
        } else if self.audio_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Audio)
        } else if self.text_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Text)
        } else if self.adjustment_tracks.iter().any(|t| t.id == id) {
            Some(TrackKind::Adjustment)
        } else {
            None
        }
    }

    /// The bake covering `position`, if one does — whether or not it is
    /// still current; `bettercut_playback::rendered` decides that.
    pub fn render_at(&self, position: TimelineTime) -> Option<&crate::render::RenderedRange> {
        self.renders.iter().find(|render| render.covers(position))
    }

    /// Every track of `kind`, in the order they are stacked.
    pub fn tracks_of(&self, kind: TrackKind) -> Vec<TrackId> {
        match kind {
            TrackKind::Video => self.video_tracks.iter().map(|t| t.id).collect(),
            TrackKind::Audio => self.audio_tracks.iter().map(|t| t.id).collect(),
            TrackKind::Text => self.text_tracks.iter().map(|t| t.id).collect(),
            TrackKind::Adjustment => self.adjustment_tracks.iter().map(|t| t.id).collect(),
        }
    }

    /// Whether `track` is the one new clips of its kind land on.
    pub fn track_targeted(&self, track: TrackId) -> bool {
        self.video_tracks
            .iter()
            .find(|t| t.id == track)
            .map(|t| t.targeted)
            .or_else(|| {
                self.audio_tracks
                    .iter()
                    .find(|t| t.id == track)
                    .map(|t| t.targeted)
            })
            .or_else(|| {
                self.text_tracks
                    .iter()
                    .find(|t| t.id == track)
                    .map(|t| t.targeted)
            })
            .or_else(|| {
                self.adjustment_tracks
                    .iter()
                    .find(|t| t.id == track)
                    .map(|t| t.targeted)
            })
            .unwrap_or(false)
    }

    /// The track of `kind` flagged as the target, if one is.
    ///
    /// The first flagged, of however many are: the editor keeps it to one, and
    /// a project hand-edited into having two should still answer the question.
    pub fn targeted_track(&self, kind: TrackKind) -> Option<TrackId> {
        self.tracks_of(kind)
            .into_iter()
            .find(|track| self.track_targeted(*track))
    }

    /// The track a new clip of `kind` lands on: the targeted one, or the first
    /// of its kind (§10's track targeting).
    pub fn target_track(&self, kind: TrackKind) -> Option<TrackId> {
        self.targeted_track(kind).or_else(|| match kind {
            TrackKind::Video => self.video_tracks.first().map(|t| t.id),
            TrackKind::Audio => self.audio_tracks.first().map(|t| t.id),
            TrackKind::Text => self.text_tracks.first().map(|t| t.id),
            TrackKind::Adjustment => self.adjustment_tracks.first().map(|t| t.id),
        })
    }

    /// Every track that rides along with a ripple edit made somewhere else
    /// (§10's sync lock), in stacking order.
    pub fn sync_locked_tracks(&self) -> Vec<TrackId> {
        let video = self
            .video_tracks
            .iter()
            .filter(|t| t.sync_lock)
            .map(|t| t.id);
        let audio = self
            .audio_tracks
            .iter()
            .filter(|t| t.sync_lock)
            .map(|t| t.id);
        let text = self
            .text_tracks
            .iter()
            .filter(|t| t.sync_lock)
            .map(|t| t.id);
        let adjustment = self
            .adjustment_tracks
            .iter()
            .filter(|t| t.sync_lock)
            .map(|t| t.id);
        video.chain(audio).chain(text).chain(adjustment).collect()
    }

    /// Every clip on every lane, in compositing order: video, audio, text,
    /// adjustments.
    ///
    /// **The one list of lanes.** Finding a clip by id, finding what covers an
    /// instant, and finding what a cut falls inside each walked the lanes
    /// themselves, one copy per question — and a lane kind added later had to be
    /// remembered in every copy. Titles were missed in one once; adjustments
    /// were missed in five, each reading as "not found" for a clip that was
    /// plainly there, with nothing from the compiler. Asked here, a new kind of
    /// lane is added once.
    pub fn clip_spans(&self) -> impl Iterator<Item = ClipSpan> + '_ {
        let video = self.video_tracks.iter().flat_map(|track| {
            track.clips().iter().map(move |clip| ClipSpan {
                track: track.id,
                kind: TrackKind::Video,
                clip: clip.id,
                timeline: clip.timeline,
                link: clip.link,
            })
        });
        let audio = self.audio_tracks.iter().flat_map(|track| {
            track.clips().iter().map(move |clip| ClipSpan {
                track: track.id,
                kind: TrackKind::Audio,
                clip: clip.id,
                timeline: clip.timeline,
                link: clip.link,
            })
        });
        let text = self.text_tracks.iter().flat_map(|track| {
            track.clips().iter().map(move |clip| ClipSpan {
                track: track.id,
                kind: TrackKind::Text,
                clip: clip.id,
                timeline: clip.timeline,
                link: None,
            })
        });
        let adjustments = self.adjustment_tracks.iter().flat_map(|track| {
            track.clips().iter().map(move |clip| ClipSpan {
                track: track.id,
                kind: TrackKind::Adjustment,
                clip: clip.id,
                timeline: clip.timeline,
                link: None,
            })
        });
        video.chain(audio).chain(text).chain(adjustments)
    }

    /// Where a clip is and what it spans, whichever lane it is on.
    pub fn clip_span(&self, clip: ClipId) -> Option<ClipSpan> {
        self.clip_spans().find(|span| span.clip == clip)
    }

    // ---- clip marks ----

    /// Where a clip's marks fall on the timeline *now*: its source offsets
    /// mapped through where the clip sits and how fast it plays. A mark
    /// outside what the clip currently shows is left out — trimming past a
    /// mark hides it rather than losing it, and trimming back brings it
    /// back.
    pub fn clip_marks_on_timeline(&self, clip: ClipId) -> Vec<(TimelineTime, &ClipMark)> {
        let Some(span) = self.clip_span(clip) else {
            return Vec::new();
        };
        let source = self.clip_source(clip);
        let Some(source) = source else {
            return Vec::new();
        };
        self.clip_marks
            .iter()
            .filter(|mark| mark.clip == clip)
            .filter_map(|mark| {
                if mark.source < source.start || mark.source >= source.end {
                    return None;
                }
                // Linear within the clip: the source span maps onto the
                // timeline span, whatever speed that works out to.
                let into = (mark.source - source.start).ticks();
                let source_len = source.duration().ticks().max(1);
                let timeline_len = span.timeline.duration().ticks();
                let at = span.timeline.start.ticks() + into * timeline_len / source_len;
                Some((TimelineTime::from_ticks(at), mark))
            })
            .collect()
    }

    /// A clip's source range, whatever kind of lane it is on.
    fn clip_source(&self, clip: ClipId) -> Option<crate::clip::SourceRange> {
        use crate::clip::Clip;
        self.video_tracks
            .iter()
            .find_map(|t| t.get(clip).map(Clip::source))
            .or_else(|| {
                self.audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).map(Clip::source))
            })
    }

    // ---- clip solo ----

    /// Whether `clip` is soloed.
    pub fn clip_soloed(&self, clip: ClipId) -> bool {
        self.soloed_clips.contains(&clip)
    }

    /// Solo or unsolo `clip`. Returns whether it was soloed before.
    pub fn set_clip_solo(&mut self, clip: ClipId, solo: bool) -> bool {
        let was = self.clip_soloed(clip);
        if solo && !was {
            self.soloed_clips.push(clip);
        } else if !solo && was {
            self.soloed_clips.retain(|id| *id != clip);
        }
        was
    }

    /// The soloed clips that are still on the timeline, by lane kind.
    fn soloed_kinds(&self) -> impl Iterator<Item = TrackKind> + '_ {
        self.soloed_clips
            .iter()
            .filter_map(|id| self.clip_span(*id).map(|span| span.kind))
    }

    /// Whether any picture clip — video, title or adjustment — is soloed.
    pub fn picture_clip_soloed(&self) -> bool {
        self.soloed_kinds().any(|kind| kind != TrackKind::Audio)
    }

    /// Whether any sound clip is soloed.
    pub fn sound_clip_soloed(&self) -> bool {
        self.soloed_kinds().any(|kind| kind == TrackKind::Audio)
    }

    /// Whether `clip`, `enabled` or not, is seen or heard given the solos
    /// in its kind of lane (`any_soloed` from [`Self::picture_clip_soloed`]
    /// or [`Self::sound_clip_soloed`]). The same rule as a lane's
    /// (`track_plays`): solo wins over mute, both ways round.
    pub fn clip_plays(&self, clip: ClipId, enabled: bool, any_soloed: bool) -> bool {
        crate::track::track_plays(enabled, self.clip_soloed(clip), any_soloed)
    }

    pub fn track_count(&self) -> usize {
        self.video_tracks.len()
            + self.audio_tracks.len()
            + self.text_tracks.len()
            + self.adjustment_tracks.len()
    }

    pub fn clip_count(&self) -> usize {
        let video: usize = self.video_tracks.iter().map(|t| t.len()).sum();
        let audio: usize = self.audio_tracks.iter().map(|t| t.len()).sum();
        let text: usize = self.text_tracks.iter().map(|t| t.len()).sum();
        let adjustments: usize = self.adjustment_tracks.iter().map(|t| t.len()).sum();
        video + audio + text + adjustments
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
