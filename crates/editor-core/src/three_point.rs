//! Three-point editing: a range marked on a file, a place on the timeline,
//! and one of two ways to put the first into the second.
//!
//! The oldest way of cutting, and still the fastest. Mark the part of the take
//! worth using in the browser, put the playhead where it belongs, and the
//! shot goes there — *overwrite* to replace what is under it, *insert* to push
//! everything along and make room. Three points decide the edit and the fourth
//! follows, which is why nobody has to type a length.
//!
//! # Which lanes
//!
//! The targeted ones (§10's track targeting, `crate::sync_lock`): the picture
//! lands on the targeted picture lane and its sound on the targeted sound
//! lane, together, linked (§12) — the same rule dropping a file on the
//! timeline follows.
//!
//! # What each one costs
//!
//! An insert opens the gap on *every* lane, as §10's insert does, so nothing
//! anywhere falls out of step. An overwrite clears the stretch on the lanes it
//! is landing on and leaves the rest of the timeline exactly where it is —
//! which is the point of it: the music keeps playing, and this shot replaces
//! that one.
//!
//! Both are one undo step, and both refuse before touching anything when
//! there is no lane of the right kind to land on.

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime, TrackId};

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// How a shot lands on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropKind {
    /// Replace what is under it; nothing else moves.
    Overwrite,
    /// Push everything from there along to make room.
    Insert,
}

impl DropKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overwrite => "Overwrite",
            Self::Insert => "Insert",
        }
    }
}

impl Editor {
    /// Where and how a file dragged onto the timeline at `at` goes, so that
    /// nothing already there is lost — CapCut's drop, which pushes clips
    /// aside rather than covering them:
    ///
    /// - inside a clip, it goes in at that clip's nearer edge, as an insert;
    /// - in a gap long enough on every lane it lands on, it goes down there
    ///   and nothing moves;
    /// - otherwise it goes in at `at`, pushing what follows along.
    pub fn drop_plan(
        &self,
        media: MediaId,
        at: TimelineTime,
    ) -> Result<(TimelineTime, DropKind), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project()
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let asset = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;
        let length = asset
            .placement_duration_with(self.project().settings.photo_length)
            .ticks()
            .max(1);
        let mut lanes = Vec::new();
        if asset.kind.has_video()
            && let Some(track) = sequence.target_track(bettercut_timeline::TrackKind::Video)
        {
            lanes.push(track);
        }
        if asset.audio_codec.is_some()
            && let Some(track) = sequence.target_track(bettercut_timeline::TrackKind::Audio)
        {
            lanes.push(track);
        }
        let spans: Vec<(i64, i64)> = sequence
            .clip_spans()
            .filter(|span| lanes.contains(&span.track))
            .map(|span| (span.timeline.start.ticks(), span.timeline.end.ticks()))
            .collect();
        let at = self
            .snap_to_frame(sequence_id, TimelineTime::from_ticks(at.ticks().max(0)))
            .ticks();

