//! Concrete reversible operations (§10).
//!
//! Each holds exactly what its undo needs — the removed clip, the previous
//! name — and nothing else. §11: "Do not snapshot the entire project after
//! every edit."

use bettercut_foundation::{
    ClipId, FrameRate, MediaId, MediaTime, Rational, SequenceId, TimelineTime, TrackId,
    ticks_per_frame,
};
use bettercut_project_format::Project;
use bettercut_timeline::{
    AnimatedParameter, AudioTrack, Clip, Keyframe, MIN_TRANSITION, Resolution, Sequence,
    SourceRange, TextClip, TimelineRange, TrackKind, Transition, Vec2, VideoTrack,
};

use crate::command::{
    ClipPayload, ClipProperty, EditorCommand, SettingChange, TextProperty, TrackFlag, TrackPayload,
};
use crate::error::EditorError;

fn sequence_mut(project: &mut Project, id: SequenceId) -> Result<&mut Sequence, EditorError> {
    project
        .sequence_mut(id)
        .ok_or(EditorError::SequenceNotFound(id))
}

/// Rebuild an executable command from a journalled request (§38.2 replay).
///
/// Unlike `Editor::dispatch`, this performs **no validation and no frame
/// snapping**:
///
/// * There is no live project to validate against — replay applies to a
///   snapshot, and `execute` reports anything that no longer fits.
/// * Times were already snapped when the command was first constructed
///   (§76). Snapping again would be a no-op at best, and at worst would move a
///   cut if the sequence's frame rate had changed since.
pub fn build_for_replay(
    command: crate::command::Command,
) -> Result<Box<dyn EditorCommand>, EditorError> {
    use crate::command::Command;

    Ok(match command {
        Command::RenameProject { name } => Box::new(RenameProject::new(name)),
        Command::ChangeSetting { change } => Box::new(ChangeSetting::new(change)),
        Command::RelinkMedia {
            media,
            path,
            file_size,
        } => Box::new(RelinkMedia::new(media, path, file_size)),
        Command::RemoveMedia { media } => Box::new(RemoveMedia::new(media)),
        Command::SetSequenceProperty { sequence, property } => {
            Box::new(SetSequenceProperty::new(sequence, property))
        }
        Command::SetClipProperty {
            sequence,
            track,
            clip,
            property,
        } => Box::new(SetClipProperty::new(sequence, track, clip, property)),
        Command::SetTransition {
            sequence,
            track,
            clip,
            transition,
        } => Box::new(SetTransition::new(sequence, track, clip, transition)),
        Command::SetClipSpeed {
            sequence,
            track,
            clip,
            speed,
        } => Box::new(SetClipSpeed::new(sequence, track, clip, speed)),
        Command::SetClipFades {
            sequence,
            track,
            clip,
            fade_in,
            fade_out,
        } => Box::new(SetClipFades::new(sequence, track, clip, fade_in, fade_out)),
        Command::SetColorLabel {
            sequence,
            track,
            clip,
            label,
        } => Box::new(SetColorLabel::new(sequence, track, clip, label)),
        Command::SetTrackMix {
            sequence,
            track,
            gain,
            pan,
        } => Box::new(SetTrackMix::new(sequence, track, gain, pan)),
        Command::SetTrackVolume {
            sequence,
            track,
            points,
        } => Box::new(SetTrackVolume::new(sequence, track, points)),
        Command::SetMarkers { sequence, markers } => Box::new(SetMarkers::new(sequence, markers)),
        Command::SetGroups { sequence, groups } => Box::new(SetGroups::new(sequence, groups)),
        Command::SetClipNote {
            sequence,
            clip,
            text,
        } => Box::new(SetClipNote::new(sequence, clip, text)),
        Command::SetInOut {
            sequence,
            mark_in,
            mark_out,
        } => Box::new(SetInOut::new(sequence, mark_in, mark_out)),
        Command::Unlink { sequence, link } => Box::new(Unlink::new(sequence, link)),
        Command::AddText {
            sequence,
            track,
            clip,
        } => Box::new(AddText::new(sequence, track, *clip)),
        Command::RemoveText {
            sequence,
            track,
            clip,
        } => Box::new(RemoveText::new(sequence, track, clip)),
        Command::SetTextProperty {
            sequence,
            track,
            clip,
            property,
        } => Box::new(SetTextProperty::new(sequence, track, clip, property)),
        Command::AddAdjustment {
            sequence,
            track,
            clip,
        } => Box::new(AddAdjustment::new(sequence, track, *clip)),
        Command::RemoveAdjustment {
            sequence,
            track,
            clip,
        } => Box::new(RemoveAdjustment::new(sequence, track, clip)),
        Command::SetAdjustmentLook {
            sequence,
            track,
            clip,
            look,
        } => Box::new(SetAdjustmentLook::new(sequence, track, clip, look)),
        Command::SetGainEnvelope {
            sequence,
            track,
            clip,
            keys,
        } => Box::new(SetGainEnvelope::new(sequence, track, clip, keys)),
        Command::SetKeyframe {
            sequence,
            track,
            clip,
            parameter,
            key,
        } => Box::new(SetKeyframe::set(sequence, track, clip, parameter, key)),
        Command::RemoveKeyframe {
            sequence,
            track,
            clip,
            parameter,
            time,
        } => Box::new(SetKeyframe::remove(sequence, track, clip, parameter, time)),
        Command::SetVisualizer {
            sequence,
            visualizer,
        } => Box::new(SetVisualizer::new(sequence, visualizer.map(|v| *v))),
        Command::SetCoverFrame { sequence, at } => Box::new(SetCoverFrame {
            sequence,
            at,
            previous: None,
        }),
        Command::SetWatermark {
            sequence,
            watermark,
        } => Box::new(SetWatermark {
            sequence,
            watermark: watermark.map(bettercut_timeline::watermark::Watermark::clamped),
            previous: None,
        }),
        Command::SetSequenceFormat {
            sequence,
            resolution,
            frame_rate,
        } => Box::new(SetSequenceFormat::new(
            sequence,
            resolution.into(),
            frame_rate,
        )),

        Command::AddTrack {
            sequence,
            kind,
            name,
            id,
        } => Box::new(AddTrack::new(sequence, kind.into(), name, id)),

        Command::RemoveTrack { sequence, track } => Box::new(RemoveTrack::new(sequence, track)),

        Command::SetTrackFlag {
            sequence,
            track,
            flag,
            value,
        } => Box::new(SetTrackFlag::new(sequence, track, flag, value)),

        Command::AddClip {
            sequence,
            track,
            clip,
        } => Box::new(AddClip::new(sequence, track, clip)),

        Command::InsertTrack {
            sequence,
            index,
            track,
        } => Box::new(InsertTrack::new(sequence, index, track)),
        Command::RenameTrack {
            sequence,
            track,
            name,
        } => Box::new(RenameTrack::new(sequence, track, name)),
        Command::RenameMedia { media, name } => Box::new(RenameMedia::new(media, name)),
        Command::SetMediaBin { media, bin } => Box::new(SetMediaBin::new(media, bin)),
        Command::SetMediaRating { media, rating } => Box::new(SetMediaRating::new(media, rating)),
        Command::SetColour { media, colour } => Box::new(SetColour::new(media, colour)),
        Command::AddSequence { sequence, index } => Box::new(AddSequence::new(sequence, index)),
        Command::RemoveSequence { sequence } => Box::new(RemoveSequence::new(sequence)),
        Command::RenameSequence { sequence, name } => Box::new(RenameSequence::new(sequence, name)),

        Command::RemoveClip {
            sequence,
            track,
            clip,
        } => Box::new(RemoveClip::new(sequence, track, clip)),

        Command::ReplaceClipMedia {
            sequence,
            track,
            clip,
            swap,
        } => Box::new(ReplaceClipMedia::new(sequence, track, clip, swap)),

        Command::MoveClip {
            sequence,
            from_track,
            to_track,
            clip,
            new_start,
        } => Box::new(MoveClip::new(
            sequence, from_track, to_track, clip, new_start,
        )),

        Command::TrimClip {
            sequence,
            track,
            clip,
            edge,
            to,
        } => Box::new(TrimClip::new(sequence, track, clip, edge, to)),

        Command::SlipClip {
            sequence,
            track,
            clip,
            offset,
        } => Box::new(crate::slip::SlipClip::new(sequence, track, clip, offset)),

        Command::SetClipAngle {
            sequence,
            track,
            clip,
            angle,
        } => Box::new(SetClipAngle {
            sequence,
            track,
            clip,
            angle,
            previous: None,
        }),

        Command::SplitClip {
            sequence,
            track,
            clip,
            at,
            left,
            right,
            relink,
        } => Box::new(SplitClip::new(sequence, track, clip, at, left, right).relinked(relink)),

        Command::RippleDeleteClip {
            sequence,
            track,
            clip,
        } => Box::new(RippleDeleteClip::new(sequence, track, clip)),

        Command::PasteClip {
            sequence,
            track,
            clip,
            at,
            new_id,
        } => Box::new(PasteClip::new(sequence, track, clip, at, new_id)),
    })
}

/// Run one body against whichever track `track` names, whatever kind it is.
///
/// A macro rather than a function because the three track types are different
/// types: `Track<VideoClip>`, `Track<AudioClip>`, `Track<TextClip>`. The
/// operations that need this — move, trim, split, remove — are written against
/// [`Track`]'s generic methods and read identically for all three, and before
/// this the video and audio versions were written out twice at every call site
/// with nothing keeping them in step.
///
/// The kind is worked out first and the track borrowed once, rather than
/// chaining `if let` over three mutable lookups: only one mutable borrow of the
/// sequence may be alive at a time.
macro_rules! on_track {
    ($project:expr, $sequence:expr, $track:expr, |$name:ident| $body:expr) => {{
        let track_id = $track;
        let sequence = sequence_mut($project, $sequence)?;
        let kind = if sequence.video_tracks.iter().any(|t| t.id == track_id) {
            0
        } else if sequence.audio_tracks.iter().any(|t| t.id == track_id) {
            1
        } else if sequence.text_tracks.iter().any(|t| t.id == track_id) {
            2
        } else if sequence.adjustment_tracks.iter().any(|t| t.id == track_id) {
            3
        } else {
            4
        };

        match kind {
            0 => {
                let $name = sequence
                    .video_track_mut(track_id)
                    .ok_or(EditorError::TrackNotFound(track_id))?;
                $body
            }
            1 => {
                let $name = sequence
                    .audio_track_mut(track_id)
                    .ok_or(EditorError::TrackNotFound(track_id))?;
                $body
            }
            2 => {
                let $name = sequence
                    .text_track_mut(track_id)
                    .ok_or(EditorError::TrackNotFound(track_id))?;
                $body
            }
            // Not caught by the compiler: the kind is a number worked out
            // above, so a lane missing from this list reads as "track not
            // found" — and an adjustment could not be moved, trimmed or split.
            3 => {
                let $name = sequence
                    .adjustment_track_mut(track_id)
                    .ok_or(EditorError::TrackNotFound(track_id))?;
                $body
            }
            _ => Err(EditorError::TrackNotFound(track_id)),
        }
    }};
}

/// Run `on_video` or `on_audio` depending on which kind of track `track` is.
///
/// Every edit command below needs this, and writing it twice per command is how
/// the video and audio paths quietly grow different behaviour.
fn with_track<R>(
    project: &mut Project,
    sequence: SequenceId,
    track: TrackId,
    on_video: impl FnOnce(&mut VideoTrack) -> Result<R, EditorError>,
    on_audio: impl FnOnce(&mut AudioTrack) -> Result<R, EditorError>,
) -> Result<R, EditorError> {
    let sequence = sequence_mut(project, sequence)?;
    match sequence.track_kind(track) {
        Some(TrackKind::Video) => {
            let track = sequence
                .video_track_mut(track)
                .ok_or(EditorError::TrackNotFound(track))?;
            on_video(track)
        }
        Some(TrackKind::Audio) => {
            let track = sequence
                .audio_track_mut(track)
                .ok_or(EditorError::TrackNotFound(track))?;
            on_audio(track)
        }
        // §26: the operations that reach a text track go through `on_track!`,
        // which handles all three kinds. This one is for the commands that
        // genuinely differ between video and audio — clip properties, gain,
        // keyframes — and none of them apply to a title.
        Some(TrackKind::Text) => Err(EditorError::ClipKindMismatch),
        // An adjustment has a look of its own, set through its own command.
        Some(TrackKind::Adjustment) => Err(EditorError::ClipKindMismatch),
        None => Err(EditorError::TrackNotFound(track)),
    }
}

/// The source out-point a clip may not be trimmed past: the media's duration.
///
/// `None` when the media is unknown, in which case only the structural
/// invariants apply — a missing asset must not make trimming impossible (§66).
/// `None` for a still, too, which runs as long as it is dragged out.
fn media_limit(project: &Project, sequence: SequenceId, clip: ClipId) -> Option<MediaTime> {
    let sequence = project.sequence(sequence)?;
    let media_id = sequence
        .video_tracks
        .iter()
        .find_map(|t| t.get(clip).map(|c| c.media_id))
        .or_else(|| {
            sequence
                .audio_tracks
                .iter()
                .find_map(|t| t.get(clip).map(|c| c.media_id))
        })?;
    // A frozen clip holds one instant, so how much material is left past it
    // does not limit how long it can be held — the same answer as a still.
    let frozen = sequence
        .video_tracks
        .iter()
        .find_map(|t| t.get(clip).map(|c| c.frozen))
        .unwrap_or(false);
    if frozen {
        return None;
    }
    project.media_asset(media_id)?.source_limit()
}

