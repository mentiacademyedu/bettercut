//! Attaching a clip to something that moves.
//!
//! The following itself is [`bettercut_playback::tracker`]; this is what turns
//! the path it found into keyframes on the clip that is to follow it.
//!
//! The clip keeps whatever offset it had from the thing being tracked at the
//! instant tracking began, so a sticker put *beside* a face stays beside it
//! rather than jumping on top of it.

use bettercut_foundation::{ClipId, MediaTime, TICKS_PER_SECOND, TimelineTime, TrackId};
use bettercut_timeline::{AnimatedParameter, Interpolation, Keyframe};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// One point of a followed path: where the patch was, in 0–1 frame units, at
/// an instant on the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedPoint {
    pub at: TimelineTime,
    pub center: [f32; 2],
}

impl Editor {
    /// Where `clip` sits in 0–1 frame units — the box a tracker should follow
    /// when the user has put the clip over the thing to follow.
    ///
    /// Its middle, and half its size, both as shares of the frame.
    pub fn clip_box(&self, clip: ClipId) -> Option<([f32; 2], [f32; 2])> {
        let video = self.video_clip(clip)?;
        let transform = video.transform;
        // A clip fills the frame at scale 1 and is offset from the middle in
        // frame units, which is exactly what the tracker wants — only moved
        // from "offset from the centre" to "where in the picture".
        let center = [0.5 + transform.position.x, 0.5 - transform.position.y];
        let half = [
            (transform.scale.x * 0.5).abs().clamp(0.01, 0.5),
            (transform.scale.y * 0.5).abs().clamp(0.01, 0.5),
        ];
        Some((center, half))
    }

    /// Make `clip` follow `path`: a position keyframe per point, keeping the
    /// offset the clip had from the first point.
    ///
    /// One undo step, and the keys replace any the clip's position already
    /// had — a second attempt at a track should leave one track behind, not
    /// two fighting each other.
    pub fn attach_to_path(
        &mut self,
        clip: ClipId,
        path: &[TrackedPoint],
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track: TrackId = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let Some(first) = path.first() else {
            return Err(EditorError::NothingTracked);
        };
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        // What the clip was doing relative to the thing being followed when
        // the track began.
        let offset = [
            video.transform.position.x - (first.center[0] - 0.5),
            video.transform.position.y - (0.5 - first.center[1]),
        ];

        let mut commands: Vec<Command> = Vec::new();
        // Off with the old: a track is a new answer for the whole span, not
        // an addition to whatever was there.
        for parameter in [AnimatedParameter::PositionX, AnimatedParameter::PositionY] {
            for key in self.keyframes_of(clip, parameter) {
                commands.push(Command::RemoveKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    time: key.time,
                });
            }
        }
        let mut written = 0;
        for point in path {
            // Keys live in source time (§24), so each instant on the timeline
            // is asked of the clip that is following.
            let at = video.progress_time_at(point.at);
            if at < video.source.start || at > video.source.end {
                continue; // the track ran past the clip that follows it
            }
            let x = point.center[0] - 0.5 + offset[0];
            let y = 0.5 - point.center[1] + offset[1];
            for (parameter, value) in [
                (AnimatedParameter::PositionX, x),
                (AnimatedParameter::PositionY, y),
            ] {
                commands.push(Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    key: Keyframe::new(at, parameter.clamp(value), Interpolation::Linear),
                });
            }
            written += 1;
        }
        if written == 0 {
            return Err(EditorError::NothingTracked);
        }
        self.dispatch_group("Track Motion".to_owned(), commands)?;
        Ok(written)
    }

    /// How long a track may run before it is cut short, so one click cannot
    /// ask for ten minutes of decoding.
    pub const MAX_TRACK_SECONDS: i64 = 10;

    /// The span a track should cover: from the playhead to the end of the two
    /// clips' overlap, and never more than [`Self::MAX_TRACK_SECONDS`].
    pub fn track_span(
        &self,
        follower: ClipId,
        footage: ClipId,
    ) -> Option<(TimelineTime, TimelineTime)> {
        let a = self.video_clip(follower)?.timeline;
        let b = self.video_clip(footage)?.timeline;
        let start = self.playhead().max(a.start).max(b.start);
        let end = a.end.min(b.end);
        if end <= start {
            return None;
        }
        let limit =
            TimelineTime::from_ticks(start.ticks() + Self::MAX_TRACK_SECONDS * TICKS_PER_SECOND);
        Some((start, end.min(limit)))
    }

    /// The instant in `footage`'s own file that the timeline is at.
    pub fn source_time_of(&self, clip: ClipId, at: TimelineTime) -> Option<MediaTime> {
        Some(self.video_clip(clip)?.source_time_at(at))
    }
}