        // The clip under the drop on the picture lane (or the sound lane, for
        // a sound): in at its nearer edge.
        let main = lanes.first().copied();
        if let Some((start, end)) = sequence
            .clip_spans()
            .filter(|span| Some(span.track) == main)
            .map(|span| (span.timeline.start.ticks(), span.timeline.end.ticks()))
            .find(|(start, end)| *start < at && at < *end)
        {
            let edge = if at - start <= end - at { start } else { end };
            return Ok((TimelineTime::from_ticks(edge), DropKind::Insert));
        }
        let free = spans
            .iter()
            .all(|(start, end)| *end <= at || *start >= at + length);
        Ok((
            TimelineTime::from_ticks(at),
            if free {
                DropKind::Overwrite
            } else {
                DropKind::Insert
            },
        ))
    }

    /// Put `media` — or the `part` of it marked in the browser — down at `at`
    /// on the targeted lanes. Returns the clips it made.
    pub fn place_media_at(
        &mut self,
        media: MediaId,
        part: Option<(MediaTime, MediaTime)>,
        at: TimelineTime,
        kind: DropKind,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self.snap_to_frame(sequence, TimelineTime::from_ticks(at.ticks().max(0)));

        let asset = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;
        let wants_video = asset.kind.has_video();
        let wants_audio = asset.audio_codec.is_some();
        let duration = asset.placement_duration_with(self.project().settings.photo_length);
        let still = asset.is_still();

        let active = self
            .project()
            .sequence(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let video_track = active
            .target_track(bettercut_timeline::TrackKind::Video)
            .filter(|_| wants_video);
        let audio_track = active
            .target_track(bettercut_timeline::TrackKind::Audio)
            .filter(|_| wants_audio);
        if video_track.is_none() && audio_track.is_none() {
            return Err(EditorError::NoTrackForMedia);
        }

        // The stretch of the file to play: the marked part, held inside it.
        let source = match part.filter(|_| !still) {
            None => bettercut_timeline::SourceRange::new(MediaTime::ZERO, duration)?,
            Some((from, to)) => {
                let (from, to) = if to < from { (to, from) } else { (from, to) };
                let limit = duration.ticks().max(1);
                let from = from.ticks().clamp(0, limit - 1);
                let to = to.ticks().clamp(from + 1, limit);
                bettercut_timeline::SourceRange::new(
                    MediaTime::from_ticks(from),
                    MediaTime::from_ticks(to),
                )?
            }
        };
        let length = TimelineTime::from_ticks(source.duration().ticks().max(1));
        let end = TimelineTime::from_ticks(at.ticks() + length.ticks());

        // Only when both halves are going down: a link to nothing is a lie
        // that later edits would have to keep checking (§12).
        let link = (video_track.is_some() && audio_track.is_some())
            .then(bettercut_foundation::LinkId::new);
        let mut placed = Vec::new();
        let mut adds = Vec::new();
        if let Some(track) = video_track {
            let mut clip = bettercut_timeline::VideoClip::new(media, at, source)?;
            clip.link = link;
            placed.push(clip.id);
            adds.push((track, ClipPayload::Video(Box::new(clip))));
        }
        if let Some(track) = audio_track {
            let mut clip = bettercut_timeline::AudioClip::new(media, at, source)?;
            clip.link = link;
            placed.push(clip.id);
            adds.push((track, ClipPayload::Audio(Box::new(clip))));
        }

        let lanes: Vec<TrackId> = adds.iter().map(|(track, _)| *track).collect();
        self.staged(kind.label(), |editor, stage| {
            match kind {
                // Room across every lane, so the sound and the titles after it
                // keep their place against the picture.
                DropKind::Insert => {
                    editor.stage_insert_time(stage, sequence, at, length)?;
                }
                // Just these lanes, and nothing else moves.
                DropKind::Overwrite => {
                    editor.stage_clear_span(stage, sequence, &lanes, at, end)?;
                }
            }
            for (track, clip) in adds {
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence,
                        track,
                        clip,
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(placed)
    }

    /// Clear `start`..`end` on `lanes`: clean cuts at both ends, and what is
    /// inside goes. Nothing moves — this is the hole an overwrite lands in.
    ///
    /// Only these lanes are cut, and a cut clip's linked partner is left
    /// alone — unlike a split on the timeline (§12), which cuts the pair.
    ///
    /// An overwrite on the sound lane is not a reason to put two extra cuts in
    /// a picture nobody touched. The halves keep the link they had, so the
    /// picture still moves its sound: what is left of a clip is still that
    /// clip's sound.
    pub(crate) fn stage_clear_span(
        &mut self,
        stage: &mut crate::editor::Stage,
        sequence: bettercut_foundation::SequenceId,
        lanes: &[TrackId],
        start: TimelineTime,
        end: TimelineTime,
    ) -> Result<usize, EditorError> {
        for at in [end, start] {
            let straddling: Vec<(TrackId, ClipId)> = self
                .clips_from(at, true)
                .into_iter()
                .filter(|(track, _, _)| lanes.contains(track))
                .map(|(track, clip, _)| (track, clip))
                .collect();
            for (track, clip) in straddling {
                self.stage(
                    stage,
                    Command::SplitClip {
                        sequence,
                        track,
                        clip,
                        at,
                        left: ClipId::new(),
                        right: ClipId::new(),
                        relink: None,
                    },
                )?;
            }
        }

        let inside: Vec<(TrackId, ClipId)> = self
            .project()
            .active()
            .map(|active| {
                active
                    .clip_spans()
                    .filter(|span| {
                        lanes.contains(&span.track)
                            && span.timeline.start >= start
                            && span.timeline.end <= end
                    })
                    .map(|span| (span.track, span.clip))
                    .collect()
            })
            .unwrap_or_default();
        let removed = inside.len();
        for (track, clip) in inside {
            self.stage(
                stage,
                Command::RemoveClip {
                    sequence,
                    track,
                    clip,
                },
            )?;
        }
        Ok(removed)
    }
}