/// The longest transition the cut at the end of `clip` can support (§25).
///
/// `Ok(None)` when there is no cut there at all — no next clip, or a gap.
pub(crate) fn transition_room(
    project: &Project,
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    kind: bettercut_timeline::TransitionKind,
) -> Option<TimelineTime> {
    let sequence = project.sequence(sequence)?;
    let track = sequence.video_tracks.iter().find(|t| t.id == track)?;
    let clips = track.clips();
    let index = clips.iter().position(|c| c.id == clip)?;
    let outgoing = &clips[index];
    let incoming = clips.get(index + 1)?;
    // A gap is not a cut: there is nothing on the other side to fade to.
    if incoming.timeline.start != outgoing.timeline.end {
        return None;
    }

    // Handles are whatever the file has outside each clip's own range. Media
    // that cannot be read reports none, which refuses a crossfade rather than
    // promising one that would flash black (§66).
    //
    // A still has handles without end: every instant outside its range is the
    // same picture as every instant inside it. A day stands in for "without
    // end" — far past any transition, and small enough that scaling it by a
    // clip's speed cannot overflow.
    const ENDLESS: MediaTime = MediaTime::from_seconds(24 * 60 * 60);
    let still = |media| project.media_asset(media).is_some_and(|m| m.is_still());

    let handle_after = if still(outgoing.media_id) {
        ENDLESS
    } else {
        project
            .media_asset(outgoing.media_id)
            .map(|m| {
                MediaTime::from_ticks((m.duration.ticks() - outgoing.source.end.ticks()).max(0))
            })
            .unwrap_or(MediaTime::ZERO)
    };
    let handle_before = if still(incoming.media_id) {
        ENDLESS
    } else {
        incoming.source.start
    };

    // `max_duration` reasons in *timeline* ticks, and a handle is source.
    // A clip playing at 2× burns two ticks of handle for every tick of
    // transition, so its handle is worth half as much — measured unscaled, a
    // fast clip would be offered twice the crossfade its footage can cover.
    let usable = |handle: MediaTime, speed: bettercut_foundation::Rational| {
        MediaTime::from_ticks(bettercut_timeline::timeline_ticks_for(handle, speed))
    };

    Some(Transition::max_duration(
        kind,
        outgoing.timeline.duration(),
        incoming.timeline.duration(),
        usable(handle_after, outgoing.speed),
        usable(handle_before, incoming.speed),
    ))
}

/// Add a text overlay (§26).
#[derive(Debug)]
pub struct AddText {
    sequence: SequenceId,
    track: TrackId,
    clip: TextClip,
    executed: bool,
}

impl AddText {
    pub fn new(sequence: SequenceId, track: TrackId, clip: TextClip) -> Self {
        Self {
            sequence,
            track,
            clip,
            executed: false,
        }
    }
}

impl EditorCommand for AddText {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        // Cloned rather than moved, so a redo has the clip to insert again —
        // identical, id included, which is what keeps a selection valid across
        // undo and redo.
        let clip = self.clip.clone();
        let sequence = sequence_mut(project, self.sequence)?;
        let track = sequence
            .text_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;
        track.insert(clip)?;
        self.executed = true;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if !self.executed {
            return Err(EditorError::NotExecuted);
        }
        self.executed = false;
        let sequence = sequence_mut(project, self.sequence)?;
        let track = sequence
            .text_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;
        track.remove(self.clip.id)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Add Text".to_owned()
    }
}

/// Take a text overlay off the timeline (§26).
#[derive(Debug)]
pub struct RemoveText {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    /// The whole clip, kept so undo restores the words and the styling rather
    /// than an empty title where one used to be.
    removed: Option<TextClip>,
}

impl RemoveText {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId) -> Self {
        Self {
            sequence,
            track,
            clip,
            removed: None,
        }
    }
}

impl EditorCommand for RemoveText {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let track = sequence
            .text_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;
        self.removed = Some(track.remove(self.clip)?);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let clip = self.removed.take().ok_or(EditorError::NotExecuted)?;
        let sequence = sequence_mut(project, self.sequence)?;
        let track = sequence
            .text_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;
        track.insert(clip)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Remove Text".to_owned()
    }
}

/// Change one thing about a text overlay (§26).
#[derive(Debug)]
pub struct SetTextProperty {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    property: TextProperty,
    previous: Option<TextProperty>,
}

impl SetTextProperty {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId, property: TextProperty) -> Self {
        Self {
            sequence,
            track,
            clip,
            property,
            previous: None,
        }
    }

    /// True when `other` is the same control being dragged, so a slider drag
    /// collapses into one undo step (§11).
    pub fn is_same_gesture(&self, other: &Self) -> bool {
        self.clip == other.clip && self.property.kind() == other.property.kind()
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        property: TextProperty,
    ) -> Result<TextProperty, EditorError> {
        let sequence = sequence_mut(project, sequence)?;
        let track = sequence
            .text_track_mut(track)
            .ok_or(EditorError::TrackNotFound(track))?;
        let clip = track.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;

        Ok(match property {
            TextProperty::Content(text) => {
                TextProperty::Content(std::mem::replace(&mut clip.text, text))
            }
            // Sanitized on the way in, not on the way out: §38.2 replays
            // commands, and a limit applied only in the interface would come
            // back unapplied.
            TextProperty::Style(style) => TextProperty::Style(Box::new(std::mem::replace(
                &mut clip.style,
                style.sanitized(),
            ))),
            // The same clamps the video clips use, from the same place, so a
            // title and a clip cannot end up with different limits on the same
            // control (§24).
            TextProperty::Position { x, y } => {
                let was = clip.transform.position;
                clip.transform.position = Vec2::new(
                    AnimatedParameter::PositionX.clamp(x),
                    AnimatedParameter::PositionY.clamp(y),
                );
                TextProperty::Position { x: was.x, y: was.y }
            }
            TextProperty::Scale { x, y } => {
                let was = clip.transform.scale;
                clip.transform.scale = Vec2::new(
                    AnimatedParameter::ScaleX.clamp(x),
                    AnimatedParameter::ScaleY.clamp(y),
                );
                TextProperty::Scale { x: was.x, y: was.y }
            }
            TextProperty::Rotation(degrees) => {
                let was = clip.transform.rotation_degrees;
                clip.transform.rotation_degrees = AnimatedParameter::Rotation.clamp(degrees);
                TextProperty::Rotation(was)
            }
            TextProperty::Opacity(value) => {
                let was = clip.opacity;
                clip.opacity = AnimatedParameter::Opacity.clamp(value);
                TextProperty::Opacity(was)
            }
            TextProperty::Animation(animation) => TextProperty::Animation(std::mem::replace(
                &mut clip.animation,
                animation.sanitized(),
            )),
            TextProperty::Shape(shape) => {
                TextProperty::Shape(std::mem::replace(&mut clip.shape, shape))
            }
            TextProperty::Counter(counter) => TextProperty::Counter(std::mem::replace(
                &mut clip.counter,
                counter.map(bettercut_timeline::Counter::sanitized),
            )),
            TextProperty::Highlight(colour) => {
                TextProperty::Highlight(std::mem::replace(&mut clip.highlight, colour))
            }
            TextProperty::MotionBlur(on) => {
                let was = clip.motion_blur;
                clip.motion_blur = on;
                TextProperty::MotionBlur(was)
            }
        })
    }
}

impl EditorCommand for SetTextProperty {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            self.property.clone(),
        )?;
        // First execute only: a redo must restore the value from before the
        // whole gesture, not from the previous redo step.
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, self.clip, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        format!("Change {}", self.property.kind())
    }
}

/// Add an adjustment clip.
#[derive(Debug)]
pub struct AddAdjustment {
    sequence: SequenceId,
    track: TrackId,
    clip: bettercut_timeline::AdjustmentClip,
    executed: bool,
}

impl AddAdjustment {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: bettercut_timeline::AdjustmentClip,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            executed: false,
        }
    }
}

impl EditorCommand for AddAdjustment {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        // Cloned rather than moved, so a redo inserts the identical clip.
        let mut clip = self.clip.clone();
        // Brought into range on the way in: §38.2 replays this command, and a
        // limit held only by the interface would come back unapplied.
        clip.look = clip.look.clamped();
        sequence_mut(project, self.sequence)?
            .adjustment_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?
            .insert(clip)?;
        self.executed = true;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if !self.executed {
            return Err(EditorError::NotExecuted);
        }
        self.executed = false;
        sequence_mut(project, self.sequence)?
            .adjustment_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?
            .remove(self.clip.id)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Add Adjustment".to_owned()
    }
}

/// Take an adjustment clip off the timeline.
#[derive(Debug)]
pub struct RemoveAdjustment {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    /// The whole clip, so undo brings back the grade and not just the span.
    removed: Option<bettercut_timeline::AdjustmentClip>,
}

impl RemoveAdjustment {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId) -> Self {
        Self {
            sequence,
            track,
            clip,
            removed: None,
        }
    }
}

impl EditorCommand for RemoveAdjustment {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let track = sequence_mut(project, self.sequence)?
            .adjustment_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;
        self.removed = Some(track.remove(self.clip)?);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let clip = self.removed.take().ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.sequence)?
            .adjustment_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?
            .insert(clip)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Remove Adjustment".to_owned()
    }
}

/// Set how an adjustment grades what is beneath it.
#[derive(Debug)]
pub struct SetAdjustmentLook {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    look: bettercut_timeline::AdjustmentLook,
    /// The look before the *first* execute, so undoing a whole drag goes back
    /// to where it began rather than one frame earlier.
    previous: Option<bettercut_timeline::AdjustmentLook>,
}

impl SetAdjustmentLook {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        look: bettercut_timeline::AdjustmentLook,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            look,
            previous: None,
        }
    }

    fn apply(
        &self,
        project: &mut Project,
        look: bettercut_timeline::AdjustmentLook,
    ) -> Result<bettercut_timeline::AdjustmentLook, EditorError> {
        let clip = sequence_mut(project, self.sequence)?
            .adjustment_track_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?
            .get_mut(self.clip)
            .ok_or(EditorError::ClipNotFound(self.clip))?;
        Ok(std::mem::replace(&mut clip.look, look.clamped()))
    }
}

impl EditorCommand for SetAdjustmentLook {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.apply(project, self.look)?;
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        self.apply(project, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Change Adjustment".to_owned()
    }
}

/// Detach linked clips from each other (§12).
#[derive(Debug)]
pub struct Unlink {
    sequence: SequenceId,
    link: bettercut_foundation::LinkId,
    /// Which clips carried it, so undo ties the same ones back together.
    unlinked: Vec<ClipId>,
}

impl Unlink {
    pub fn new(sequence: SequenceId, link: bettercut_foundation::LinkId) -> Self {
        Self {
            sequence,
            link,
            unlinked: Vec::new(),
        }
    }
}

impl EditorCommand for Unlink {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let unlinked = set_link_where(sequence, |c| c == Some(self.link), None);
        if unlinked.is_empty() {
            // Nothing carried it. Succeeding would put an undo entry in the
            // history that undoes nothing.
            return Err(EditorError::NothingLinked);
        }
        self.unlinked = unlinked;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        for clip in &self.unlinked {
            relink_one(sequence, *clip, Some(self.link));
        }
        Ok(())
    }

    fn label(&self) -> String {
        "Unlink Audio".to_owned()
    }
}

/// Set the link on every video and audio clip whose current link matches,
/// returning which clips changed.
///
/// Ids first and mutation second, through `get_mut`: `Track` deliberately has
/// no mutable view of its whole slice, because that is how a caller would
/// break its ordering without meaning to.
fn set_link_where(
    sequence: &mut Sequence,
    matches: impl Fn(Option<bettercut_foundation::LinkId>) -> bool,
    to: Option<bettercut_foundation::LinkId>,
) -> Vec<ClipId> {
    let video: Vec<ClipId> = sequence
        .video_tracks
        .iter()
        .flat_map(|t| t.clips())
        .filter(|c| matches(c.link))
        .map(|c| c.id)
        .collect();
    let audio: Vec<ClipId> = sequence
        .audio_tracks
        .iter()
        .flat_map(|t| t.clips())
        .filter(|c| matches(c.link))
        .map(|c| c.id)
        .collect();

    for clip in video.iter().chain(audio.iter()) {
        relink_one(sequence, *clip, to);
    }
    video.into_iter().chain(audio).collect()
}

fn relink_one(sequence: &mut Sequence, clip: ClipId, to: Option<bettercut_foundation::LinkId>) {
    if let Some(found) = sequence
        .video_tracks
        .iter_mut()
        .find_map(|t| t.get_mut(clip))
    {
        found.link = to;
    } else if let Some(found) = sequence
        .audio_tracks
        .iter_mut()
        .find_map(|t| t.get_mut(clip))
    {
        found.link = to;
    }
}

/// Change how fast a clip plays.
///
/// Speeding a clip up shortens it and slowing it down lengthens it, because the
/// source range is what the clip *plays* and the speed decides how long that
/// takes. The clip keeps its start; the end moves, and the rest of the track
/// moves with it (see [`bettercut_timeline::VideoTrack::ripple_resize`]).
#[derive(Debug)]
pub struct SetClipSpeed {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    speed: Rational,
    /// The speed *and* the end it had, because restoring one without the other
    /// leaves a clip whose length does not match the material it plays.
    previous: Option<(Rational, TimelineTime)>,
}

impl SetClipSpeed {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId, speed: Rational) -> Self {
        Self {
            sequence,
            track,
            clip,
            speed,
            previous: None,
        }
    }

    /// True when `other` is the same clip's speed being dragged, so a drag
    /// across the slider collapses into one undo step (§11).
    pub fn is_same_gesture(&self, other: &Self) -> bool {
        self.clip == other.clip
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track_id: TrackId,
        clip_id: ClipId,
        speed: Rational,
        end: Option<TimelineTime>,
    ) -> Result<(Rational, TimelineTime), EditorError> {
        // Generic over the track kind: §12's video file is a picture clip on a
        // video track *and* a sound clip on an audio one, and re-timing has to
        // reach both. The body is identical for either.
        on_track!(project, sequence, track_id, |track| {
            let clip = track
                .get_mut(clip_id)
                .ok_or(EditorError::ClipNotFound(clip_id))?;

            let was_speed = clip.speed();
            let speed = bettercut_timeline::clamped_speed(speed);

            // On undo the end comes from what was recorded; on execute it is
            // derived. Recomputing it on undo instead would round a second
            // time and could land a tick away from where the clip actually was.
            let new_end = match end {
                Some(end) => end,
                None => {
                    clip.timeline().start
                        + TimelineTime::from_ticks(bettercut_timeline::timeline_ticks_for(
                            clip.source().duration(),
                            speed,
                        ))
                }
            };

            clip.set_speed(speed);
            match track.ripple_resize(clip_id, new_end) {
                Ok(was_end) => Ok((was_speed, was_end)),
                Err(err) => {
                    // The length did not change, so the speed must not either —
                    // otherwise the clip would play material it has no room for.
                    if let Some(clip) = track.get_mut(clip_id) {
                        clip.set_speed(was_speed);
                    }
                    Err(err.into())
                }
            }
        })
    }
}

impl EditorCommand for SetClipSpeed {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            self.speed,
            None,
        )?;
        // First execute only: a redo restores the state from before the whole
        // gesture, not from the previous redo step.
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (speed, end) = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            speed,
            Some(end),
        )?;
        Ok(())
    }

    fn label(&self) -> String {
        "Change speed".to_owned()
    }
}

/// Leave, change or take off a clip's note.
#[derive(Debug)]
pub struct SetClipNote {
    sequence: SequenceId,
    clip: ClipId,
    text: String,
    /// What was there before, `Some("")` meaning no note: set on execute.
    previous: Option<String>,
}

impl SetClipNote {
    pub fn new(sequence: SequenceId, clip: ClipId, text: String) -> Self {
        Self {
            sequence,
            clip,
            text,
            previous: None,
        }
    }

