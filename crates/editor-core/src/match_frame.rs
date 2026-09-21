//! Match frame: from a frame on the timeline back to the file it came from.
//!
//! "Where is this shot from, and what else is on that take?" is a question
//! asked a dozen times in a cut — to find a better moment, to check what
//! happened either side, to cut the same shot in again somewhere else. The
//! answer is a file and an instant in it, and finding them by hand means
//! reading a clip's name off the timeline and scrubbing a thumbnail until the
//! pictures line up.
//!
//! # Which clip is matched
//!
//! The one being *looked at*: the topmost picture the playhead is inside, on a
//! lane that is actually on screen (§8's hidden lanes and §20a.4's solo both
//! apply) — the same rule the burn-in uses to name the file it prints. A
//! selected clip wins over that when the playhead is inside it, because a
//! selection is the user saying which clip they mean. With no picture under
//! the playhead the sound is matched instead, since a sound clip is as much a
//! piece of a file as a shot is.

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_timeline::SourceRange;

use crate::editor::Editor;

/// A frame on the timeline, as a place in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchedFrame {
    /// The clip the frame was read from.
    pub clip: ClipId,
    pub media: MediaId,
    /// Where in the file the playhead is — through the clip's speed and its
    /// direction, so a reversed clip at 2× answers with the frame actually on
    /// screen.
    pub at: MediaTime,
    /// The stretch of the file this clip plays, which is what the browser
    /// shows as its marked part.
    pub source: SourceRange,
}

impl Editor {
    /// The file and instant behind the frame under the playhead.
    ///
    /// `selected` is the interface's selection, which lives there rather than
    /// here (§56): one selected clip the playhead is inside is the user saying
    /// which clip they mean, and wins over whatever is topmost.
    ///
    /// `None` when nothing is under the playhead, or when what is there was
    /// made rather than filmed — a colour clip has no file to match to.
    pub fn match_frame(&self, selected: &[ClipId]) -> Option<MatchedFrame> {
        let at = self.playhead();
        let selected = self
            .selected_clip_under(selected, at)
            .and_then(|clip| self.match_frame_of(clip));
        selected.or_else(|| {
            self.visible_clip_at(at)
                .and_then(|clip| self.match_frame_of(clip))
        })
    }

    /// The same for a clip named outright — a right-click on it, wherever the
    /// playhead is. The instant is where the playhead falls inside the clip,
    /// or its in-point when the playhead is elsewhere.
    pub fn match_frame_of(&self, clip: ClipId) -> Option<MatchedFrame> {
        let playhead = self.playhead();
        let (media, source, at) = if let Some(video) = self.video_clip(clip) {
            let inside = video.timeline.contains(playhead);
            (
                video.media_id,
                video.source,
                if inside {
                    video.source_time_at(playhead)
                } else {
                    video.source.start
                },
            )
        } else {
            let audio = self.audio_clip(clip)?;
            let inside = audio.timeline.contains(playhead);
            (
                audio.media_id,
                audio.source,
                if inside {
                    audio.source_time_at(playhead)
                } else {
                    audio.source.start
                },
            )
        };

        // A colour, a compound or anything else generated has no file behind
        // it, so there is nothing to show in the browser.
        let asset = self.project().media_asset(media)?;
        if asset.generated.is_some() {
            return None;
        }
        Some(MatchedFrame {
            clip,
            media,
            at: MediaTime::from_ticks(at.ticks().clamp(source.start.ticks(), source.end.ticks())),
            source,
        })
    }

    /// The one selected clip the playhead is inside, if there is exactly one.
    fn selected_clip_under(&self, selected: &[ClipId], at: TimelineTime) -> Option<ClipId> {
        let sequence = self.project().active()?;
        let mut under = selected
            .iter()
            .copied()
            .filter(|clip| {
                sequence
                    .clip_span(*clip)
                    .is_some_and(|span| span.timeline.contains(at))
            })
            .take(2);
        let first = under.next()?;
        under.next().is_none().then_some(first)
    }

    /// The topmost picture on screen at `at`, or the topmost sound when there
    /// is no picture there.
    fn visible_clip_at(&self, at: TimelineTime) -> Option<ClipId> {
        let sequence = self.project().active()?;
        let picture_soloed = sequence.video_tracks.iter().any(|track| track.solo);
        let picture = sequence
            .video_tracks
            .iter()
            .rev()
            .filter(|track| {
                bettercut_timeline::track_plays(track.enabled, track.solo, picture_soloed)
            })
            .find_map(|track| track.clip_at(at).map(|clip| clip.id));
        picture.or_else(|| {
            let sound_soloed = sequence.audio_tracks.iter().any(|track| track.solo);
            sequence
                .audio_tracks
                .iter()
                .filter(|track| {
                    bettercut_timeline::track_plays(track.enabled, track.solo, sound_soloed)
                })
                .find_map(|track| track.clip_at(at).map(|clip| clip.id))
        })
    }
}
