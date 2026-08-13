//! Concrete reversible operations (§10).
//!
//! Each holds exactly what its undo needs — the removed clip, the previous
//! name — and nothing else. §11: "Do not snapshot the entire project after
//! every edit."

use bettercut_foundation::{
    ClipId, FrameRate, MediaId, MediaTime, SequenceId, TimelineTime, TrackId, ticks_per_frame,
};
use bettercut_project_format::Project;
use bettercut_timeline::{
    AnimatedParameter, AudioTrack, Clip, Keyframe, Resolution, Sequence, SourceRange,
    TimelineRange, TrackKind, Vec2, VideoTrack,
};

use crate::command::{
    ClipPayload, ClipProperty, EditorCommand, SettingChange, TrackFlag, TrackPayload,
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
        Command::SetClipProperty {
            sequence,
            track,
            clip,
            property,
        } => Box::new(SetClipProperty::new(sequence, track, clip, property)),
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

        Command::RemoveClip {
            sequence,
            track,
            clip,
        } => Box::new(RemoveClip::new(sequence, track, clip)),

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

        Command::SplitClip {
            sequence,
            track,
            clip,
            at,
            left,
            right,
        } => Box::new(SplitClip::new(sequence, track, clip, at, left, right)),

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
        None => Err(EditorError::TrackNotFound(track)),
    }
}

/// The source out-point a clip may not be trimmed past: the media's duration.
///
/// `None` when the media is unknown, in which case only the structural
/// invariants apply — a missing asset must not make trimming impossible (§66).
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
    project.media_asset(media_id).map(|m| m.duration)
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
                    ClipProperty::Blur(value) => {
                        let was = clip.blur;
                        clip.blur = AnimatedParameter::Blur.clamp(value);
                        ClipProperty::Blur(was)
                    }
                    // Gain is an audio property; a video clip has none.
                    ClipProperty::Gain(_) => return Err(EditorError::ClipKindMismatch),
                })
            },
            |audio| {
                let clip = audio.get_mut(clip).ok_or(EditorError::ClipNotFound(clip))?;
                match property {
                    ClipProperty::Gain(value) => {
                        let was = clip.gain;
                        clip.gain = value.clamp(0.0, 4.0);
                        Ok(ClipProperty::Gain(was))
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
            // §59 lists clip gain as keyframeable, but Milestone 8 animates the
            // video parameters only. Refusing here beats silently accepting a
            // key that nothing would ever read.
            |_audio| Err(EditorError::ClipKindMismatch),
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
        }
        self.executed = false;
        Ok(())
    }

    fn label(&self) -> String {
        match self.kind {
            TrackKind::Video => "Add Video Track".to_owned(),
            TrackKind::Audio => "Add Audio Track".to_owned(),
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
        }
        Ok(())
    }

    fn label(&self) -> String {
        "Remove Track".to_owned()
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
                }
            }
            TrackKind::Audio => {
                let track = sequence
                    .audio_track_mut(self.track)
                    .ok_or(EditorError::TrackNotFound(self.track))?;
                match self.flag {
                    TrackFlag::Enabled => &mut track.enabled,
                    TrackFlag::Locked => &mut track.locked,
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
        }
        self.executed = false;
        Ok(())
    }

    fn label(&self) -> String {
        "Add Clip".to_owned()
    }
}

// --------------------------------------------------------------------------

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
        };
        let range = TimelineRange {
            start: new_start,
            end: new_start + duration,
        };

        let mut moved = payload.clone();
        match &mut moved {
            ClipPayload::Video(c) => c.timeline = range,
            ClipPayload::Audio(c) => c.timeline = range,
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
            with_track(
                project,
                self.sequence,
                self.from_track,
                |track| Ok(track.move_clip(self.clip, self.new_start)?),
                |track| Ok(track.move_clip(self.clip, self.new_start)?),
            )?
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
            with_track(
                project,
                self.sequence,
                self.from_track,
                |track| Ok(track.move_clip(self.clip, previous).map(|_| ())?),
                |track| Ok(track.move_clip(self.clip, previous).map(|_| ())?),
            )
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
        with_track(
            project,
            self.sequence,
            self.track,
            |track| {
                let clip = track.remove(self.clip)?;
                let mut clip = clip;
                clip.timeline = timeline;
                clip.source = source;
                track.insert(clip)?;
                Ok(())
            },
            |track| {
                let clip = track.remove(self.clip)?;
                let mut clip = clip;
                clip.timeline = timeline;
                clip.source = source;
                track.insert(clip)?;
                Ok(())
            },
        )
    }
}

impl EditorCommand for TrimClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let limit = media_limit(project, self.sequence, self.clip);
        let (edge, to) = (self.edge, self.to);

        let previous = with_track(
            project,
            self.sequence,
            self.track,
            |track| match edge {
                TrimEdge::Start => Ok(track.trim_start(self.clip, to)?),
                TrimEdge::End => Ok(track.trim_end(self.clip, to, limit)?),
            },
            |track| match edge {
                TrimEdge::Start => Ok(track.trim_start(self.clip, to)?),
                TrimEdge::End => Ok(track.trim_end(self.clip, to, limit)?),
            },
        )?;

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
        }
    }
}

impl EditorCommand for SplitClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (at, left, right) = (self.at, self.left, self.right);
        let original = with_track(
            project,
            self.sequence,
            self.track,
            |track| {
                let split = track.split(self.clip, at, left, right)?;
                Ok(ClipPayload::Video(Box::new(split.original)))
            },
            |track| {
                let split = track.split(self.clip, at, left, right)?;
                Ok(ClipPayload::Audio(Box::new(split.original)))
            },
        )?;

        self.original = Some(original);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let original = self.original.take().ok_or(EditorError::NotExecuted)?;
        let (left, right) = (self.left, self.right);

        with_track(
            project,
            self.sequence,
            self.track,
            |track| {
                track.remove(left)?;
                track.remove(right)?;
                Ok(())
            },
            |track| {
                track.remove(left)?;
                track.remove(right)?;
                Ok(())
            },
        )?;

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
        let removed = with_track(
            project,
            self.sequence,
            self.track,
            |track| {
                let (clip, shift) = track.ripple_remove(self.clip)?;
                Ok((ClipPayload::Video(Box::new(clip)), shift))
            },
            |track| {
                let (clip, shift) = track.ripple_remove(self.clip)?;
                Ok((ClipPayload::Audio(Box::new(clip)), shift))
            },
        )?;

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
        };
        let range = TimelineRange {
            start: self.at,
            end: self.at + duration,
        };

        match &mut payload {
            ClipPayload::Video(c) => {
                c.set_id(id);
                c.timeline = range;
            }
            ClipPayload::Audio(c) => {
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
        with_track(
            project,
            self.sequence,
            self.track,
            |track| {
                track.remove(id)?;
                Ok(())
            },
            |track| {
                track.remove(id)?;
                Ok(())
            },
        )
    }

    fn label(&self) -> String {
        "Paste Clip".to_owned()
    }
}