    /// Put `text` on the clip, or none when empty; returns what was there.
    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        clip: ClipId,
        text: &str,
    ) -> Result<String, EditorError> {
        let notes = &mut sequence_mut(project, sequence)?.notes;
        let was = match notes.iter().position(|note| note.clip == clip) {
            Some(index) if text.is_empty() => notes.remove(index).text,
            Some(index) => std::mem::replace(&mut notes[index].text, text.to_owned()),
            None => {
                if !text.is_empty() {
                    notes.push(bettercut_timeline::ClipNote {
                        clip,
                        text: text.to_owned(),
                    });
                }
                String::new()
            }
        };
        Ok(was)
    }
}

impl EditorCommand for SetClipNote {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let was = Self::apply(project, self.sequence, self.clip, &self.text)?;
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.clip, &previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        if self.text.is_empty() {
            "Remove Note".to_owned()
        } else {
            "Clip Note".to_owned()
        }
    }
}

/// Replace a sequence's clip groups.
#[derive(Debug)]
pub struct SetGroups {
    sequence: SequenceId,
    groups: Vec<Vec<ClipId>>,
    previous: Option<Vec<Vec<ClipId>>>,
}

impl SetGroups {
    pub fn new(sequence: SequenceId, groups: Vec<Vec<ClipId>>) -> Self {
        Self {
            sequence,
            groups,
            previous: None,
        }
    }
}

impl EditorCommand for SetGroups {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        // A group of one is no group; none is kept.
        let groups = self
            .groups
            .iter()
            .filter(|group| group.len() > 1)
            .cloned()
            .collect();
        let previous = std::mem::replace(&mut sequence.groups, groups);
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.sequence)?.groups = previous;
        Ok(())
    }

    fn label(&self) -> String {
        "Change groups".to_owned()
    }
}

/// Replace a sequence's markers.
#[derive(Debug)]
pub struct SetMarkers {
    sequence: SequenceId,
    markers: Vec<bettercut_timeline::Marker>,
    previous: Option<Vec<bettercut_timeline::Marker>>,
}

impl SetMarkers {
    pub fn new(sequence: SequenceId, markers: Vec<bettercut_timeline::Marker>) -> Self {
        Self {
            sequence,
            markers,
            previous: None,
        }
    }
}

impl EditorCommand for SetMarkers {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let previous = std::mem::replace(
            &mut sequence.markers,
            bettercut_timeline::marker::normalized(self.markers.clone()),
        );
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.sequence)?.markers = previous;
        Ok(())
    }

    fn label(&self) -> String {
        "Change markers".to_owned()
    }
}

/// Set the in and out marks.
#[derive(Debug)]
pub struct SetInOut {
    sequence: SequenceId,
    marks: (Option<TimelineTime>, Option<TimelineTime>),
    previous: Option<(Option<TimelineTime>, Option<TimelineTime>)>,
}

impl SetInOut {
    pub fn new(
        sequence: SequenceId,
        mark_in: Option<TimelineTime>,
        mark_out: Option<TimelineTime>,
    ) -> Self {
        Self {
            sequence,
            marks: (mark_in, mark_out),
            previous: None,
        }
    }
}

impl EditorCommand for SetInOut {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let was = (sequence.mark_in, sequence.mark_out);
        // Never before the start: a mark is an instant on the timeline.
        let clamp = |t: Option<TimelineTime>| t.map(|t| t.max(TimelineTime::ZERO));
        (sequence.mark_in, sequence.mark_out) = (clamp(self.marks.0), clamp(self.marks.1));
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (mark_in, mark_out) = self.previous.ok_or(EditorError::NotExecuted)?;
        let sequence = sequence_mut(project, self.sequence)?;
        (sequence.mark_in, sequence.mark_out) = (mark_in, mark_out);
        Ok(())
    }

    fn label(&self) -> String {
        "Change In/Out".to_owned()
    }
}

/// Set an audio track's volume line (`timeline::track_volume`).
#[derive(Debug)]
pub struct SetTrackVolume {
    sequence: SequenceId,
    track: TrackId,
    points: Vec<bettercut_timeline::VolumePoint>,
    previous: Option<bettercut_timeline::TrackVolume>,
}

impl SetTrackVolume {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        points: Vec<bettercut_timeline::VolumePoint>,
    ) -> Self {
        Self {
            sequence,
            track,
            points,
            previous: None,
        }
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        volume: bettercut_timeline::TrackVolume,
    ) -> Result<bettercut_timeline::TrackVolume, EditorError> {
        let sequence = sequence_mut(project, sequence)?;
        // A picture lane has no volume to ride, the same refusal the track
        // stage makes.
        let missing = if sequence.track_kind(track).is_some() {
            EditorError::ClipKindMismatch
        } else {
            EditorError::TrackNotFound(track)
        };
        let track = sequence.audio_track_mut(track).ok_or(missing)?;
        Ok(std::mem::replace(&mut track.volume, volume))
    }
}

impl EditorCommand for SetTrackVolume {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        // Sorted and clamped on the way in, in the model, for §38.2's reason:
        // a replayed command has to land exactly where the first one did.
        let volume = bettercut_timeline::TrackVolume::from(self.points.clone());
        let previous = Self::apply(project, self.sequence, self.track, volume)?;
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        if self.points.is_empty() {
            "Clear Track Volume".to_owned()
        } else {
            "Track Volume".to_owned()
        }
    }
}

/// Set an audio track's volume and pan.
#[derive(Debug)]
pub struct SetTrackMix {
    sequence: SequenceId,
    track: TrackId,
    mix: (f32, f32),
    previous: Option<(f32, f32)>,
}

impl SetTrackMix {
    pub fn new(sequence: SequenceId, track: TrackId, gain: f32, pan: f32) -> Self {
        Self {
            sequence,
            track,
            mix: (gain, pan),
            previous: None,
        }
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        (gain, pan): (f32, f32),
    ) -> Result<(f32, f32), EditorError> {
        let sequence = sequence_mut(project, sequence)?;
        // A picture has no volume: refused rather than stored where nothing
        // would ever read it.
        let missing = if sequence.track_kind(track).is_some() {
            EditorError::ClipKindMismatch
        } else {
            EditorError::TrackNotFound(track)
        };
        let track = sequence.audio_track_mut(track).ok_or(missing)?;
        let was = (track.gain, track.pan);
        // Clamped in the model, for §38.2's reason; a non-number becomes the
        // neutral value rather than silence or a hard pan.
        let finite = |v: f32, neutral: f32| if v.is_finite() { v } else { neutral };
        track.gain = finite(gain, 1.0).clamp(0.0, bettercut_timeline::MAX_TRACK_GAIN);
        track.pan = finite(pan, 0.0).clamp(-1.0, 1.0);
        Ok(was)
    }
}

impl EditorCommand for SetTrackMix {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(project, self.sequence, self.track, self.mix)?;
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Change track volume".to_owned()
    }
}

/// A glitch amount brought into range on the way in, for the journal's sake.
fn glitch_amount(amount: f32) -> f32 {
    if amount.is_finite() {
        amount.clamp(0.0, bettercut_timeline::MAX_GLITCH)
    } else {
        0.0
    }
}

/// Tag a clip with a colour, on whichever kind of lane it is.
#[derive(Debug)]
pub struct SetColorLabel {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    label: bettercut_timeline::ColorLabel,
    previous: Option<bettercut_timeline::ColorLabel>,
}

impl SetColorLabel {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        label: bettercut_timeline::ColorLabel,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            label,
            previous: None,
        }
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track_id: TrackId,
        clip_id: ClipId,
        label: bettercut_timeline::ColorLabel,
    ) -> Result<bettercut_timeline::ColorLabel, EditorError> {
        use bettercut_timeline::Clip;
        on_track!(project, sequence, track_id, |track| {
            let clip = track
                .get_mut(clip_id)
                .ok_or(EditorError::ClipNotFound(clip_id))?;
            let was = clip.color_label();
            clip.set_color_label(label);
            Ok(was)
        })
    }
}

impl EditorCommand for SetColorLabel {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let was = Self::apply(project, self.sequence, self.track, self.clip, self.label)?;
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let was = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, self.clip, was)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Colour Label".to_owned()
    }
}

/// Set a sound clip's fade in and fade out.
#[derive(Debug)]
pub struct SetClipFades {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    fades: (TimelineTime, TimelineTime),
    previous: Option<(TimelineTime, TimelineTime)>,
}

impl SetClipFades {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        fade_in: TimelineTime,
        fade_out: TimelineTime,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            fades: (fade_in, fade_out),
            previous: None,
        }
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        (fade_in, fade_out): (TimelineTime, TimelineTime),
    ) -> Result<(TimelineTime, TimelineTime), EditorError> {
        let sequence = sequence_mut(project, sequence)?;
        // A fade on a picture or a title is a request for something else.
        let missing = if sequence.track_kind(track).is_some() {
            EditorError::ClipKindMismatch
        } else {
            EditorError::TrackNotFound(track)
        };
        let track = sequence.audio_track_mut(track).ok_or(missing)?;
        let clip = track.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
        // Clamped here rather than in the panel, for §38.2's reason: the
        // journal replays commands, and a limit only the slider knew about
        // would come back unapplied. Longer than the clip is allowed — the
        // mixer fits the two into whatever length the clip has at the time,
        // so trimming a clip does not quietly throw its fades away.
        let limit = |t: TimelineTime| t.clamp(TimelineTime::ZERO, bettercut_timeline::MAX_FADE);
        let was = (clip.fade_in, clip.fade_out);
        clip.fade_in = limit(fade_in);
        clip.fade_out = limit(fade_out);
        Ok(was)
    }
}

impl EditorCommand for SetClipFades {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(project, self.sequence, self.track, self.clip, self.fades)?;
        // First execute only: a redo restores the state from before the whole
        // gesture, not from the previous redo step.
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, self.clip, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Change fade".to_owned()
    }
}

/// Put a transition on the end of a clip, or take it off (§25).
#[derive(Debug)]
pub struct SetTransition {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    transition: Option<Transition>,
    previous: Option<Option<Transition>>,
}

impl SetTransition {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        transition: Option<Transition>,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            transition,
            previous: None,
        }
    }

    /// Write it, returning what was there.
    ///
    /// The clamp happens here rather than in the interface because §38.2
    /// replays commands after a crash and §54 has the model own its own
    /// invariants: a duration checked only on the way in would come back
    /// unchecked on the way out.
    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        transition: Option<Transition>,
        clamp: bool,
    ) -> Result<Option<Transition>, EditorError> {
        let transition = match transition {
            Some(mut wanted) if clamp => {
                let room = transition_room(project, sequence, track, clip, wanted.kind)
                    .ok_or(EditorError::NoRoomForTransition)?;
                if room < MIN_TRANSITION {
                    return Err(EditorError::NoRoomForTransition);
                }
                wanted.duration = TimelineTime::from_ticks(
                    wanted
                        .duration
                        .ticks()
                        .clamp(MIN_TRANSITION.ticks(), room.ticks()),
                );
                Some(wanted)
            }
            other => other,
        };

        with_track(
            project,
            sequence,
            track,
            |video| {
                let clip = video.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
                Ok(std::mem::replace(&mut clip.transition_out, transition))
            },
            // §25 v1 is picture only. An audio crossfade is a different
            // mechanism — mixing two sources, not blending two images — and
            // accepting one here would store something nothing reads.
            |_audio| Err(EditorError::ClipKindMismatch),
        )
    }
}

impl EditorCommand for SetTransition {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            self.transition,
            true,
        )?;
        self.previous.get_or_insert(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        // Restored as it was, not re-clamped: it was legal when it was stored,
        // and re-clamping here would let an undo hand back something different
        // from what the user had.
        Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            previous,
            false,
        )?;
        Ok(())
    }

    fn label(&self) -> String {
        match self.transition {
            Some(t) => format!("Add {}", t.kind.label().to_lowercase()),
            None => "Remove transition".to_string(),
        }
    }
}

// --------------------------------------------------------------------------

#[derive(Debug)]
pub struct RenameProject {
    pub name: String,
    previous: Option<String>,
}

impl RenameProject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            previous: None,
        }
    }
}

impl EditorCommand for RenameProject {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        self.previous = Some(std::mem::replace(&mut project.name, self.name.clone()));
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        project.name = self.previous.take().ok_or(EditorError::NotExecuted)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Rename Project".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Set one property of one clip (§59).
///
/// Values are clamped on the way in rather than trusted. A slider cannot send
/// an opacity of 40, but the journal replays whatever was written and a
/// hand-edited project file is a supported input — §50's rule is that bad data
/// degrades rather than corrupts.
#[derive(Debug)]
pub struct SetClipProperty {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    property: ClipProperty,
    previous: Option<ClipProperty>,
}

impl SetClipProperty {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId, property: ClipProperty) -> Self {
        Self {
            sequence,
            track,
            clip,
            property,
            previous: None,
        }
    }

    /// True when `other` is the same slider being dragged: same clip, same
    /// property. Used to collapse a drag into one undo step (§11).
    pub fn is_same_gesture(&self, other: &Self) -> bool {
        self.clip == other.clip && self.property.kind() == other.property.kind()
    }

