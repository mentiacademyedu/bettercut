//! Compound clips: several clips folded into a sequence of their own and
//! played on the timeline as one.
//!
//! What it is for: a title sequence, a three-shot montage, an insert with its
//! own sound — anything that has been finished and should now move, fade and
//! be graded as a single thing. Made rather than copied, so opening it later
//! and changing what is inside changes every place it plays.
//!
//! How it works: the clips are moved into a new sequence, and a generated clip
//! ([`bettercut_media::Generated::Compound`]) is left in their place. Nothing
//! decodes that clip — the player and the export expand it into the sequence's
//! own layers and sound (`bettercut_playback::compound`), which is what keeps
//! one copy of the contents rather than a rendered duplicate.

use bettercut_foundation::{ClipId, MediaTime, SequenceId, TimelineTime, TrackId};
use bettercut_media::{Generated, MediaAsset};
use bettercut_timeline::{AudioTrack, Clip, SourceRange, VideoClip, VideoTrack};

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// The sequence `clip` plays, if it is a compound clip.
    pub fn compound_of(&self, clip: ClipId) -> Option<SequenceId> {
        let media = self.video_clip(clip)?.media_id;
        self.project().media_asset(media)?.generated?.compound()
    }

    /// Fold `clips` into a sequence of their own and leave one compound clip
    /// in their place. Returns the clip left behind.
    ///
    /// The clips keep their positions relative to each other: the earliest
    /// starts the inner sequence, and the compound covers from there to the
    /// last one's end. Picture goes on picture lanes and sound on sound lanes,
    /// in the order the lanes had, so a two-layer montage stays two layers.
    ///
    /// One undo step for the whole thing (§79).
    pub fn make_compound(&mut self, clips: &[ClipId], name: &str) -> Result<ClipId, EditorError> {
        let parent_id = self.active_sequence_id()?;
        // A shot's own sound comes with it (§12), asked for or not: leaving it
        // behind would put the sound of what is now inside the compound over
        // whatever plays next.
        let mut wanted: Vec<ClipId> = Vec::new();
        for clip in clips {
            for member in self.linked_with(*clip) {
                if !wanted.contains(&member) {
                    wanted.push(member);
                }
            }
            if !wanted.contains(clip) {
                wanted.push(*clip);
            }
        }
        let parent = self
            .project()
            .sequence(parent_id)
            .ok_or(EditorError::SequenceNotFound(parent_id))?;

        // Everything asked for that is a picture or sound clip, with where it
        // sits. Titles are left where they are: a compound holds the layers it
        // can move as one, and a title track is shared with the rest of the
        // edit.
        let mut members: Vec<(TrackId, ClipId, bool, usize)> = Vec::new();
        for clip in &wanted {
            let Some(span) = parent.clip_span(*clip) else {
                continue;
            };
            let video = parent
                .video_tracks
                .iter()
                .position(|track| track.id == span.track);
            let audio = parent
                .audio_tracks
                .iter()
                .position(|track| track.id == span.track);
            match (video, audio) {
                (Some(lane), _) => members.push((span.track, *clip, true, lane)),
                (_, Some(lane)) => members.push((span.track, *clip, false, lane)),
                _ => {}
            }
        }
        if members.is_empty() {
            return Err(EditorError::NothingToCompound);
        }
        let spans: Vec<_> = members
            .iter()
            .filter_map(|(_, clip, ..)| parent.clip_span(*clip).map(|span| span.timeline))
            .collect();
        let (Some(first), Some(last)) = (
            spans.iter().map(|s| s.start).min(),
            spans.iter().map(|s| s.end).max(),
        ) else {
            return Err(EditorError::NothingToCompound);
        };
        let length = last - first;
        // Where the compound goes: the lane the earliest picture was on, or
        // the first picture lane when only sound was folded in.
        let home = members
            .iter()
            .filter(|(_, _, video, _)| *video)
            .min_by_key(|(_, clip, ..)| {
                parent
                    .clip_span(*clip)
                    .map_or(i64::MAX, |span| span.timeline.start.ticks())
            })
            .map(|(track, ..)| *track)
            .or_else(|| parent.video_tracks.first().map(|track| track.id))
            .ok_or(EditorError::NoTrackForMedia)?;

        // The inner sequence: the parent's shape, and enough lanes of each
        // kind to hold what is going in.
        let video_lanes = members
            .iter()
            .filter(|(_, _, video, _)| *video)
            .map(|(_, _, _, lane)| *lane + 1)
            .max()
            .unwrap_or(0)
            .max(1);
        let audio_lanes = members
            .iter()
            .filter(|(_, _, video, _)| !*video)
            .map(|(_, _, _, lane)| *lane + 1)
            .max()
            .unwrap_or(0);
        let taken: Vec<String> = self
            .project()
            .sequences
            .iter()
            .map(|s| s.name.clone())
            .collect();
        let name = compound_name(name, &taken);
        let mut inner =
            bettercut_timeline::Sequence::new(name, parent.resolution, parent.frame_rate)?;
        for lane in 0..video_lanes {
            inner
                .video_tracks
                .push(VideoTrack::new(format!("V{}", lane + 1)));
        }
        for lane in 0..audio_lanes {
            inner
                .audio_tracks
                .push(AudioTrack::new(format!("A{}", lane + 1)));
        }
        inner
            .text_tracks
            .push(bettercut_timeline::TextTrack::new("T1"));
        let inner_id = inner.id;
        let inner_video: Vec<TrackId> = inner.video_tracks.iter().map(|t| t.id).collect();
        let inner_audio: Vec<TrackId> = inner.audio_tracks.iter().map(|t| t.id).collect();
        let index = self.project().sequences.len();

        // The clip left in their place, made from a generated entry the way a
        // colour clip is.
        let (width, height) = (parent.resolution.width, parent.resolution.height);
        let payloads: Vec<(TrackId, ClipId, bool, usize, ClipPayload)> = members
            .iter()
            .filter_map(|(track, clip, video, lane)| {
                let payload = if *video {
                    ClipPayload::Video(Box::new(self.video_clip(*clip)?.clone()))
                } else {
                    ClipPayload::Audio(Box::new(self.audio_clip(*clip)?.clone()))
                };
                Some((*track, *clip, *video, *lane, payload))
            })
            .collect();

        let media = self.import_media(MediaAsset::generated(
            Generated::Compound { sequence: inner_id },
            width,
            height,
        ));
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(length.ticks()))?;
        let compound = VideoClip::new(media, first, source)?;
        let compound_id = compound.id;

        self.staged("Make Compound Clip", |editor, stage| {
            editor.stage(
                stage,
                Command::AddSequence {
                    sequence: Box::new(inner.clone()),
                    index,
                },
            )?;
            // Out of the parent first, so the compound can take their place.
            for (track, clip, ..) in &payloads {
                editor.stage(
                    stage,
                    Command::RemoveClip {
                        sequence: parent_id,
                        track: *track,
                        clip: *clip,
                    },
                )?;
            }
            for (_, _, video, lane, payload) in &payloads {
                let mut payload = payload.clone();
                shift_payload(&mut payload, -first.ticks());
                let track = if *video {
                    inner_video.get(*lane).copied()
                } else {
                    inner_audio.get(*lane).copied()
                };
                let Some(track) = track else { continue };
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence: inner_id,
                        track,
                        clip: payload,
                    },
                )?;
            }
            editor.stage(
                stage,
                Command::AddClip {
                    sequence: parent_id,
                    track: home,
                    clip: ClipPayload::Video(Box::new(compound.clone())),
                },
            )
        })?;
        Ok(compound_id)
    }

    /// Open the sequence behind a compound clip, so what is inside it can be
    /// edited. Returns false when the clip is not a compound.
    pub fn open_compound(&mut self, clip: ClipId) -> bool {
        match self.compound_of(clip) {
            Some(sequence) => self.switch_sequence(sequence),
            None => false,
        }
    }

    /// Put what is inside compound clip `clip` back on this timeline, where
    /// the compound sat, and take the compound off. One undo step. Returns
    /// how many clips came out.
    ///
    /// Inner picture lane *k* lands on the lane *k* above the compound's
    /// own; inner sound lane *k* on sound lane *k*. Lanes that are not there
    /// are added. A compound that was trimmed plays part of its inside, and
    /// breaking that apart would mean cutting clips the person never asked
    /// to cut — so it is refused with a word about why. The inner sequence
    /// stays in the project: another compound may play it too, and an undo
    /// needs it anyway.
    pub fn break_apart(&mut self, clip: ClipId) -> Result<usize, EditorError> {
        use crate::command::TrackKindRepr;

        let inner_id = self.compound_of(clip).ok_or(EditorError::NotACompound)?;
        let parent_id = self.active_sequence_id()?;
        let parent = self
            .project()
            .sequence(parent_id)
            .ok_or(EditorError::SequenceNotFound(parent_id))?;
        let span = parent
            .clip_span(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let compound = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let inner = self
            .project()
            .sequence(inner_id)
            .ok_or(EditorError::SequenceNotFound(inner_id))?;
        if compound.source.start != MediaTime::ZERO
            || compound.source.duration().ticks() != inner.duration().ticks()
            || !compound.speed.is_one()
        {
            return Err(EditorError::CompoundTrimmed);
        }

        let home = parent
            .video_tracks
            .iter()
            .position(|t| t.id == span.track)
            .ok_or(EditorError::TrackNotFound(span.track))?;
        let start = span.timeline.start.ticks();

        // What comes out, lane by lane, already moved to where it goes.
        let mut pictures: Vec<(usize, ClipPayload)> = Vec::new();
        for (lane, track) in inner.video_tracks.iter().enumerate() {
            for inside in track.clips() {
                let mut payload = ClipPayload::Video(Box::new(inside.clone()));
                shift_payload(&mut payload, start);
                pictures.push((home + lane, payload));
            }
        }
        let mut sounds: Vec<(usize, ClipPayload)> = Vec::new();
        for (lane, track) in inner.audio_tracks.iter().enumerate() {
            for inside in track.clips() {
                let mut payload = ClipPayload::Audio(Box::new(inside.clone()));
                shift_payload(&mut payload, start);
                sounds.push((lane, payload));
            }
        }
        let count = pictures.len() + sounds.len();

        // Lanes the parent does not have yet, minted now so the commands
        // carry their ids (§38.2's journal replays them exactly).
        let picture_lanes = parent.video_tracks.len();
        let sound_lanes = parent.audio_tracks.len();
        let picture_needed = pictures.iter().map(|(lane, _)| lane + 1).max().unwrap_or(0);
        let sound_needed = sounds.iter().map(|(lane, _)| lane + 1).max().unwrap_or(0);
        let mut picture_ids: Vec<TrackId> = parent.video_tracks.iter().map(|t| t.id).collect();
        let mut sound_ids: Vec<TrackId> = parent.audio_tracks.iter().map(|t| t.id).collect();
        let new_pictures: Vec<(TrackId, String)> = (picture_lanes..picture_needed)
            .map(|lane| (TrackId::new(), format!("V{}", lane + 1)))
            .collect();
        let new_sounds: Vec<(TrackId, String)> = (sound_lanes..sound_needed)
            .map(|lane| (TrackId::new(), format!("A{}", lane + 1)))
            .collect();
        picture_ids.extend(new_pictures.iter().map(|(id, _)| *id));
        sound_ids.extend(new_sounds.iter().map(|(id, _)| *id));

        self.staged("Break Apart Compound", |editor, stage| {
            for (id, name) in &new_pictures {
                editor.stage(
                    stage,
                    Command::AddTrack {
                        sequence: parent_id,
                        kind: TrackKindRepr::Video,
                        name: name.clone(),
                        id: *id,
                    },
                )?;
            }
            for (id, name) in &new_sounds {
                editor.stage(
                    stage,
                    Command::AddTrack {
                        sequence: parent_id,
                        kind: TrackKindRepr::Audio,
                        name: name.clone(),
                        id: *id,
                    },
                )?;
            }
            editor.stage(
                stage,
                Command::RemoveClip {
                    sequence: parent_id,
                    track: span.track,
                    clip,
                },
            )?;
            for (lane, payload) in &pictures {
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence: parent_id,
                        track: picture_ids[*lane],
                        clip: payload.clone(),
                    },
                )?;
            }
            for (lane, payload) in &sounds {
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence: parent_id,
                        track: sound_ids[*lane],
                        clip: payload.clone(),
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(count)
    }
}

/// Move a clip to start `by` ticks later (earlier when negative), keeping its
/// length — what putting a clip into a sequence that starts at zero needs.
fn shift_payload(payload: &mut ClipPayload, by: i64) {
    let shift = |range: bettercut_timeline::TimelineRange| bettercut_timeline::TimelineRange {
        start: TimelineTime::from_ticks(range.start.ticks() + by),
        end: TimelineTime::from_ticks(range.end.ticks() + by),
    };
    match payload {
        ClipPayload::Video(clip) => {
            let range = shift(clip.timeline());
            clip.set_timeline(range);
        }
        ClipPayload::Audio(clip) => {
            let range = shift(clip.timeline());
            clip.set_timeline(range);
        }
        ClipPayload::Text(clip) => {
            let range = shift(clip.timeline());
            clip.set_timeline(range);
        }
        ClipPayload::Adjustment(clip) => {
            let range = shift(clip.timeline());
            clip.set_timeline(range);
        }
    }
}

/// A name for the inner sequence: what was asked for, or "Compound 1", and
/// never one a sequence already has.
fn compound_name(wanted: &str, taken: &[String]) -> String {
    let wanted = wanted.trim();
    if !wanted.is_empty() && !taken.iter().any(|name| name == wanted) {
        return wanted.to_owned();
    }
    let stem = if wanted.is_empty() {
        "Compound"
    } else {
        wanted
    };
    (1..)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !taken.iter().any(|taken| taken == name))
        .unwrap_or_else(|| stem.to_owned())
}