    /// Apply, returning what was there before.
    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        property: ClipProperty,
    ) -> Result<ClipProperty, EditorError> {
        with_track(
            project,
            sequence,
            track,
            |video| {
                let clip = video.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
                Ok(match property {
                    // Every limit below comes from `AnimatedParameter::limits`
                    // rather than a literal here. A keyed value goes through
                    // the same clamp, and two lists of numbers would eventually
                    // disagree — leaving an animated opacity able to reach 1.4
                    // where the slider stops at 1.0.
                    ClipProperty::Opacity(value) => {
                        let was = clip.opacity;
                        clip.opacity = AnimatedParameter::Opacity.clamp(value);
                        ClipProperty::Opacity(was)
                    }
                    ClipProperty::Position { x, y } => {
                        let was = clip.transform.position;
                        clip.transform.position = Vec2::new(
                            AnimatedParameter::PositionX.clamp(x),
                            AnimatedParameter::PositionY.clamp(y),
                        );
                        ClipProperty::Position { x: was.x, y: was.y }
                    }
                    ClipProperty::Scale { x, y } => {
                        let was = clip.transform.scale;
                        clip.transform.scale = Vec2::new(
                            AnimatedParameter::ScaleX.clamp(x),
                            AnimatedParameter::ScaleY.clamp(y),
                        );
                        ClipProperty::Scale { x: was.x, y: was.y }
                    }
                    ClipProperty::Rotation(value) => {
                        let was = clip.transform.rotation_degrees;
                        clip.transform.rotation_degrees = AnimatedParameter::Rotation.clamp(value);
                        ClipProperty::Rotation(was)
                    }
                    ClipProperty::Brightness(value) => {
                        let was = clip.color.brightness;
                        clip.color.brightness = AnimatedParameter::Brightness.clamp(value);
                        ClipProperty::Brightness(was)
                    }
                    ClipProperty::Contrast(value) => {
                        let was = clip.color.contrast;
                        clip.color.contrast = AnimatedParameter::Contrast.clamp(value);
                        ClipProperty::Contrast(was)
                    }
                    ClipProperty::Saturation(value) => {
                        let was = clip.color.saturation;
                        clip.color.saturation = AnimatedParameter::Saturation.clamp(value);
                        ClipProperty::Saturation(was)
                    }
                    ClipProperty::Temperature(value) => {
                        let was = clip.color.temperature;
                        clip.color.temperature = AnimatedParameter::Temperature.clamp(value);
                        ClipProperty::Temperature(was)
                    }
                    ClipProperty::Tint(value) => {
                        let was = clip.color.tint;
                        clip.color.tint = AnimatedParameter::Tint.clamp(value);
                        ClipProperty::Tint(was)
                    }
                    ClipProperty::CornerPin(pin) => {
                        let was = clip.corner_pin;
                        clip.corner_pin = pin.clamped();
                        ClipProperty::CornerPin(was)
                    }
                    ClipProperty::Vibrance(value) => {
                        let was = clip.color.vibrance;
                        clip.color.vibrance = value.clamp(-1.0, 1.0);
                        ClipProperty::Vibrance(was)
                    }
                    ClipProperty::Blur(value) => {
                        let was = clip.blur;
                        clip.blur = AnimatedParameter::Blur.clamp(value);
                        ClipProperty::Blur(was)
                    }
                    ClipProperty::Sharpen(value) => {
                        let was = clip.sharpen;
                        // Clamped on the way in, for the journal's sake.
                        clip.sharpen = if value.is_finite() {
                            value.clamp(0.0, bettercut_timeline::MAX_SHARPEN)
                        } else {
                            0.0
                        };
                        ClipProperty::Sharpen(was)
                    }
                    ClipProperty::Reverse(on) => {
                        let was = clip.reversed;
                        clip.reversed = on;
                        ClipProperty::Reverse(was)
                    }
                    ClipProperty::RgbSplit(amount) => {
                        let was = clip.rgb_split;
                        clip.rgb_split = glitch_amount(amount);
                        ClipProperty::RgbSplit(was)
                    }
                    ClipProperty::Glitch(amount) => {
                        let was = clip.glitch;
                        clip.glitch = glitch_amount(amount);
                        ClipProperty::Glitch(was)
                    }
                    ClipProperty::Pixelate(amount) => {
                        let was = clip.pixelate;
                        clip.pixelate = glitch_amount(amount);
                        ClipProperty::Pixelate(was)
                    }
                    ClipProperty::ZoomBlur(amount) => {
                        let was = clip.zoom_blur;
                        clip.zoom_blur = glitch_amount(amount);
                        ClipProperty::ZoomBlur(was)
                    }
                    ClipProperty::Glow(amount) => {
                        let was = clip.glow;
                        clip.glow = glitch_amount(amount);
                        ClipProperty::Glow(was)
                    }
                    ClipProperty::OldFilm(amount) => {
                        let was = clip.old_film;
                        clip.old_film = glitch_amount(amount);
                        ClipProperty::OldFilm(was)
                    }
                    ClipProperty::Tone(tone) => {
                        let was = clip.curves.tone;
                        clip.curves.tone = tone;
                        ClipProperty::Tone(was)
                    }
                    ClipProperty::Curves(curves) => {
                        let was = clip.curves;
                        clip.curves = curves.clamped();
                        ClipProperty::Curves(was)
                    }
                    ClipProperty::LightLeak(amount) => {
                        let was = clip.light_leak;
                        clip.light_leak = glitch_amount(amount);
                        ClipProperty::LightLeak(was)
                    }
                    ClipProperty::BeatPulse(amount) => {
                        let was = clip.beat_pulse;
                        clip.beat_pulse = glitch_amount(amount);
                        ClipProperty::BeatPulse(was)
                    }
                    ClipProperty::Reflection(kind) => {
                        let was = clip.reflection;
                        clip.reflection = kind;
                        ClipProperty::Reflection(was)
                    }
                    ClipProperty::Lut(lut) => {
                        let was = clip.lut;
                        clip.lut = lut.map(bettercut_timeline::ClipLut::clamped);
                        ClipProperty::Lut(was)
                    }
                    ClipProperty::Backdrop(backdrop) => {
                        let was = clip.backdrop;
                        clip.backdrop = backdrop;
                        ClipProperty::Backdrop(was)
                    }
                    ClipProperty::Blend(blend) => {
                        let was = clip.blend;
                        clip.blend = blend;
                        ClipProperty::Blend(was)
                    }
                    ClipProperty::MotionBlur(on) => {
                        let was = clip.motion_blur;
                        clip.motion_blur = on;
                        ClipProperty::MotionBlur(was)
                    }
                    ClipProperty::SmoothMotion(on) => {
                        let was = clip.smooth_motion;
                        clip.smooth_motion = on;
                        ClipProperty::SmoothMotion(was)
                    }
                    ClipProperty::Border(border) => {
                        let was = clip.border;
                        clip.border = border.clamped();
                        ClipProperty::Border(was)
                    }
                    ClipProperty::Shadow(shadow) => {
                        let was = clip.shadow;
                        clip.shadow = shadow.clamped();
                        ClipProperty::Shadow(was)
                    }
                    ClipProperty::Crop(crop) => {
                        let was = clip.crop;
                        // Clamped on the way in, like every other value a
                        // project file can carry: a crop that takes everything
                        // leaves a zero-sized quad.
                        clip.crop = crop.clamped();
                        ClipProperty::Crop(was)
                    }
                    ClipProperty::Flip { axis, on } => {
                        let flag = axis.flag(&mut clip.transform);
                        let was = *flag;
                        *flag = on;
                        ClipProperty::Flip { axis, on: was }
                    }
                    ClipProperty::Motion(motion) => {
                        let was = clip.motion;
                        // Lengths brought into range on the way in, for the
                        // reason `TextAnimation::sanitized` gives: a limit
                        // that lived only in the interface would come back
                        // unapplied when the journal replayed the command.
                        clip.motion = motion.sanitized();
                        ClipProperty::Motion(was)
                    }
                    ClipProperty::Mask(mask) => {
                        let was = clip.mask;
                        clip.mask = mask.map(bettercut_timeline::Mask::clamped);
                        ClipProperty::Mask(was)
                    }
                    ClipProperty::ChromaKey(key) => {
                        let was = clip.chroma_key;
                        // Clamped on the way in, like every other value a
                        // project file can carry: the shader trusts what it is
                        // given, and a tolerance of a million would key the
                        // whole picture.
                        clip.chroma_key = key.map(bettercut_timeline::ChromaKey::clamped);
                        ClipProperty::ChromaKey(was)
                    }
                    // Gain is an audio property; a video clip has none — and
                    // a background belongs to the sequence, not to a picture.
                    // And a vignette frames the whole frame, not one clip in
                    // it (`MasterLook::vignette`).
                    ClipProperty::Gain(_)
                    | ClipProperty::Denoise(_)
                    | ClipProperty::Eq(_)
                    | ClipProperty::Space(_)
                    | ClipProperty::Channels(_)
                    | ClipProperty::Pitch(_)
                    | ClipProperty::KeepPitch(_)
                    | ClipProperty::Leveller(_)
                    | ClipProperty::DeEss(_)
                    | ClipProperty::FadeShape(_)
                    | ClipProperty::Mute(_)
                    | ClipProperty::Crossfade(_)
                    | ClipProperty::Background(_)
                    | ClipProperty::Grain(_)
                    | ClipProperty::Bars(_)
                    | ClipProperty::ProgressBar(_)
                    | ClipProperty::BurnIn(_) => {
                        return Err(EditorError::ClipKindMismatch);
                    }
                    ClipProperty::Vignette(amount) => {
                        let was = clip.vignette;
                        clip.vignette = if amount.is_finite() {
                            amount.clamp(0.0, bettercut_timeline::MAX_VIGNETTE)
                        } else {
                            0.0
                        };
                        ClipProperty::Vignette(was)
                    }
                })
            },
            |audio| {
                let clip = audio.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
                match property {
                    // All three are about a picture; sound has none of them.
                    ClipProperty::Backdrop(_)
                    | ClipProperty::ChromaKey(_)
                    | ClipProperty::Mask(_)
                    | ClipProperty::Blend(_)
                    | ClipProperty::Motion(_)
                    | ClipProperty::MotionBlur(_)
                    | ClipProperty::SmoothMotion(_)
                    | ClipProperty::Border(_)
                    | ClipProperty::Shadow(_)
                    | ClipProperty::Flip { .. }
                    | ClipProperty::Crop(_)
                    | ClipProperty::Sharpen(_)
                    | ClipProperty::Lut(_)
                    | ClipProperty::RgbSplit(_)
                    | ClipProperty::Glitch(_)
                    | ClipProperty::Reflection(_)
                    | ClipProperty::Background(_) => Err(EditorError::ClipKindMismatch),
                    ClipProperty::Gain(value) => {
                        let was = clip.gain;
                        clip.gain = value.clamp(0.0, 4.0);
                        Ok(ClipProperty::Gain(was))
                    }
                    ClipProperty::Reverse(on) => {
                        let was = clip.reversed;
                        clip.reversed = on;
                        Ok(ClipProperty::Reverse(was))
                    }
                    ClipProperty::Denoise(amount) => {
                        let was = clip.denoise;
                        clip.denoise = if amount.is_finite() {
                            amount.clamp(0.0, bettercut_timeline::MAX_DENOISE)
                        } else {
                            0.0
                        };
                        Ok(ClipProperty::Denoise(was))
                    }
                    ClipProperty::Eq(eq) => {
                        let was = clip.eq;
                        clip.eq = eq.clamped();
                        Ok(ClipProperty::Eq(was))
                    }
                    ClipProperty::Space(space) => {
                        let was = clip.space;
                        clip.space = space.clamped();
                        Ok(ClipProperty::Space(was))
                    }
                    ClipProperty::Channels(mode) => {
                        let was = clip.channels;
                        clip.channels = mode;
                        Ok(ClipProperty::Channels(was))
                    }
                    ClipProperty::KeepPitch(on) => {
                        let was = clip.keep_pitch;
                        clip.keep_pitch = on;
                        Ok(ClipProperty::KeepPitch(was))
                    }
                    ClipProperty::Mute(muted) => {
                        let was = clip.muted;
                        clip.muted = muted;
                        Ok(ClipProperty::Mute(was))
                    }
                    ClipProperty::FadeShape(shape) => {
                        let was = clip.fade_shape;
                        clip.fade_shape = shape;
                        Ok(ClipProperty::FadeShape(was))
                    }
                    ClipProperty::DeEss(amount) => {
                        let was = clip.de_ess;
                        clip.de_ess = if amount.is_finite() {
                            amount.clamp(0.0, 100.0)
                        } else {
                            0.0
                        };
                        Ok(ClipProperty::DeEss(was))
                    }
                    ClipProperty::Leveller(amount) => {
                        let was = clip.leveller;
                        clip.leveller = if amount.is_finite() {
                            amount.clamp(0.0, 100.0)
                        } else {
                            0.0
                        };
                        Ok(ClipProperty::Leveller(was))
                    }
                    ClipProperty::Pitch(semitones) => {
                        let was = clip.pitch;
                        clip.pitch = if semitones.is_finite() {
                            // Whole tenths, so a slider's float noise does not
                            // leave "0.0001 semitones" that is not quite off.
                            (semitones.clamp(-12.0, 12.0) * 10.0).round() / 10.0
                        } else {
                            0.0
                        };
                        Ok(ClipProperty::Pitch(was))
                    }
                    ClipProperty::Crossfade(length) => {
                        let was = clip.crossfade_out;
                        clip.crossfade_out = TimelineTime::from_ticks(
                            length
                                .ticks()
                                .clamp(0, bettercut_timeline::MAX_CROSSFADE.ticks()),
                        );
                        Ok(ClipProperty::Crossfade(was))
                    }
                    _ => Err(EditorError::ClipKindMismatch),
                }
            },
        )
    }
}

/// Add, replace, or delete one keyframe (§24).
///
/// One command for both directions: removing a key is setting it to nothing,
/// and writing them as two types would duplicate the undo logic, which is the
/// part with the subtlety in it — undoing an *insert* has to delete, while
/// undoing a *replace* has to put the old key back.
/// Replace a sound clip's whole volume envelope (§24).
#[derive(Debug)]
pub struct SetGainEnvelope {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    keys: Vec<Keyframe>,
    /// What was on the parameter before the first execute. The outer `Option`
    /// is "has this run"; the inner is "was there an envelope".
    previous: Option<Option<bettercut_timeline::KeyframeTrack>>,
}

impl SetGainEnvelope {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId, keys: Vec<Keyframe>) -> Self {
        Self {
            sequence,
            track,
            clip,
            keys,
            previous: None,
        }
    }

    fn with_clip<T>(
        &self,
        project: &mut Project,
        act: impl FnOnce(&mut bettercut_timeline::AudioClip) -> T,
    ) -> Result<T, EditorError> {
        with_track(
            project,
            self.sequence,
            self.track,
            // Volume belongs to sound. A picture clip's own audio is a
            // separate, linked clip (§12), and that is the one to envelope.
            |_video| Err(EditorError::ClipKindMismatch),
            |audio| {
                let clip = audio
                    .get_mut(self.clip)
                    .ok_or(EditorError::ClipNotFound(self.clip))?;
                Ok(act(clip))
            },
        )
    }
}

impl EditorCommand for SetGainEnvelope {
    fn label(&self) -> String {
        if self.keys.is_empty() {
            "Clear Volume Envelope".to_owned()
        } else {
            "Volume Envelope".to_owned()
        }
    }

    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let keys: Vec<Keyframe> = self
            .keys
            .iter()
            .map(|key| {
                let mut key = *key;
                key.value = AnimatedParameter::Gain.clamp(key.value);
                key
            })
            .collect();
        let previous = self.with_clip(project, |clip| {
            clip.keyframes.replace(AnimatedParameter::Gain, keys)
        })?;
        // Only the first execute records what was there; a redo must not
        // remember the envelope it just wrote.
        if self.previous.is_none() {
            self.previous = Some(previous);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        self.with_clip(project, |clip| {
            clip.keyframes.restore(AnimatedParameter::Gain, previous);
        })
    }
}

#[derive(Debug)]
pub struct SetKeyframe {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    parameter: AnimatedParameter,
    time: MediaTime,
    /// The key to write; `None` deletes whatever is at `time`.
    key: Option<Keyframe>,
    /// What was at `time` before the first execute. The outer `Option` is
    /// "has this run"; the inner is "was there a key there".
    previous: Option<Option<Keyframe>>,
}

impl SetKeyframe {
    pub fn set(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        parameter: AnimatedParameter,
        key: Keyframe,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            parameter,
            time: key.time,
            key: Some(key),
            previous: None,
        }
    }

    pub fn remove(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        parameter: AnimatedParameter,
        time: MediaTime,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            parameter,
            time,
            key: None,
            previous: None,
        }
    }

    /// Write `key` (or delete, when `None`), returning what was there.
    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        parameter: AnimatedParameter,
        time: MediaTime,
        key: Option<Keyframe>,
    ) -> Result<Option<Keyframe>, EditorError> {
        with_track(
            project,
            sequence,
            track,
            |video| {
                let clip = video.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
                Ok(match key {
                    Some(mut key) => {
                        key.value = parameter.clamp(key.value);
                        key.time = time;
                        clip.keyframes.set(parameter, key)
                    }
                    None => clip.keyframes.remove(parameter, time),
                })
            },
            // §59's keyframeable clip gain: the one parameter a sound clip
            // has. The rest describe a picture, and accepting a key nothing
            // would ever read would be worse than refusing it.
            |audio| {
                if parameter != AnimatedParameter::Gain {
                    return Err(EditorError::ClipKindMismatch);
                }
                let clip = audio.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
                Ok(match key {
                    Some(mut key) => {
                        key.value = parameter.clamp(key.value);
                        key.time = time;
                        clip.keyframes.set(parameter, key)
                    }
                    None => clip.keyframes.remove(parameter, time),
                })
            },
        )
    }
}

impl EditorCommand for SetKeyframe {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            self.parameter,
            self.time,
            self.key,
        )?;
        // First execute only, for the same reason as `SetClipProperty`: a redo
        // must restore what was there before the gesture, not before the redo.
        self.previous.get_or_insert(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(
            project,
            self.sequence,
            self.track,
            self.clip,
            self.parameter,
            self.time,
            previous,
        )?;
        Ok(())
    }

    fn label(&self) -> String {
        match self.key {
            Some(_) => format!("Keyframe {}", self.parameter.label()),
            None => format!("Delete {} keyframe", self.parameter.label()),
        }
    }
}

impl EditorCommand for SetClipProperty {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(project, self.sequence, self.track, self.clip, self.property)?;
        // Only on the first execute: a redo must restore the value from before
        // the whole gesture, not from the previous redo step.
        self.previous.get_or_insert(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, self.clip, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        format!("Change {}", self.property.kind())
    }
}

// --------------------------------------------------------------------------

/// Point an asset at a moved file (§66).
///
/// Only the location changes. Everything decoded from the file — duration,
/// resolution, colour, codecs — is left exactly as imported, because relinking
/// is meant to restore a project to working order, not to silently redefine the
/// media underneath clips that were cut against it. A genuinely different file
/// should be imported, not relinked.
#[derive(Debug)]
pub struct RelinkMedia {
    media: MediaId,
    path: std::path::PathBuf,
    file_size: u64,
    /// Where it pointed before, so undo restores the broken state exactly —
    /// including `missing`, which is what makes the undo honest.
    previous: Option<(std::path::PathBuf, String, u64, bool)>,
}

impl RelinkMedia {
    pub fn new(media: MediaId, path: std::path::PathBuf, file_size: u64) -> Self {
        Self {
            media,
            path,
            file_size,
            previous: None,
        }
    }
}

/// Adjust the whole sequence's finished picture (§22).
#[derive(Debug)]
pub struct SetSequenceProperty {
    sequence: SequenceId,
    property: ClipProperty,
    previous: Option<ClipProperty>,
}

impl SetSequenceProperty {
    pub fn new(sequence: SequenceId, property: ClipProperty) -> Self {
        Self {
            sequence,
            property,
            previous: None,
        }
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        property: ClipProperty,
    ) -> Result<ClipProperty, EditorError> {
        use bettercut_timeline::AnimatedParameter as A;

        let sequence = sequence_mut(project, sequence)?;
        // §20a.4's master gain lives beside the picture adjustments, not among
        // them; see `Sequence::master_volume`.
        if let ClipProperty::Gain(value) = property {
            let was = sequence.master_volume;
            sequence.master_volume =
                value.clamp(0.0, bettercut_timeline::sequence::MAX_MASTER_VOLUME);
            return Ok(ClipProperty::Gain(was));
        }
        let master = &mut sequence.master;
        // Clamped through the same limits a clip uses, so the master cannot
        // reach a value the controls could not express.
        Ok(match property {
            ClipProperty::Opacity(value) => {
                let was = master.opacity;
                master.opacity = A::Opacity.clamp(value);
                ClipProperty::Opacity(was)
            }
            ClipProperty::Position { x, y } => {
                let was = master.transform.position;
                master.transform.position = Vec2::new(A::PositionX.clamp(x), A::PositionY.clamp(y));
                ClipProperty::Position { x: was.x, y: was.y }
            }
            ClipProperty::Scale { x, y } => {
                let was = master.transform.scale;
                master.transform.scale = Vec2::new(A::ScaleX.clamp(x), A::ScaleY.clamp(y));
                ClipProperty::Scale { x: was.x, y: was.y }
            }
            ClipProperty::Rotation(value) => {
                let was = master.transform.rotation_degrees;
                master.transform.rotation_degrees = A::Rotation.clamp(value);
                ClipProperty::Rotation(was)
            }
            ClipProperty::Brightness(value) => {
                let was = master.color.brightness;
                master.color.brightness = A::Brightness.clamp(value);
                ClipProperty::Brightness(was)
            }
            ClipProperty::Contrast(value) => {
                let was = master.color.contrast;
                master.color.contrast = A::Contrast.clamp(value);
                ClipProperty::Contrast(was)
            }
            ClipProperty::Saturation(value) => {
                let was = master.color.saturation;
                master.color.saturation = A::Saturation.clamp(value);
                ClipProperty::Saturation(was)
            }
            ClipProperty::Temperature(value) => {
                let was = master.color.temperature;
                master.color.temperature = A::Temperature.clamp(value);
                ClipProperty::Temperature(was)
            }
            ClipProperty::Tint(value) => {
                let was = master.color.tint;
                master.color.tint = A::Tint.clamp(value);
                ClipProperty::Tint(was)
            }
            ClipProperty::Vibrance(value) => {
                let was = master.color.vibrance;
                master.color.vibrance = value.clamp(-1.0, 1.0);
                ClipProperty::Vibrance(was)
            }
            ClipProperty::Blur(value) => {
                let was = master.blur;
                master.blur = A::Blur.clamp(value);
                ClipProperty::Blur(was)
            }
            // Both belong to a clip: a backdrop is made from its own
            // picture, and a key is about the screen it was shot against.
            ClipProperty::Backdrop(_)
            | ClipProperty::ChromaKey(_)
            | ClipProperty::Mask(_)
            | ClipProperty::Blend(_)
            // And an animation belongs to a clip because it is timed against
            // that clip's own start and end; the master has neither.
            | ClipProperty::Motion(_)
            | ClipProperty::MotionBlur(_)
            | ClipProperty::SmoothMotion(_)
            | ClipProperty::Border(_)
            | ClipProperty::Shadow(_)
            // A corner pin sets one picture into another; the frame has no
            // corners of its own to move.
            | ClipProperty::CornerPin(_)
            // Pitch belongs to a sound clip, and the master mixes them all.
            | ClipProperty::KeepPitch(_)
            // A mirror is about a shot — which way the subject faces. Mirroring
            // the whole sequence would reverse every title's lettering with it.
            | ClipProperty::Flip { .. }
            // And a crop is about a shot: §36's canvas shape is how a whole
            // sequence is re-framed, not this.
            | ClipProperty::Crop(_)
            // Sharpening is a decision about one shot's own detail.
            | ClipProperty::Sharpen(_)
            // A LUT grades footage as shot; the finished picture has the
            // colour controls and adjustment layers for that.
            | ClipProperty::Lut(_)
            // Reversing belongs to a clip's material, not the finished picture.
            | ClipProperty::Reverse(_)
            // A glitch breaks up a shot, not the finished picture.
            | ClipProperty::RgbSplit(_)
            | ClipProperty::Glitch(_)
            | ClipProperty::Pixelate(_)
            | ClipProperty::ZoomBlur(_)
            | ClipProperty::Glow(_)
            | ClipProperty::OldFilm(_)
            | ClipProperty::Tone(_)
            | ClipProperty::Curves(_)
            | ClipProperty::LightLeak(_)
            | ClipProperty::BeatPulse(_)
            // A reflection is made of one shot's own picture.
            | ClipProperty::Reflection(_)
            // Cleaning up a voice is about one recording, not the whole mix.
            | ClipProperty::Denoise(_)
            | ClipProperty::Eq(_)
            | ClipProperty::Space(_)
            | ClipProperty::Channels(_)
            | ClipProperty::Pitch(_)
            | ClipProperty::Leveller(_)
            | ClipProperty::DeEss(_)
            | ClipProperty::FadeShape(_)
            | ClipProperty::Mute(_)
            | ClipProperty::Crossfade(_) => return Err(EditorError::ClipKindMismatch),
            // §22: the one property here that is the master's alone.
            ClipProperty::Background(colour) => {
                let was = master.background;
                master.background = [
                    colour[0].clamp(0.0, 1.0),
                    colour[1].clamp(0.0, 1.0),
                    colour[2].clamp(0.0, 1.0),
                ];
                ClipProperty::Background(was)
            }
            ClipProperty::Vignette(amount) => {
                let was = master.vignette;
                master.vignette = if amount.is_finite() {
                    amount.clamp(0.0, bettercut_timeline::MAX_VIGNETTE)
                } else {
                    0.0
                };
                ClipProperty::Vignette(was)
            }
            ClipProperty::Grain(amount) => {
                let was = master.grain;
                master.grain = if amount.is_finite() {
                    amount.clamp(0.0, bettercut_timeline::MAX_GRAIN)
                } else {
                    0.0
                };
                ClipProperty::Grain(was)
            }
            ClipProperty::Bars(shape) => {
                let was = master.bars;
                master.bars = if shape.is_finite() {
                    shape.clamp(0.0, bettercut_timeline::MAX_BARS_ASPECT)
                } else {
                    0.0
                };
                ClipProperty::Bars(was)
            }
            ClipProperty::BurnIn(burn) => {
                let was = master.burn_in;
                master.burn_in = burn.clamped();
                ClipProperty::BurnIn(was)
            }
            ClipProperty::ProgressBar(bar) => {
                let was = master.progress_bar;
                master.progress_bar = bar.clamped();
                ClipProperty::ProgressBar(was)
            }
            // Handled above, before `master` was borrowed.
            ClipProperty::Gain(_) => unreachable!("master volume returns early"),
        })
    }
}

impl EditorCommand for SetSequenceProperty {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = Self::apply(project, self.sequence, self.property)?;
        self.previous.get_or_insert(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        format!("Change {} for the whole video", self.property.kind())
    }
}

/// Remove an asset from the library, holding it so undo can put it back.
#[derive(Debug)]
pub struct RemoveMedia {
    media: MediaId,
    /// The asset itself, captured on execute. §11: the undo payload is derived
    /// at execute time and never written to the journal.
    removed: Option<Box<bettercut_media::MediaAsset>>,
}

impl RemoveMedia {
    pub fn new(media: MediaId) -> Self {
        Self {
            media,
            removed: None,
        }
    }
}

impl EditorCommand for RemoveMedia {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = project.remove_media(self.media)?;
        self.removed = Some(Box::new(asset));
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = self.removed.take().ok_or(EditorError::NotExecuted)?;
        project.add_media(*asset);
        Ok(())
    }

    fn label(&self) -> String {
        "Remove Media".to_owned()
    }
}

impl EditorCommand for RelinkMedia {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = project
            .media
            .iter_mut()
            .find(|m| m.id == self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;

        self.previous = Some((
            asset.path.clone(),
            asset.file_name.clone(),
            asset.file_size,
            asset.missing,
        ));

        // The name is taken from the new path: a user who renamed the file as
        // well as moving it would otherwise keep seeing the old name forever.
        asset.file_name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| asset.file_name.clone());
        asset.path = self.path.clone();
        asset.file_size = self.file_size;
        asset.missing = false;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (path, file_name, file_size, missing) =
            self.previous.take().ok_or(EditorError::NotExecuted)?;
        let asset = project
            .media
            .iter_mut()
            .find(|m| m.id == self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;

        asset.path = path;
        asset.file_name = file_name;
        asset.file_size = file_size;
        asset.missing = missing;
        Ok(())
    }

    fn label(&self) -> String {
        format!(
            "Relink {}",
            self.path
                .file_name()
                .map_or_else(|| "media".to_owned(), |n| n.to_string_lossy().into_owned())
        )
    }
}

// --------------------------------------------------------------------------

/// Choose which angle of a multicam clip plays.
#[derive(Debug)]
pub struct SetClipAngle {
    pub(crate) sequence: SequenceId,
    pub(crate) track: TrackId,
    pub(crate) clip: ClipId,
    pub(crate) angle: Option<usize>,
    pub(crate) previous: Option<Option<usize>>,
}

impl EditorCommand for SetClipAngle {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let angle = self.angle;
        let was = on_track!(project, self.sequence, self.track, |track| {
            let clip = track
                .get_mut(self.clip)
                .ok_or(EditorError::ClipNotFound(self.clip))?;
            Ok(clip.set_angle(angle))
        })?;
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.take().ok_or(EditorError::NotExecuted)?;
        on_track!(project, self.sequence, self.track, |track| {
            let clip = track
                .get_mut(self.clip)
                .ok_or(EditorError::ClipNotFound(self.clip))?;
            clip.set_angle(previous);
            Ok(())
        })
    }

    fn label(&self) -> String {
        match self.angle {
            Some(angle) => format!("Angle {}", angle + 1),
            None => "All Angles".to_owned(),
        }
    }
}

// --------------------------------------------------------------------------

/// Choose a sequence's cover frame, or clear it.
#[derive(Debug)]
pub struct SetCoverFrame {
    pub(crate) sequence: SequenceId,
    pub(crate) at: Option<TimelineTime>,
    pub(crate) previous: Option<Option<TimelineTime>>,
}

impl EditorCommand for SetCoverFrame {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let was = std::mem::replace(&mut sequence.cover_frame, self.at);
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.sequence)?.cover_frame = previous;
        Ok(())
    }

    fn label(&self) -> String {
        if self.at.is_some() {
            "Set Cover Frame".to_owned()
        } else {
            "Clear Cover Frame".to_owned()
        }
    }
}

/// Put a watermark on a sequence, or take it off.
#[derive(Debug)]
pub struct SetWatermark {
    pub(crate) sequence: SequenceId,
    pub(crate) watermark: Option<bettercut_timeline::watermark::Watermark>,
    pub(crate) previous: Option<Option<bettercut_timeline::watermark::Watermark>>,
}

impl EditorCommand for SetWatermark {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let was = std::mem::replace(&mut sequence.watermark, self.watermark);
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.sequence)?.watermark = previous;
        Ok(())
    }

    fn label(&self) -> String {
        if self.watermark.is_some() {
            "Set Watermark".to_owned()
        } else {
            "Remove Watermark".to_owned()
        }
    }
}

/// Put a visualizer over a sequence, or take it off.
#[derive(Debug)]
pub struct SetVisualizer {
    sequence: SequenceId,
    visualizer: Option<bettercut_timeline::visualizer::Visualizer>,
    previous: Option<Option<bettercut_timeline::visualizer::Visualizer>>,
}

impl SetVisualizer {
    pub fn new(
        sequence: SequenceId,
        visualizer: Option<bettercut_timeline::visualizer::Visualizer>,
    ) -> Self {
        Self {
            sequence,
            visualizer: visualizer.map(bettercut_timeline::visualizer::Visualizer::clamped),
            previous: None,
        }
    }
}

impl EditorCommand for SetVisualizer {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let was = std::mem::replace(&mut sequence.visualizer, self.visualizer.clone());
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.sequence)?.visualizer = previous;
        Ok(())
    }

    fn label(&self) -> String {
        if self.visualizer.is_some() {
            "Set Visualizer".to_owned()
        } else {
            "Remove Visualizer".to_owned()
        }
    }
}

/// Set a sequence's resolution and frame rate (§8, §36).
///
/// # Clips do not move
///
/// Positions are absolute ticks (§9), not frame numbers, so changing the rate
/// leaves every clip exactly where it was in time. What changes is which
/// instants count as frame boundaries — a cut made at frame 30 of a 30 fps
/// sequence lands between frames at 25 fps.
///
/// Nothing is re-snapped, deliberately. Snapping would move the user's cuts
/// without being asked, and §9's whole point is that a tick is exact whether or
/// not it falls on a frame line. Future edits snap to the new grid; existing
/// ones are left alone.
#[derive(Debug)]
pub struct SetSequenceFormat {
    sequence: SequenceId,
    resolution: Resolution,
    frame_rate: FrameRate,
    previous: Option<(Resolution, FrameRate)>,
}

impl SetSequenceFormat {
    pub fn new(sequence: SequenceId, resolution: Resolution, frame_rate: FrameRate) -> Self {
        Self {
            sequence,
            resolution,
            frame_rate,
            previous: None,
        }
    }
}

impl EditorCommand for SetSequenceFormat {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        // Validate before touching anything: a rate the timebase cannot divide
        // exactly would make every frame boundary in the sequence drift (§9).
        if ticks_per_frame(self.frame_rate).is_none() {
            return Err(EditorError::Timeline(
                bettercut_timeline::TimelineError::UnrepresentableFrameRate {
                    rate: self.frame_rate.to_string(),
                },
            ));
        }
        if self.resolution.width == 0 || self.resolution.height == 0 {
            return Err(EditorError::Timeline(
                bettercut_timeline::TimelineError::UnrepresentableFrameRate {
                    rate: format!(
                        "resolution {}x{} has a zero dimension",
                        self.resolution.width, self.resolution.height
                    ),
                },
            ));
        }

        let sequence = sequence_mut(project, self.sequence)?;
        self.previous = Some((sequence.resolution, sequence.frame_rate));
        sequence.resolution = self.resolution;
        sequence.frame_rate = self.frame_rate;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (resolution, frame_rate) = self.previous.take().ok_or(EditorError::NotExecuted)?;
        let sequence = sequence_mut(project, self.sequence)?;
        sequence.resolution = resolution;
        sequence.frame_rate = frame_rate;
        Ok(())
    }

    fn label(&self) -> String {
        format!(
            "Set Format to {}x{} @ {}",
            self.resolution.width, self.resolution.height, self.frame_rate
        )
    }
}

// --------------------------------------------------------------------------

/// Change one project setting (§13, §67).
///
/// Goes through the command path like every other mutation, for two reasons
/// beyond §54's rule: turning proxies off mid-session is exactly the kind of
/// thing a user does by accident and wants back, and §38.2's journal has to
/// replay it or a recovered session would silently revert to the default.
#[derive(Debug)]
pub struct ChangeSetting {
    change: SettingChange,
    /// The value that was there before, captured at execute time.
    previous: Option<SettingChange>,
}

impl ChangeSetting {
    pub fn new(change: SettingChange) -> Self {
        Self {
            change,
            previous: None,
        }
    }

    /// Write `change` into `project`, returning what it replaced.
    fn apply(project: &mut Project, change: SettingChange) -> SettingChange {
        let settings = &mut project.settings;
        match change {
            SettingChange::AutoGenerateProxies(value) => SettingChange::AutoGenerateProxies(
                std::mem::replace(&mut settings.auto_generate_proxies, value),
            ),
            SettingChange::PerformanceMode(value) => SettingChange::PerformanceMode(
                std::mem::replace(&mut settings.performance_mode, value),
            ),
            SettingChange::CacheLimitBytes(value) => SettingChange::CacheLimitBytes(
                std::mem::replace(&mut settings.cache_limit_bytes, value),
            ),
            SettingChange::MagneticTimeline(value) => SettingChange::MagneticTimeline(
                std::mem::replace(&mut settings.magnetic_timeline, value),
            ),
            SettingChange::PhotoLength(value) => SettingChange::PhotoLength(std::mem::replace(
                &mut settings.photo_length,
                value.clamp(
                    bettercut_project_format::MIN_PHOTO_LENGTH,
                    bettercut_project_format::MAX_PHOTO_LENGTH,
                ),
            )),
        }
    }
}

impl EditorCommand for ChangeSetting {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        self.previous = Some(Self::apply(project, self.change));
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.take().ok_or(EditorError::NotExecuted)?;
        Self::apply(project, previous);
        Ok(())
    }

    fn label(&self) -> String {
        format!("Change {}", self.change.label())
    }
}

// --------------------------------------------------------------------------

#[derive(Debug)]
pub struct AddTrack {
    pub sequence: SequenceId,
    pub kind: TrackKind,
    pub name: String,
    /// Fixed at construction, so a redo re-creates the *same* track.
    pub id: TrackId,
    executed: bool,
}

impl AddTrack {
    pub fn new(
        sequence: SequenceId,
        kind: TrackKind,
        name: impl Into<String>,
        id: TrackId,
    ) -> Self {
        Self {
            sequence,
            kind,
            name: name.into(),
            id,
            executed: false,
        }
    }
}

impl EditorCommand for AddTrack {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        match self.kind {
            TrackKind::Video => {
                let mut track = VideoTrack::new(self.name.clone());
                track.id = self.id;
                sequence.video_tracks.push(track);
            }
            TrackKind::Audio => {
                let mut track = AudioTrack::new(self.name.clone());
                track.id = self.id;
                sequence.audio_tracks.push(track);
            }
            TrackKind::Text => {
                let mut track = bettercut_timeline::TextTrack::new(self.name.clone());
                track.id = self.id;
                sequence.text_tracks.push(track);
            }
            TrackKind::Adjustment => {
                let mut track = bettercut_timeline::AdjustmentTrack::new(self.name.clone());
                track.id = self.id;
                sequence.adjustment_tracks.push(track);
            }
        }
        self.executed = true;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if !self.executed {
            return Err(EditorError::NotExecuted);
        }
        let id = self.id;
        let sequence = sequence_mut(project, self.sequence)?;
        match self.kind {
            TrackKind::Video => sequence.video_tracks.retain(|t| t.id != id),
            TrackKind::Audio => sequence.audio_tracks.retain(|t| t.id != id),
            TrackKind::Text => sequence.text_tracks.retain(|t| t.id != id),
            TrackKind::Adjustment => sequence.adjustment_tracks.retain(|t| t.id != id),
        }
        self.executed = false;
        Ok(())
    }

    fn label(&self) -> String {
        match self.kind {
            TrackKind::Video => "Add Video Track".to_owned(),
            TrackKind::Audio => "Add Audio Track".to_owned(),
            TrackKind::Text => "Add Text Track".to_owned(),
            TrackKind::Adjustment => "Add Adjustment Track".to_owned(),
        }
    }
}

// --------------------------------------------------------------------------

#[derive(Debug)]
pub struct RemoveTrack {
    pub sequence: SequenceId,
    pub track: TrackId,
    /// The whole track, so undo restores its clips too — not just an empty one.
    removed: Option<(usize, TrackPayload)>,
}

impl RemoveTrack {
    pub fn new(sequence: SequenceId, track: TrackId) -> Self {
        Self {
            sequence,
            track,
            removed: None,
        }
    }
}

impl EditorCommand for RemoveTrack {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;

        if let Some(index) = sequence
            .video_tracks
            .iter()
            .position(|t| t.id == self.track)
        {
            let track = sequence.video_tracks.remove(index);
            self.removed = Some((index, TrackPayload::Video(Box::new(track))));
            return Ok(());
        }
        if let Some(index) = sequence
            .audio_tracks
            .iter()
            .position(|t| t.id == self.track)
        {
            let track = sequence.audio_tracks.remove(index);
            self.removed = Some((index, TrackPayload::Audio(Box::new(track))));
            return Ok(());
        }
        if let Some(index) = sequence.text_tracks.iter().position(|t| t.id == self.track) {
            let track = sequence.text_tracks.remove(index);
            self.removed = Some((index, TrackPayload::Text(Box::new(track))));
            return Ok(());
        }
        // Not caught by the compiler: this is a chain of lookups, not a match,
        // and a missing branch here reads as "track not found" for a track
        // that is plainly there.
        if let Some(index) = sequence
            .adjustment_tracks
            .iter()
            .position(|t| t.id == self.track)
        {
            let track = sequence.adjustment_tracks.remove(index);
            self.removed = Some((index, TrackPayload::Adjustment(Box::new(track))));
            return Ok(());
        }
        Err(EditorError::TrackNotFound(self.track))
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (index, payload) = self.removed.take().ok_or(EditorError::NotExecuted)?;
        let sequence = sequence_mut(project, self.sequence)?;
        // Restore at the original index: track order is compositing order (§22).
        match payload {
            TrackPayload::Video(track) => {
                let index = index.min(sequence.video_tracks.len());
                sequence.video_tracks.insert(index, *track);
            }
            TrackPayload::Audio(track) => {
                let index = index.min(sequence.audio_tracks.len());
                sequence.audio_tracks.insert(index, *track);
            }
            TrackPayload::Text(track) => {
                let index = index.min(sequence.text_tracks.len());
                sequence.text_tracks.insert(index, *track);
            }
            TrackPayload::Adjustment(track) => {
                let index = index.min(sequence.adjustment_tracks.len());
                sequence.adjustment_tracks.insert(index, *track);
            }
        }
        Ok(())
    }

    fn label(&self) -> String {
        "Remove Track".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Put a prepared sequence into the project.
#[derive(Debug)]
pub struct AddSequence {
    index: usize,
    id: SequenceId,
    /// Held until executed, and again after an undo, so a redo adds the very
    /// same sequence.
    sequence: Option<Box<Sequence>>,
}

impl AddSequence {
    pub fn new(sequence: Box<Sequence>, index: usize) -> Self {
        Self {
            index,
            id: sequence.id,
            sequence: Some(sequence),
        }
    }
}

impl EditorCommand for AddSequence {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if project.sequence(self.id).is_some() {
            return Err(EditorError::SequenceAlreadyExists(self.id));
        }
        let sequence = self.sequence.take().ok_or(EditorError::NotExecuted)?;
        let index = self.index.min(project.sequences.len());
        project.sequences.insert(index, *sequence);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let index = project
            .sequences
            .iter()
            .position(|s| s.id == self.id)
            .ok_or(EditorError::SequenceNotFound(self.id))?;
        let sequence = project.sequences.remove(index);
        // Undoing the sequence being looked at leaves the one beside it on
        // screen rather than nothing.
        if project.active_sequence == Some(self.id) {
            project.active_sequence = project.sequences.get(index.saturating_sub(1)).map(|s| s.id);
        }
        self.sequence = Some(Box::new(sequence));
        Ok(())
    }

    fn label(&self) -> String {
        "Add Sequence".to_owned()
    }
}

/// Take a sequence out of the project.
#[derive(Debug)]
pub struct RemoveSequence {
    id: SequenceId,
    removed: Option<(usize, Box<Sequence>, bool)>,
}

impl RemoveSequence {
    pub fn new(id: SequenceId) -> Self {
        Self { id, removed: None }
    }
}

impl EditorCommand for RemoveSequence {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if project.sequences.len() <= 1 {
            return Err(EditorError::LastSequence);
        }
        let index = project
            .sequences
            .iter()
            .position(|s| s.id == self.id)
            .ok_or(EditorError::SequenceNotFound(self.id))?;
        let sequence = project.sequences.remove(index);
        let was_active = project.active_sequence == Some(self.id);
        if was_active {
            project.active_sequence = project
                .sequences
                .get(index.min(project.sequences.len() - 1))
                .map(|s| s.id);
        }
        self.removed = Some((index, Box::new(sequence), was_active));
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (index, sequence, was_active) = self.removed.take().ok_or(EditorError::NotExecuted)?;
        let index = index.min(project.sequences.len());
        project.sequences.insert(index, *sequence);
        if was_active {
            project.active_sequence = Some(self.id);
        }
        Ok(())
    }

    fn label(&self) -> String {
        "Remove Sequence".to_owned()
    }
}

/// Give a sequence a new name.
#[derive(Debug)]
pub struct RenameSequence {
    id: SequenceId,
    name: String,
    previous: Option<String>,
}

impl RenameSequence {
    pub fn new(id: SequenceId, name: String) -> Self {
        Self {
            id,
            name,
            previous: None,
        }
    }
}

impl EditorCommand for RenameSequence {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.id)?;
        let was = std::mem::replace(&mut sequence.name, self.name.clone());
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        sequence_mut(project, self.id)?.name = previous;
        Ok(())
    }

    fn label(&self) -> String {
        "Rename Sequence".to_owned()
    }
}

/// File a file in a bin.
/// Rate a file 0–5.
#[derive(Debug)]
pub struct SetMediaRating {
    media: bettercut_foundation::MediaId,
    rating: u8,
    previous: Option<u8>,
}

impl SetMediaRating {
    pub fn new(media: bettercut_foundation::MediaId, rating: u8) -> Self {
        Self {
            media,
            // Held here rather than trusted: a project file could say nine.
            rating: rating.min(5),
            previous: None,
        }
    }
}

impl EditorCommand for SetMediaRating {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;
        let was = std::mem::replace(&mut asset.rating, self.rating);
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.take().ok_or(EditorError::NotExecuted)?;
        project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?
            .rating = previous;
        Ok(())
    }

    fn label(&self) -> String {
        match self.rating {
            0 => "Clear Rating".to_owned(),
            1 => "Rate 1 Star".to_owned(),
            stars => format!("Rate {stars} Stars"),
        }
    }
}

// --------------------------------------------------------------------------

#[derive(Debug)]
pub struct SetMediaBin {
    media: bettercut_foundation::MediaId,
    bin: Option<String>,
    previous: Option<Option<String>>,
}

impl SetMediaBin {
    pub fn new(media: bettercut_foundation::MediaId, bin: Option<String>) -> Self {
        Self {
            media,
            bin,
            previous: None,
        }
    }
}

impl EditorCommand for SetMediaBin {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;
        let was = std::mem::replace(&mut asset.bin, self.bin.clone());
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?
            .bin = previous;
        Ok(())
    }

    fn label(&self) -> String {
        "Move to Bin".to_owned()
    }
}

/// Change what a colour clip draws — and the name it goes by in the media
/// list, which says its colour.
#[derive(Debug)]
pub struct SetColour {
    media: bettercut_foundation::MediaId,
    colour: bettercut_media::Generated,
    previous: Option<(bettercut_media::Generated, String)>,
}

impl SetColour {
    pub fn new(media: bettercut_foundation::MediaId, colour: bettercut_media::Generated) -> Self {
        Self {
            media,
            colour,
            previous: None,
        }
    }
}

impl EditorCommand for SetColour {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;
        let was = asset.generated.ok_or(EditorError::NotAColourClip)?;
        let name = std::mem::replace(&mut asset.file_name, self.colour.name());
        asset.generated = Some(self.colour);
        if self.previous.is_none() {
            self.previous = Some((was, name));
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (colour, name) = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        let asset = project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;
        asset.generated = Some(colour);
        asset.file_name = name;
        Ok(())
    }

    fn label(&self) -> String {
        "Change Colour".to_owned()
    }
}

/// Give a file in the project a name of its own.
#[derive(Debug)]
pub struct RenameMedia {
    media: bettercut_foundation::MediaId,
    name: Option<String>,
    previous: Option<Option<String>>,
}

impl RenameMedia {
    pub fn new(media: bettercut_foundation::MediaId, name: Option<String>) -> Self {
        Self {
            media,
            name,
            previous: None,
        }
    }
}

impl EditorCommand for RenameMedia {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let asset = project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?;
        let was = std::mem::replace(&mut asset.label, self.name.clone());
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        project
            .media_asset_mut(self.media)
            .ok_or(EditorError::MediaNotFound(self.media))?
            .label = previous;
        Ok(())
    }

    fn label(&self) -> String {
        "Rename Media".to_owned()
    }
}

/// Give a track a new name.
#[derive(Debug)]
pub struct RenameTrack {
    pub sequence: SequenceId,
    pub track: TrackId,
    name: String,
    previous: Option<String>,
}

impl RenameTrack {
    pub fn new(sequence: SequenceId, track: TrackId, name: String) -> Self {
        Self {
            sequence,
            track,
            name,
            previous: None,
        }
    }
}

impl EditorCommand for RenameTrack {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let name = sequence_mut(project, self.sequence)?
            .track_name_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;
        let was = std::mem::replace(name, self.name.clone());
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.clone().ok_or(EditorError::NotExecuted)?;
        *sequence_mut(project, self.sequence)?
            .track_name_mut(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))? = previous;
        Ok(())
    }

    fn label(&self) -> String {
        "Rename Track".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Put a prepared track in at an index (see `Command::InsertTrack`).
#[derive(Debug)]
pub struct InsertTrack {
    pub sequence: SequenceId,
    index: usize,
    track: Option<TrackPayload>,
    id: TrackId,
}

impl InsertTrack {
    pub fn new(sequence: SequenceId, index: usize, track: TrackPayload) -> Self {
        let id = match &track {
            TrackPayload::Video(t) => t.id,
            TrackPayload::Audio(t) => t.id,
            TrackPayload::Text(t) => t.id,
            TrackPayload::Adjustment(t) => t.id,
        };
        Self {
            sequence,
            index,
            track: Some(track),
            id,
        }
    }
}

impl EditorCommand for InsertTrack {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let payload = self.track.take().ok_or(EditorError::NotExecuted)?;
        let sequence = sequence_mut(project, self.sequence)?;
        if sequence.track_kind(self.id).is_some() {
            // Put it back for a later attempt rather than losing it.
            self.track = Some(payload);
            return Err(EditorError::TrackAlreadyExists(self.id));
        }
        // Held to the end of the list: a replay onto a sequence that has fewer
        // tracks than it did still puts the track in.
        match payload {
            TrackPayload::Video(track) => {
                let index = self.index.min(sequence.video_tracks.len());
                sequence.video_tracks.insert(index, *track);
            }
            TrackPayload::Audio(track) => {
                let index = self.index.min(sequence.audio_tracks.len());
                sequence.audio_tracks.insert(index, *track);
            }
            TrackPayload::Text(track) => {
                let index = self.index.min(sequence.text_tracks.len());
                sequence.text_tracks.insert(index, *track);
            }
            TrackPayload::Adjustment(track) => {
                let index = self.index.min(sequence.adjustment_tracks.len());
                sequence.adjustment_tracks.insert(index, *track);
            }
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        let taken = if let Some(i) = sequence.video_tracks.iter().position(|t| t.id == self.id) {
            TrackPayload::Video(Box::new(sequence.video_tracks.remove(i)))
        } else if let Some(i) = sequence.audio_tracks.iter().position(|t| t.id == self.id) {
            TrackPayload::Audio(Box::new(sequence.audio_tracks.remove(i)))
        } else if let Some(i) = sequence.text_tracks.iter().position(|t| t.id == self.id) {
            TrackPayload::Text(Box::new(sequence.text_tracks.remove(i)))
        } else if let Some(i) = sequence
            .adjustment_tracks
            .iter()
            .position(|t| t.id == self.id)
        {
            TrackPayload::Adjustment(Box::new(sequence.adjustment_tracks.remove(i)))
        } else {
            return Err(EditorError::TrackNotFound(self.id));
        };
        // Kept, so a redo puts back exactly this track.
        self.track = Some(taken);
        Ok(())
    }

    fn label(&self) -> String {
        "Duplicate Track".to_owned()
    }
}

// --------------------------------------------------------------------------

#[derive(Debug)]
pub struct SetTrackFlag {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub flag: TrackFlag,
    pub value: bool,
    previous: Option<bool>,
}

impl SetTrackFlag {
    pub fn new(sequence: SequenceId, track: TrackId, flag: TrackFlag, value: bool) -> Self {
        Self {
            sequence,
            track,
            flag,
            value,
            previous: None,
        }
    }

    fn apply(&self, project: &mut Project, value: bool) -> Result<bool, EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;

        // Resolve the kind first, so each arm takes exactly one mutable borrow.
        let kind = sequence
            .track_kind(self.track)
            .ok_or(EditorError::TrackNotFound(self.track))?;

        let slot: &mut bool = match kind {
            TrackKind::Video => {
                let track = sequence
                    .video_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                match self.flag {
                    TrackFlag::Enabled => &mut track.enabled,
                    TrackFlag::Locked => &mut track.locked,
                    TrackFlag::Solo => &mut track.solo,
                    TrackFlag::SyncLock => &mut track.sync_lock,
                    TrackFlag::Targeted => &mut track.targeted,
                }
            }
            TrackKind::Audio => {
                let track = sequence
                    .audio_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                match self.flag {
                    TrackFlag::Enabled => &mut track.enabled,
                    TrackFlag::Locked => &mut track.locked,
                    TrackFlag::Solo => &mut track.solo,
                    TrackFlag::SyncLock => &mut track.sync_lock,
                    TrackFlag::Targeted => &mut track.targeted,
                }
            }
            TrackKind::Text => {
                let track = sequence
                    .text_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                match self.flag {
                    TrackFlag::Enabled => &mut track.enabled,
                    TrackFlag::Locked => &mut track.locked,
                    TrackFlag::Solo => &mut track.solo,
                    TrackFlag::SyncLock => &mut track.sync_lock,
                    TrackFlag::Targeted => &mut track.targeted,
                }
            }
            TrackKind::Adjustment => {
                let track = sequence
                    .adjustment_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                match self.flag {
                    TrackFlag::Enabled => &mut track.enabled,
                    TrackFlag::Locked => &mut track.locked,
                    TrackFlag::Solo => &mut track.solo,
                    TrackFlag::SyncLock => &mut track.sync_lock,
                    TrackFlag::Targeted => &mut track.targeted,
                }
            }
        };
        Ok(std::mem::replace(slot, value))
    }
}

impl EditorCommand for SetTrackFlag {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        self.previous = Some(self.apply(project, self.value)?);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous.take().ok_or(EditorError::NotExecuted)?;
        self.apply(project, previous)?;
        Ok(())
    }

    fn label(&self) -> String {
        match (self.flag, self.value) {
            (TrackFlag::Enabled, true) => "Show Track".to_owned(),
            (TrackFlag::Enabled, false) => "Hide Track".to_owned(),
            (TrackFlag::Locked, true) => "Lock Track".to_owned(),
            (TrackFlag::Locked, false) => "Unlock Track".to_owned(),
            (TrackFlag::Solo, true) => "Solo Track".to_owned(),
            (TrackFlag::Solo, false) => "Unsolo Track".to_owned(),
            (TrackFlag::SyncLock, true) => "Sync Lock Track".to_owned(),
            (TrackFlag::SyncLock, false) => "Unlock Sync".to_owned(),
            (TrackFlag::Targeted, true) => "Target Track".to_owned(),
            (TrackFlag::Targeted, false) => "Untarget Track".to_owned(),
        }
    }
}

// --------------------------------------------------------------------------

#[derive(Debug)]
pub struct AddClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub clip: ClipPayload,
    executed: bool,
}

impl AddClip {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipPayload) -> Self {
        Self {
            sequence,
            track,
            clip,
            executed: false,
        }
    }
}

impl EditorCommand for AddClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;
        match &self.clip {
            ClipPayload::Video(clip) => {
                let track = sequence
                    .video_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                track.insert((**clip).clone())?;
            }
            ClipPayload::Audio(clip) => {
                let track = sequence
                    .audio_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                track.insert((**clip).clone())?;
            }
            ClipPayload::Text(clip) => {
                let track = sequence
                    .text_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                track.insert((**clip).clone())?;
            }
            ClipPayload::Adjustment(clip) => {
                let track = sequence
                    .adjustment_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                track.insert((**clip).clone())?;
            }
        }
        self.executed = true;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if !self.executed {
            return Err(EditorError::NotExecuted);
        }
        let sequence = sequence_mut(project, self.sequence)?;
        let id = self.clip.id();
        match &self.clip {
            ClipPayload::Video(_) => {
                sequence
                    .video_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .remove(id)?;
            }
            ClipPayload::Audio(_) => {
                sequence
                    .audio_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .remove(id)?;
            }
            ClipPayload::Text(_) => {
                sequence
                    .text_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .remove(id)?;
            }
            ClipPayload::Adjustment(_) => {
                sequence
                    .adjustment_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .remove(id)?;
            }
        }
        self.executed = false;
        Ok(())
    }

    fn label(&self) -> String {
        "Add Clip".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Point a clip at other media (`crate::replace`).
#[derive(Debug)]
pub struct ReplaceClipMedia {
    sequence: SequenceId,
    track: TrackId,
    clip: ClipId,
    swap: crate::command::MediaSwap,
    previous: Option<crate::command::MediaSwap>,
}

impl ReplaceClipMedia {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        swap: crate::command::MediaSwap,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            swap,
            previous: None,
        }
    }

    fn apply(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip_id: ClipId,
        swap: crate::command::MediaSwap,
    ) -> Result<crate::command::MediaSwap, EditorError> {
        use crate::command::MediaSwap;
        // The same five fields on both kinds; the span is untouched, so the
        // track's order and spacing cannot change.
        macro_rules! swap_into {
            ($clip:expr) => {{
                let clip = $clip;
                let was = MediaSwap {
                    media: clip.media_id,
                    source: clip.source,
                    speed: clip.speed,
                    reversed: clip.reversed,
                    link: clip.link,
                };
                clip.media_id = swap.media;
                clip.source = swap.source;
                clip.speed = swap.speed;
                clip.reversed = swap.reversed;
                clip.link = swap.link;
                Ok(was)
            }};
        }
        with_track(
            project,
            sequence,
            track,
            |track| {
                swap_into!(
                    track
                        .get_mut(clip_id)
                        .ok_or(EditorError::ClipNotFound(clip_id))?
                )
            },
            |track| {
                swap_into!(
                    track
                        .get_mut(clip_id)
                        .ok_or(EditorError::ClipNotFound(clip_id))?
                )
            },
        )
    }
}

impl EditorCommand for ReplaceClipMedia {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let was = Self::apply(project, self.sequence, self.track, self.clip, self.swap)?;
        if self.previous.is_none() {
            self.previous = Some(was);
        }
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let was = self.previous.ok_or(EditorError::NotExecuted)?;
        Self::apply(project, self.sequence, self.track, self.clip, was)?;
        Ok(())
    }

    fn label(&self) -> String {
        "Replace Media".to_owned()
    }
}

#[derive(Debug)]
pub struct RemoveClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub clip: ClipId,
    /// The removed clip itself — undo re-inserts this exact value, so trims,
    /// transforms, and opacity survive the round trip.
    removed: Option<ClipPayload>,
}

impl RemoveClip {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId) -> Self {
        Self {
            sequence,
            track,
            clip,
            removed: None,
        }
    }
}

impl EditorCommand for RemoveClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let sequence = sequence_mut(project, self.sequence)?;

        if let Some(track) = sequence.video_track_mut(self.track) {
            let clip = track.remove(self.clip)?;
            self.removed = Some(ClipPayload::Video(Box::new(clip)));
            return Ok(());
        }
        if let Some(track) = sequence.audio_track_mut(self.track) {
            let clip = track.remove(self.clip)?;
            self.removed = Some(ClipPayload::Audio(Box::new(clip)));
            return Ok(());
        }
        Err(EditorError::TrackNotFound(self.track))
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let payload = self.removed.take().ok_or(EditorError::NotExecuted)?;
        let sequence = sequence_mut(project, self.sequence)?;
        match payload {
            ClipPayload::Video(clip) => {
                sequence
                    .video_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .insert(*clip)?;
            }
            ClipPayload::Audio(clip) => {
                sequence
                    .audio_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .insert(*clip)?;
            }
            ClipPayload::Text(clip) => {
                sequence
                    .text_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .insert(*clip)?;
            }
            ClipPayload::Adjustment(clip) => {
                sequence
                    .adjustment_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?
                    .insert(*clip)?;
            }
        }
        Ok(())
    }

    fn label(&self) -> String {
        "Delete Clip".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Move a clip, optionally to a different track (§10).
///
/// Cross-track moves are the same operation as within-track ones: remove, then
/// insert. Treating them separately would mean two code paths for one user
/// gesture — dragging a clip upward from V1 to V2.
#[derive(Debug)]
pub struct MoveClip {
    pub sequence: SequenceId,
    pub from_track: TrackId,
    pub to_track: TrackId,
    pub clip: ClipId,
    pub new_start: TimelineTime,
    /// Where it was, captured on execute so undo is exact even if the move was
    /// snapped or clamped.
    previous_start: Option<TimelineTime>,
}

impl MoveClip {
    pub fn new(
        sequence: SequenceId,
        from_track: TrackId,
        to_track: TrackId,
        clip: ClipId,
        new_start: TimelineTime,
    ) -> Self {
        Self {
            sequence,
            from_track,
            to_track,
            clip,
            new_start,
            previous_start: None,
        }
    }

    /// Move between two different tracks by removing and re-inserting.
    ///
    /// If the insert fails the clip goes back where it came from, so a rejected
    /// move never deletes the user's clip.
    fn relocate(
        project: &mut Project,
        sequence: SequenceId,
        from: TrackId,
        to: TrackId,
        clip: ClipId,
        new_start: TimelineTime,
    ) -> Result<TimelineTime, EditorError> {
        let payload = with_track(
            project,
            sequence,
            from,
            |track| Ok(ClipPayload::Video(Box::new(track.remove(clip)?))),
            |track| Ok(ClipPayload::Audio(Box::new(track.remove(clip)?))),
        )?;

        let previous_start = payload.start();
        let duration = match &payload {
            ClipPayload::Video(c) => c.timeline.duration(),
            ClipPayload::Audio(c) => c.timeline.duration(),
            ClipPayload::Text(c) => c.timeline.duration(),
            ClipPayload::Adjustment(c) => c.timeline.duration(),
        };
        let range = TimelineRange {
            start: new_start,
            end: new_start + duration,
        };

        let mut moved = payload.clone();
        match &mut moved {
            ClipPayload::Video(c) => c.timeline = range,
            ClipPayload::Audio(c) => c.timeline = range,
            ClipPayload::Text(c) => c.timeline = range,
            ClipPayload::Adjustment(c) => c.timeline = range,
        }

        let inserted = insert_payload(project, sequence, to, moved);
        if inserted.is_err() {
            // Put it back exactly as it was.
            let _ = insert_payload(project, sequence, from, payload);
        }
        inserted?;

        Ok(previous_start)
    }
}

/// Insert a clip payload into whichever kind of track it belongs to.
fn insert_payload(
    project: &mut Project,
    sequence: SequenceId,
    track: TrackId,
    payload: ClipPayload,
) -> Result<(), EditorError> {
    let kind = payload.kind();
    let target_kind = project
        .sequence(sequence)
        .and_then(|s| s.track_kind(track))
        .ok_or(EditorError::TrackNotFound(track))?;
    if kind != target_kind {
        return Err(EditorError::ClipKindMismatch);
    }

    match payload {
        ClipPayload::Video(clip) => with_track(
            project,
            sequence,
            track,
            move |t| {
                t.insert(*clip)?;
                Ok(())
            },
            |_| Err(EditorError::ClipKindMismatch),
        ),
        ClipPayload::Text(clip) => {
            let sequence = sequence_mut(project, sequence)?;
            let track = sequence
                .text_track_mut(track)
                .ok_or(EditorError::TrackNotFound(track))?;
            track.insert(*clip)?;
            Ok(())
        }
        ClipPayload::Adjustment(clip) => {
            let sequence = sequence_mut(project, sequence)?;
            let track = sequence
                .adjustment_track_mut(track)
                .ok_or(EditorError::TrackNotFound(track))?;
            track.insert(*clip)?;
            Ok(())
        }
        ClipPayload::Audio(clip) => with_track(
            project,
            sequence,
            track,
            |_| Err(EditorError::ClipKindMismatch),
            move |t| {
                t.insert(*clip)?;
                Ok(())
            },
        ),
    }
}

impl EditorCommand for MoveClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = if self.from_track == self.to_track {
            on_track!(project, self.sequence, self.from_track, |track| Ok(track
                .move_clip(
                self.clip,
                self.new_start
            )?))?
        } else {
            Self::relocate(
                project,
                self.sequence,
                self.from_track,
                self.to_track,
                self.clip,
                self.new_start,
            )?
        };

        self.previous_start = Some(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let previous = self.previous_start.take().ok_or(EditorError::NotExecuted)?;

        if self.from_track == self.to_track {
            on_track!(project, self.sequence, self.from_track, |track| Ok(track
                .move_clip(self.clip, previous)
                .map(|_| ())?))
        } else {
            Self::relocate(
                project,
                self.sequence,
                self.to_track,
                self.from_track,
                self.clip,
                previous,
            )
            .map(|_| ())
        }
    }

    fn label(&self) -> String {
        "Move Clip".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Which edge a trim is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimEdge {
    Start,
    End,
}

/// Trim one edge of a clip (§10 "Trim left", "Trim right").
#[derive(Debug)]
pub struct TrimClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub clip: ClipId,
    pub edge: TrimEdge,
    pub to: TimelineTime,
    /// Both ranges as they were. Storing the source range too is what makes
    /// undo restore the in/out points rather than just the visible extent.
    previous: Option<(TimelineRange, SourceRange)>,
}

impl TrimClip {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        edge: TrimEdge,
        to: TimelineTime,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            edge,
            to,
            previous: None,
        }
    }

    fn restore(
        &self,
        project: &mut Project,
        timeline: TimelineRange,
        source: SourceRange,
    ) -> Result<(), EditorError> {
        on_track!(project, self.sequence, self.track, |track| {
            let mut clip = track.remove(self.clip)?;
            clip.set_timeline(timeline);
            clip.set_source(source);
            track.insert(clip)?;
            Ok(())
        })
    }
}

impl EditorCommand for TrimClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let limit = media_limit(project, self.sequence, self.clip);
        let (edge, to) = (self.edge, self.to);

        let previous = on_track!(project, self.sequence, self.track, |track| match edge {
            TrimEdge::Start => Ok(track.trim_start(self.clip, to)?),
            TrimEdge::End => Ok(track.trim_end(self.clip, to, limit)?),
        })?;

        self.previous = Some(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (timeline, source) = self.previous.take().ok_or(EditorError::NotExecuted)?;
        self.restore(project, timeline, source)
    }

    fn label(&self) -> String {
        match self.edge {
            TrimEdge::Start => "Trim Clip Start".to_owned(),
            TrimEdge::End => "Trim Clip End".to_owned(),
        }
    }
}

// --------------------------------------------------------------------------

/// Split a clip at a frame-aligned instant (§76).
///
/// The caller must have snapped `at` to a frame boundary. `Editor::split_at`
/// does this; constructing the command directly does not.
#[derive(Debug)]
pub struct SplitClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub clip: ClipId,
    pub at: TimelineTime,
    /// Fixed at construction so a redo reproduces the same two clips.
    pub left: ClipId,
    pub right: ClipId,
    /// The clip as it was. Undo removes both halves and reinstates it —
    /// reconstructing it from the halves would silently lose anything the split
    /// did not carry across.
    original: Option<ClipPayload>,
    /// New links for the left and right halves; see `Command::SplitClip`.
    relink: Option<(bettercut_foundation::LinkId, bettercut_foundation::LinkId)>,
}

impl SplitClip {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        at: TimelineTime,
        left: ClipId,
        right: ClipId,
    ) -> Self {
        Self {
            sequence,
            track,
            clip,
            at,
            left,
            right,
            original: None,
            relink: None,
        }
    }

    pub fn relinked(
        mut self,
        relink: Option<(bettercut_foundation::LinkId, bettercut_foundation::LinkId)>,
    ) -> Self {
        self.relink = relink;
        self
    }
}

impl EditorCommand for SplitClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (at, left, right, relink) = (self.at, self.left, self.right, self.relink);
        let original = on_track!(project, self.sequence, self.track, |track| {
            let split = track.split(self.clip, at, left, right)?;
            // Undo reinstates `split.original`, which still carries the old
            // link, so only the halves need changing here.
            if let Some((left_link, right_link)) = relink {
                if let Some(half) = track.get_mut(left) {
                    half.set_link(Some(left_link));
                }
                if let Some(half) = track.get_mut(right) {
                    half.set_link(Some(right_link));
                }
            }
            Ok(ClipPayload::from(split.original))
        })?;

        // A grouped clip's halves are both in its group: cutting a clip is not
        // a reason for either half to stop moving with the rest.
        let (clip, left, right) = (self.clip, self.left, self.right);
        let sequence = sequence_mut(project, self.sequence)?;
        for group in &mut sequence.groups {
            if let Some(index) = group.iter().position(|member| *member == clip) {
                group.splice(index..=index, [left, right]);
            }
        }
        // And a note left on the clip is still there on both halves of it.
        if let Some(index) = sequence.notes.iter().position(|note| note.clip == clip) {
            let text = sequence.notes[index].text.clone();
            sequence.notes[index].clip = left;
            sequence
                .notes
                .push(bettercut_timeline::ClipNote { clip: right, text });
        }

        self.original = Some(original);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let original = self.original.take().ok_or(EditorError::NotExecuted)?;
        let (left, right) = (self.left, self.right);

        on_track!(project, self.sequence, self.track, |track| {
            track.remove(left)?;
            track.remove(right)?;
            Ok(())
        })?;

        // The halves give their place in a group back to the whole clip.
        let clip = self.clip;
        let sequence = sequence_mut(project, self.sequence)?;
        for group in &mut sequence.groups {
            if let Some(index) = group.iter().position(|member| *member == left) {
                group.retain(|member| *member != right);
                group[index] = clip;
            }
        }
        sequence.notes.retain(|note| note.clip != right);
        if let Some(note) = sequence.notes.iter_mut().find(|note| note.clip == left) {
            note.clip = clip;
        }

        insert_payload(project, self.sequence, self.track, original)
    }

    fn label(&self) -> String {
        "Split Clip".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Delete a clip and close the gap behind it (§10 "Ripple delete").
#[derive(Debug)]
pub struct RippleDeleteClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub clip: ClipId,
    removed: Option<(ClipPayload, TimelineTime)>,
}

impl RippleDeleteClip {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId) -> Self {
        Self {
            sequence,
            track,
            clip,
            removed: None,
        }
    }
}

impl EditorCommand for RippleDeleteClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let removed = on_track!(project, self.sequence, self.track, |track| {
            let (clip, shift) = track.ripple_remove(self.clip)?;
            Ok((ClipPayload::from(clip), shift))
        })?;

        self.removed = Some(removed);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (payload, shift) = self.removed.take().ok_or(EditorError::NotExecuted)?;

        match payload {
            ClipPayload::Video(clip) => with_track(
                project,
                self.sequence,
                self.track,
                move |track| Ok(track.ripple_restore(*clip, shift)?),
                |_| Err(EditorError::ClipKindMismatch),
            ),
            ClipPayload::Audio(clip) => with_track(
                project,
                self.sequence,
                self.track,
                |_| Err(EditorError::ClipKindMismatch),
                move |track| Ok(track.ripple_restore(*clip, shift)?),
            ),
            ClipPayload::Text(clip) => {
                let sequence = sequence_mut(project, self.sequence)?;
                let track = sequence
                    .text_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                Ok(track.ripple_restore(*clip, shift)?)
            }
            ClipPayload::Adjustment(clip) => {
                let sequence = sequence_mut(project, self.sequence)?;
                let track = sequence
                    .adjustment_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                Ok(track.ripple_restore(*clip, shift)?)
            }
        }
    }

    fn label(&self) -> String {
        "Ripple Delete".to_owned()
    }
}

// --------------------------------------------------------------------------

/// Place a copy of a clip at a given time (§10 "Duplicate", "Paste").
///
/// The payload is given a fresh `ClipId` on every execute, so pasting the same
/// clipboard entry twice produces two independent clips.
#[derive(Debug)]
pub struct PasteClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub payload: ClipPayload,
    pub at: TimelineTime,
    /// Fixed at construction, so a redo pastes the same clip rather than a
    /// second, differently-identified one.
    pub new_id: ClipId,
    executed: bool,
}

impl PasteClip {
    pub fn new(
        sequence: SequenceId,
        track: TrackId,
        payload: ClipPayload,
        at: TimelineTime,
        new_id: ClipId,
    ) -> Self {
        Self {
            sequence,
            track,
            payload,
            at,
            new_id,
            executed: false,
        }
    }
}

impl EditorCommand for PasteClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let mut payload = self.payload.clone();
        let id = self.new_id;
        let duration = match &payload {
            ClipPayload::Video(c) => c.timeline.duration(),
            ClipPayload::Audio(c) => c.timeline.duration(),
            ClipPayload::Text(c) => c.timeline.duration(),
            ClipPayload::Adjustment(c) => c.timeline.duration(),
        };
        let range = TimelineRange {
            start: self.at,
            end: self.at + duration,
        };

        match &mut payload {
            ClipPayload::Video(c) => {
                c.set_id(id);
                c.timeline = range;
                // §25: a transition describes a *cut*, and this clip is landing
                // somewhere else. Carried along, it would lie dormant until the
                // pasted clip happened to abut something and then dissolve into
                // a neighbour the user never paired it with.
                c.clear_transition_out();
            }
            ClipPayload::Audio(c) => {
                c.set_id(id);
                c.timeline = range;
            }
            ClipPayload::Text(c) => {
                c.set_id(id);
                c.timeline = range;
            }
            ClipPayload::Adjustment(c) => {
                c.set_id(id);
                c.timeline = range;
            }
        }

        insert_payload(project, self.sequence, self.track, payload)?;
        self.executed = true;
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        if !self.executed {
            return Err(EditorError::NotExecuted);
        }
        self.executed = false;
        let id = self.new_id;
        on_track!(project, self.sequence, self.track, |track| {
            track.remove(id)?;
            Ok(())
        })
    }

    fn label(&self) -> String {
        "Paste Clip".to_owned()
    }
}
