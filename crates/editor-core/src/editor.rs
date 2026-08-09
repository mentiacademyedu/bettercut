//! The editor facade — §55's command API, in process.
//!
//! ## The one rule
//!
//! `project()` returns `&Project`. There is no `project_mut()`. Rust would let
//! us expose one; the API does not, because §54 says the UI must never mutate
//! `Project` directly. Direct mutation bypasses undo, and automation (§32) and
//! templates (§77) are both defined as *command generators* — they stop working
//! the moment there is a second way to change state.

use std::path::{Path, PathBuf};

use bettercut_foundation::{ClipId, SequenceId, TimelineTime, TrackId};
use bettercut_media::{FfmpegProber, MediaAsset, MediaProber};
use bettercut_project_format::{PROJECT_EXTENSION, Project};
use bettercut_timeline::{Sequence, TrackKind};

use crate::command::{ClipPayload, Command, CommandGroup, EditorCommand, TrackFlag, TrimEdge};
use crate::error::EditorError;
use crate::event::{Event, EventReceiver, EventSender, event_channel};
use crate::hardware::HardwareProfile;
use crate::history::History;
use crate::journal::{Journal, RecoveryPaths};
use crate::ops;

/// A per-process id, so two instances editing unsaved projects do not write
/// into the same recovery directory and produce a nonsense replay.
fn session_id() -> String {
    format!("{}-{}", std::process::id(), SequenceId::new().short())
}

pub struct Editor {
    project: Project,
    path: Option<PathBuf>,
    history: History,

    /// Unsaved changes since the last save (§38 drives autosave from this).
    dirty: bool,

    /// Playhead position. Session state, not project state — moving the
    /// playhead is not an undoable edit (§10 lists it as an operation, but it
    /// changes nothing that gets saved).
    playhead: TimelineTime,

    /// Detected once at startup. Machine state, not project state: a project
    /// opened on a different computer must use *that* machine's limits (§44).
    hardware: HardwareProfile,

    /// Copied clips (§10). Session state — deliberately not saved, and
    /// deliberately not the OS clipboard, so copying a clip cannot clobber
    /// whatever the user had copied elsewhere.
    clipboard: Vec<ClipPayload>,

    /// Autosave journal (§38.2). Every successful command is appended here, so
    /// a crash costs at most the one command in flight.
    journal: Journal,

    events: EventSender,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("project", &self.project.name)
            .field("path", &self.path)
            .field("dirty", &self.dirty)
            .field("history", &self.history)
            .finish()
    }
}

impl Editor {
    /// `project.new` (§55). Returns the editor and the UI's event receiver.
    pub fn new_project(name: impl Into<String>) -> (Self, EventReceiver) {
        let mut project = Project::new(name);
        // §44: a low-end machine configures itself. The user can still change
        // the mode; this only picks the starting point.
        project.settings.performance_mode = HardwareProfile::detect().recommended_mode();
        Self::from_project(project)
    }

    /// Wrap an already-built `Project`.
    ///
    /// Used by tests to construct sequences at frame rates the default does not
    /// use, and by anything that generates a project wholesale — §77's template
    /// engine will need exactly this.
    pub fn from_project(project: Project) -> (Self, EventReceiver) {
        let (events, receiver) = event_channel();
        let journal = Journal::new(RecoveryPaths::for_project(None, &session_id()));

        let editor = Self {
            project,
            path: None,
            history: History::new(),
            dirty: false,
            playhead: TimelineTime::ZERO,
            hardware: HardwareProfile::detect(),
            clipboard: Vec::new(),
            journal,
            events,
        };
        editor.events.emit(Event::ProjectLoaded);
        (editor, receiver)
    }

    /// `project.open` (§55).
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, EventReceiver), EditorError> {
        let path = path.as_ref().to_path_buf();
        let mut project = bettercut_project_format::load(&path)?;

        // §66: tell the user immediately which files have moved, rather than
        // failing at the first decode.
        let missing = project.refresh_missing_media();
        if missing > 0 {
            tracing::warn!(missing, "project references media that is not present");
        }

        let (events, receiver) = event_channel();
        for asset in project.media.iter().filter(|m| m.missing) {
            events.emit(Event::MediaMissing(asset.id));
        }

        let journal = Journal::new(RecoveryPaths::for_project(Some(&path), &session_id()));

        let editor = Self {
            project,
            path: Some(path),
            history: History::new(),
            dirty: false,
            playhead: TimelineTime::ZERO,
            // The saved performance mode is kept — it is the user's choice —
            // but job and thread limits come from *this* machine (§44).
            hardware: HardwareProfile::detect(),
            clipboard: Vec::new(),
            journal,
            events,
        };
        editor.events.emit(Event::ProjectLoaded);
        Ok((editor, receiver))
    }

    // ---- reads (§54: borrowed, per frame, no allocation) ----

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn active_sequence(&self) -> Option<&Sequence> {
        self.project.active()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn playhead(&self) -> TimelineTime {
        self.playhead
    }

    /// This machine's detected limits (§44). Background jobs must read their
    /// concurrency and FFmpeg thread caps from here, never from a constant.
    pub fn hardware(&self) -> HardwareProfile {
        self.hardware
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo_label(&self) -> Option<String> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<String> {
        self.history.redo_label()
    }

    /// Window title: `"name — bettercut"`, with a marker when unsaved.
    pub fn window_title(&self) -> String {
        format!(
            "{}{} — bettercut",
            self.project.name,
            if self.dirty { " •" } else { "" }
        )
    }

    // ---- writes (§54: commands only) ----

    /// `playback.seek` (§55). Snapped to a frame boundary (§9, §76).
    pub fn set_playhead(&mut self, position: TimelineTime) {
        let clamped = position.max(TimelineTime::ZERO);
        let snapped = self
            .project
            .active()
            .map_or(clamped, |s| s.snap_to_frame(clamped));

        if snapped != self.playhead {
            self.playhead = snapped;
            // §56: throttled to once per UI frame by the caller, not per tick.
            self.events.emit(Event::PlaybackPositionChanged(snapped));
        }
    }

    /// Step one frame forward or back (§57's arrow keys).
    pub fn step_frames(&mut self, frames: i64) {
        let Some(sequence) = self.project.active() else {
            return;
        };
        let delta = TimelineTime::from_ticks(sequence.ticks_per_frame().saturating_mul(frames));
        self.set_playhead(self.playhead + delta);
    }

    /// Dispatch a request from the UI (§55). This is the whole write surface.
    pub fn dispatch(&mut self, command: Command) -> Result<(), EditorError> {
        self.ensure_journal_baseline();
        let boxed = self.build(command.clone())?;
        self.apply(boxed)?;
        // §38.2: journal only what actually took effect. A rejected edit
        // changed nothing, and replaying it would fail identically.
        self.journal_command(command);
        Ok(())
    }

    /// Execute several commands as one undo step (§79).
    pub fn dispatch_group(
        &mut self,
        label: impl Into<String>,
        commands: Vec<Command>,
    ) -> Result<(), EditorError> {
        self.ensure_journal_baseline();
        let mut group = CommandGroup::new(label);
        let journalled = commands.clone();
        for command in commands {
            group.push(self.build(command)?);
        }
        if group.is_empty() {
            return Ok(());
        }
        self.apply(Box::new(group))?;

        // A group is one undo step but several journal entries: replay rebuilds
        // state, and it needs each individual change (§38.2).
        for command in journalled {
            self.journal_command(command);
        }
        Ok(())
    }

    /// Run an already-built command. Used by automation and templates, which
    /// generate commands rather than mutating state (§32, §77).
    pub fn apply(&mut self, command: Box<dyn EditorCommand>) -> Result<(), EditorError> {
        self.history.execute(command, &mut self.project)?;
        self.mark_changed();
        Ok(())
    }

    pub fn undo(&mut self) -> Result<(), EditorError> {
        self.history.undo(&mut self.project)?;
        self.mark_changed();
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditorError> {
        self.history.redo(&mut self.project)?;
        self.mark_changed();
        Ok(())
    }

    /// `media.import` (§55) — probe a file and add it to the library (§12, §84).
    ///
    /// Probing is synchronous. It costs a few milliseconds per file because
    /// `avformat_find_stream_info` reads a little of each stream, which is well
    /// inside a frame budget for the handful of files a user picks at once.
    /// Bulk import belongs on the §15 job scheduler when that lands; §2's "the
    /// UI must never freeze" is about decoding and proxying, not about a single
    /// header read.
    pub fn import_file(
        &mut self,
        path: &Path,
    ) -> Result<bettercut_foundation::MediaId, EditorError> {
        let asset = FfmpegProber.probe(path)?;
        Ok(self.import_media(asset))
    }

    /// Add an already-probed asset. Not undoable: importing adds a library
    /// entry and touches no timeline state.
    pub fn import_media(&mut self, asset: MediaAsset) -> bettercut_foundation::MediaId {
        let id = self.project.add_media(asset);
        self.dirty = true;

        // Importing is not a `Command`, so the journal cannot replay it. The
        // only way a recovered session keeps the imported media is to fold it
        // into a fresh snapshot. Anything that changes project state outside
        // the command path has to do this, or recovery quietly loses it.
        if !self.journal.needs_baseline()
            && let Err(err) = self.journal.snapshot(&self.project)
        {
            tracing::error!(%err, "could not snapshot after importing media");
        }

        self.events.emit(Event::MediaImported(id));
        self.events.emit(Event::ProjectChanged);
        id
    }

    // ---- persistence (§55 project.save) ----

    pub fn save(&mut self) -> Result<(), EditorError> {
        let path = self.path.clone().ok_or(EditorError::NoProjectPath)?;
        self.save_as(path)
    }

    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), EditorError> {
        let mut path = path.as_ref().to_path_buf();
        if path.extension().is_none() {
            path.set_extension(PROJECT_EXTENSION);
        }

        bettercut_project_format::save(&self.project, &path)?;

        // §39: the user's file now holds everything, so there is nothing left
        // to recover. Leaving the journal behind would make the next launch
        // offer to restore work they already have — which reads as data loss
        // even though nothing was lost.
        self.journal.discard();
        self.journal = Journal::new(RecoveryPaths::for_project(Some(&path), &session_id()));

        self.path = Some(path);
        self.dirty = false;
        self.events.emit(Event::ProjectSaved);
        Ok(())
    }

    /// Discard recovery data on an orderly shutdown.
    pub fn shutdown(&mut self) {
        self.journal.discard();
    }

    // ---- internals ----

    fn mark_changed(&mut self) {
        self.dirty = true;
        self.events.emit(Event::ProjectChanged);
    }

    /// Write the baseline snapshot the journal replays onto, if there is none.
    ///
    /// **Must run before the command executes.** Snapshotting afterwards would
    /// capture a project that already contains the command, and replay would
    /// then apply it a second time.
    fn ensure_journal_baseline(&mut self) {
        if !self.journal.needs_baseline() {
            return;
        }
        if let Err(err) = self.journal.snapshot(&self.project) {
            tracing::error!(%err, "could not write the autosave baseline; recovery is unavailable");
        }
    }

    /// Append to the journal, snapshotting when one is due (§38.2).
    fn journal_command(&mut self, command: Command) {
        self.journal.append(&command);

        if self.journal.snapshot_is_due()
            && let Err(err) = self.journal.snapshot(&self.project)
        {
            // A failed snapshot is not a failed edit. The journal keeps
            // growing and recovery still works; it is only slower to replay.
            tracing::error!(%err, "autosave snapshot failed");
        }
    }

    /// Autosave state for the status bar.
    ///
    /// Surfaced because §38 makes autosave mandatory, and an autosave that has
    /// silently stopped working is worse than none — the user believes they are
    /// protected when they are not.
    pub fn autosave_failures(&self) -> u32 {
        self.journal.write_failures()
    }

    pub fn unsaved_commands(&self) -> u32 {
        self.journal.pending_commands()
    }

    pub fn recovery_paths(&self) -> &RecoveryPaths {
        self.journal.paths()
    }

    /// Force a snapshot now. Called when the app is closing.
    pub fn snapshot_now(&mut self) -> Result<(), EditorError> {
        self.journal.snapshot(&self.project)
    }

    /// Turn a request into an executable command, validating the parts that can
    /// be checked before any mutation happens.
    fn build(&self, command: Command) -> Result<Box<dyn EditorCommand>, EditorError> {
        match command {
            Command::RenameProject { name } => Ok(Box::new(ops::RenameProject::new(name))),

            Command::ChangeSetting { change } => Ok(Box::new(ops::ChangeSetting::new(change))),

            Command::AddTrack {
                sequence,
                kind,
                name,
                id,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::AddTrack::new(
                    sequence,
                    kind.into(),
                    name,
                    id,
                )))
            }

            Command::RemoveTrack { sequence, track } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::RemoveTrack::new(sequence, track)))
            }

            Command::SetTrackFlag {
                sequence,
                track,
                flag,
                value,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetTrackFlag::new(
                    sequence, track, flag, value,
                )))
            }

            Command::AddClip {
                sequence,
                track,
                clip,
            } => {
                let kind = self.require_track(sequence, track)?;
                // Catch a video clip aimed at an audio track before the insert
                // half-happens.
                if kind != clip.kind() {
                    return Err(EditorError::ClipKindMismatch);
                }
                Ok(Box::new(ops::AddClip::new(sequence, track, clip)))
            }

            Command::RemoveClip {
                sequence,
                track,
                clip,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::RemoveClip::new(sequence, track, clip)))
            }

            Command::MoveClip {
                sequence,
                from_track,
                to_track,
                clip,
                new_start,
            } => {
                let from = self.require_track(sequence, from_track)?;
                let to = self.require_track(sequence, to_track)?;
                // Dragging a video clip onto an audio track is a slip of the
                // mouse, not an instruction. Reject before anything moves.
                if from != to {
                    return Err(EditorError::ClipKindMismatch);
                }
                Ok(Box::new(ops::MoveClip::new(
                    sequence,
                    from_track,
                    to_track,
                    clip,
                    self.snap_to_frame(sequence, new_start),
                )))
            }

            Command::TrimClip {
                sequence,
                track,
                clip,
                edge,
                to,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::TrimClip::new(
                    sequence,
                    track,
                    clip,
                    edge,
                    self.snap_to_frame(sequence, to),
                )))
            }

            Command::SplitClip {
                sequence,
                track,
                clip,
                at,
                left,
                right,
            } => {
                self.require_track(sequence, track)?;
                // §76: "The split point must be snapped to a frame boundary in
                // TimelineTime ticks before the command is constructed."
                Ok(Box::new(ops::SplitClip::new(
                    sequence,
                    track,
                    clip,
                    self.snap_to_frame(sequence, at),
                    left,
                    right,
                )))
            }

            Command::RippleDeleteClip {
                sequence,
                track,
                clip,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::RippleDeleteClip::new(sequence, track, clip)))
            }

            Command::PasteClip {
                sequence,
                track,
                clip,
                at,
                new_id,
            } => {
                let kind = self.require_track(sequence, track)?;
                if kind != clip.kind() {
                    return Err(EditorError::ClipKindMismatch);
                }
                Ok(Box::new(ops::PasteClip::new(
                    sequence,
                    track,
                    clip,
                    self.snap_to_frame(sequence, at),
                    new_id,
                )))
            }
        }
    }

    /// Round a position to the sequence's frame grid.
    ///
    /// Applied to every edit that takes a time, so no command can introduce a
    /// sub-frame boundary (§9, §76). A cut that is half a frame off cannot be
    /// rendered and shows up as a duplicated or dropped frame at export.
    fn snap_to_frame(&self, sequence: SequenceId, at: TimelineTime) -> TimelineTime {
        self.project
            .sequence(sequence)
            .map_or(at, |s| s.snap_to_frame(at.max(TimelineTime::ZERO)))
    }

    fn require_sequence(&self, id: SequenceId) -> Result<&Sequence, EditorError> {
        self.project
            .sequence(id)
            .ok_or(EditorError::SequenceNotFound(id))
    }

    fn require_track(
        &self,
        sequence: SequenceId,
        track: TrackId,
    ) -> Result<TrackKind, EditorError> {
        self.require_sequence(sequence)?
            .track_kind(track)
            .ok_or(EditorError::TrackNotFound(track))
    }
}

/// Convenience constructors for the commands the UI sends most often, so call
/// sites read as intent rather than as struct literals.
impl Editor {
    pub fn add_video_track(&mut self, name: impl Into<String>) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::AddTrack {
            sequence,
            kind: crate::command::TrackKindRepr::Video,
            name: name.into(),
            id: TrackId::new(),
        })
    }

    pub fn add_audio_track(&mut self, name: impl Into<String>) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::AddTrack {
            sequence,
            kind: crate::command::TrackKindRepr::Audio,
            name: name.into(),
            id: TrackId::new(),
        })
    }

    pub fn add_clip(&mut self, track: TrackId, clip: ClipPayload) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::AddClip {
            sequence,
            track,
            clip,
        })
    }

    pub fn remove_clip(&mut self, track: TrackId, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::RemoveClip {
            sequence,
            track,
            clip,
        })
    }

    pub fn set_track_flag(
        &mut self,
        track: TrackId,
        flag: TrackFlag,
        value: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetTrackFlag {
            sequence,
            track,
            flag,
            value,
        })
    }

    fn active_sequence_id(&self) -> Result<SequenceId, EditorError> {
        self.project
            .active()
            .map(|s| s.id)
            .ok_or_else(|| EditorError::SequenceNotFound(SequenceId::new()))
    }
}

/// Milestone 3 editing (§10, §85).
impl Editor {
    pub fn move_clip(
        &mut self,
        from_track: TrackId,
        to_track: TrackId,
        clip: ClipId,
        new_start: TimelineTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::MoveClip {
            sequence,
            from_track,
            to_track,
            clip,
            new_start,
        })
    }

    pub fn trim_clip(
        &mut self,
        track: TrackId,
        clip: ClipId,
        edge: TrimEdge,
        to: TimelineTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::TrimClip {
            sequence,
            track,
            clip,
            edge,
            to,
        })
    }

    /// Split every selected clip at the playhead, or — when nothing is
    /// selected — whatever clip the playhead is currently over (§76).
    ///
    /// Returns how many clips were cut. Splitting the clip under the playhead
    /// with no selection is what makes `S` a one-key operation, which is the
    /// single most-used edit in a rough cut.
    pub fn split_at_playhead(&mut self, selected: &[ClipId]) -> Result<usize, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let at = self.playhead;

        let Some(sequence) = self.project.sequence(sequence_id) else {
            return Ok(0);
        };

        // Find every clip that the playhead falls strictly inside.
        let mut cuts: Vec<(TrackId, ClipId)> = Vec::new();
        let mut consider =
            |track: TrackId, clip: ClipId, range: bettercut_timeline::TimelineRange| {
                // Strictly inside: splitting on an edge would make a zero-length clip.
                if at > range.start
                    && at < range.end
                    && (selected.is_empty() || selected.contains(&clip))
                {
                    cuts.push((track, clip));
                }
            };

        for track in &sequence.video_tracks {
            for clip in track.clips() {
                consider(track.id, clip.id, clip.timeline);
            }
        }
        for track in &sequence.audio_tracks {
            for clip in track.clips() {
                consider(track.id, clip.id, clip.timeline);
            }
        }

        if cuts.is_empty() {
            return Ok(0);
        }

        let count = cuts.len();
        let commands = cuts
            .into_iter()
            .map(|(track, clip)| Command::SplitClip {
                sequence: sequence_id,
                track,
                clip,
                at,
                left: ClipId::new(),
                right: ClipId::new(),
            })
            .collect();

        // One undo step, however many tracks were cut (§79).
        let label = if count == 1 {
            "Split Clip".to_owned()
        } else {
            format!("Split {count} Clips")
        };
        self.dispatch_group(label, commands)?;
        Ok(count)
    }

    pub fn ripple_delete(&mut self, track: TrackId, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::RippleDeleteClip {
            sequence,
            track,
            clip,
        })
    }

    /// Which track holds a clip. Linear over tracks, not over clips.
    pub fn track_of(&self, clip: ClipId) -> Option<TrackId> {
        let sequence = self.project.active()?;
        sequence
            .video_tracks
            .iter()
            .find(|t| t.get(clip).is_some())
            .map(|t| t.id)
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find(|t| t.get(clip).is_some())
                    .map(|t| t.id)
            })
    }

    /// Look a clip up as a payload, for copy and duplicate.
    pub fn clip_payload(&self, clip: ClipId) -> Option<ClipPayload> {
        let sequence = self.project.active()?;
        for track in &sequence.video_tracks {
            if let Some(c) = track.get(clip) {
                return Some(ClipPayload::Video(Box::new(c.clone())));
            }
        }
        for track in &sequence.audio_tracks {
            if let Some(c) = track.get(clip) {
                return Some(ClipPayload::Audio(Box::new(c.clone())));
            }
        }
        None
    }

    // ---- clipboard (§10 copy/paste) ----

    /// Copy clips to the internal clipboard.
    ///
    /// Stored relative to the earliest clip, so a multi-clip paste keeps the
    /// spacing between them instead of stacking everything at the playhead.
    pub fn copy_clips(&mut self, clips: &[ClipId]) -> usize {
        let payloads: Vec<ClipPayload> =
            clips.iter().filter_map(|c| self.clip_payload(*c)).collect();
        let count = payloads.len();
        self.clipboard = payloads;
        count
    }

    pub fn clipboard_len(&self) -> usize {
        self.clipboard.len()
    }

    /// Paste the clipboard at the playhead, preserving relative offsets.
    ///
    /// Each clip goes back to a track of its own kind — a copied audio clip
    /// must not land on a video track.
    pub fn paste_at_playhead(&mut self) -> Result<usize, EditorError> {
        if self.clipboard.is_empty() {
            return Err(EditorError::Timeline(
                bettercut_timeline::TimelineError::ClipboardEmpty,
            ));
        }

        let sequence_id = self.active_sequence_id()?;
        let at = self.playhead;

        let origin = self
            .clipboard
            .iter()
            .map(ClipPayload::start)
            .min()
            .unwrap_or(TimelineTime::ZERO);

        let (video_track, audio_track) = {
            let sequence = self
                .project
                .sequence(sequence_id)
                .ok_or(EditorError::SequenceNotFound(sequence_id))?;
            (
                sequence.video_tracks.first().map(|t| t.id),
                sequence.audio_tracks.first().map(|t| t.id),
            )
        };

        let mut commands = Vec::new();
        for payload in &self.clipboard {
            let offset = payload.start() - origin;
            let track = match payload.kind() {
                bettercut_timeline::TrackKind::Video => video_track,
                bettercut_timeline::TrackKind::Audio => audio_track,
            };
            let Some(track) = track else { continue };

            commands.push(Command::PasteClip {
                sequence: sequence_id,
                track,
                clip: payload.clone(),
                at: at + offset,
                new_id: ClipId::new(),
            });
        }

        if commands.is_empty() {
            return Err(EditorError::TrackNotFound(TrackId::new()));
        }

        let count = commands.len();
        let label = if count == 1 {
            "Paste Clip".to_owned()
        } else {
            format!("Paste {count} Clips")
        };
        self.dispatch_group(label, commands)?;
        Ok(count)
    }

    /// Duplicate a clip, placing the copy immediately after the original
    /// (§10 "Duplicate clip").
    pub fn duplicate_clip(&mut self, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let payload = self
            .clip_payload(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        let at = match &payload {
            ClipPayload::Video(c) => c.timeline.end,
            ClipPayload::Audio(c) => c.timeline.end,
        };

        self.dispatch(Command::PasteClip {
            sequence,
            track,
            clip: payload,
            at,
            new_id: ClipId::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{FrameRate, MediaId, MediaTime};
    use bettercut_media::MediaKind;
    use bettercut_timeline::{SourceRange, VideoClip};

    fn clip_at(media: MediaId, start_ticks: i64, len_ticks: i64) -> ClipPayload {
        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(len_ticks)).expect("valid");
        let clip =
            VideoClip::new(media, TimelineTime::from_ticks(start_ticks), source).expect("valid");
        ClipPayload::Video(Box::new(clip))
    }

    fn editor_with_media() -> (Editor, EventReceiver, TrackId, MediaId) {
        let (mut editor, rx) = Editor::new_project("Test");
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/a.mp4",
            MediaTime::from_seconds(10),
        ));
        let track = editor.active_sequence().expect("sequence").video_tracks[0].id;
        (editor, rx, track, media)
    }

    #[test]
    fn a_new_project_is_not_dirty_and_has_no_history() {
        let (editor, _rx) = Editor::new_project("Test");
        assert!(!editor.is_dirty());
        assert!(!editor.can_undo());
        assert!(!editor.can_redo());
        assert_eq!(editor.playhead(), TimelineTime::ZERO);
    }

    #[test]
    fn dispatching_a_command_makes_the_project_dirty_and_undoable() {
        let (mut editor, _rx) = Editor::new_project("Test");
        editor
            .dispatch(Command::RenameProject {
                name: "Renamed".to_owned(),
            })
            .expect("ok");

        assert_eq!(editor.project().name, "Renamed");
        assert!(editor.is_dirty());
        assert!(editor.can_undo());

        editor.undo().expect("ok");
        assert_eq!(editor.project().name, "Test");
    }

    #[test]
    fn add_and_remove_clip_round_trip_through_undo() {
        let (mut editor, _rx, track, media) = editor_with_media();

        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");
        assert_eq!(editor.project().clip_count(), 1);

        let clip_id = editor.active_sequence().expect("seq").video_tracks[0].clips()[0].id;
        editor.remove_clip(track, clip_id).expect("ok");
        assert_eq!(editor.project().clip_count(), 0);

        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 1);
        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 0);

        editor.redo().expect("ok");
        editor.redo().expect("ok");
        assert_eq!(editor.project().clip_count(), 0);
    }

    /// A rejected edit must leave no trace — not in the project, not in history.
    #[test]
    fn a_failed_command_is_not_recorded_in_history() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");
        let depth_before = editor.can_undo();

        // Overlaps the clip that is already there.
        let err = editor.add_clip(track, clip_at(media, 1_000, 96_000));
        assert!(err.is_err(), "overlapping insert should be rejected");

        assert_eq!(editor.project().clip_count(), 1);
        assert!(depth_before);
        // Undoing once returns to empty: the failed command left no entry.
        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 0);
        assert!(!editor.can_undo());
    }

    #[test]
    fn a_video_clip_cannot_be_added_to_an_audio_track() {
        let (mut editor, _rx, _track, media) = editor_with_media();
        let audio_track = editor.active_sequence().expect("seq").audio_tracks[0].id;

        assert!(matches!(
            editor.add_clip(audio_track, clip_at(media, 0, 96_000)),
            Err(EditorError::ClipKindMismatch)
        ));
    }

    #[test]
    fn removing_a_track_restores_its_clips_on_undo() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");

        let sequence = editor.active_sequence().expect("seq").id;
        editor
            .dispatch(Command::RemoveTrack { sequence, track })
            .expect("ok");
        assert_eq!(editor.project().clip_count(), 0);
        assert!(
            editor
                .active_sequence()
                .expect("seq")
                .video_tracks
                .is_empty()
        );

        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 1, "clips were not restored");
    }

    /// §79 — a group is one undo step, and a failed group applies nothing.
    #[test]
    fn a_command_group_is_a_single_undo_step() {
        let (mut editor, _rx, track, media) = editor_with_media();
        let sequence = editor.active_sequence().expect("seq").id;

        editor
            .dispatch_group(
                "Add three clips",
                vec![
                    Command::AddClip {
                        sequence,
                        track,
                        clip: clip_at(media, 0, 32_000),
                    },
                    Command::AddClip {
                        sequence,
                        track,
                        clip: clip_at(media, 32_000, 32_000),
                    },
                    Command::AddClip {
                        sequence,
                        track,
                        clip: clip_at(media, 64_000, 32_000),
                    },
                ],
            )
            .expect("ok");

        assert_eq!(editor.project().clip_count(), 3);
        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 0, "group undid partially");
    }

    #[test]
    fn a_group_that_fails_partway_applies_nothing() {
        let (mut editor, _rx, track, media) = editor_with_media();
        let sequence = editor.active_sequence().expect("seq").id;

        let result = editor.dispatch_group(
            "Two overlapping clips",
            vec![
                Command::AddClip {
                    sequence,
                    track,
                    clip: clip_at(media, 0, 32_000),
                },
                // Overlaps the previous one.
                Command::AddClip {
                    sequence,
                    track,
                    clip: clip_at(media, 16_000, 32_000),
                },
            ],
        );

        assert!(result.is_err());
        assert_eq!(
            editor.project().clip_count(),
            0,
            "the first clip of a failed group was left behind"
        );
        assert!(!editor.can_undo());
    }

    /// §9/§76: the playhead always sits on a frame boundary.
    #[test]
    fn the_playhead_snaps_to_a_frame_boundary() {
        let (mut editor, _rx) = Editor::new_project("Test");
        // Default sequence is 30 fps: 32,000 ticks per frame.
        editor.set_playhead(TimelineTime::from_ticks(40_000));
        assert_eq!(editor.playhead().ticks(), 32_000);

        editor.step_frames(1);
        assert_eq!(editor.playhead().ticks(), 64_000);
        editor.step_frames(-2);
        assert_eq!(editor.playhead().ticks(), 0);
    }

    #[test]
    fn the_playhead_never_goes_negative() {
        let (mut editor, _rx) = Editor::new_project("Test");
        editor.step_frames(-10);
        assert_eq!(editor.playhead(), TimelineTime::ZERO);
    }

    #[test]
    fn playhead_snapping_follows_an_ntsc_sequence() {
        let (mut editor, _rx) = Editor::new_project("Test");
        // Change the sequence rate through the project, then verify snapping.
        // (Sequence settings are not yet a command; that arrives with §85.)
        editor.set_playhead(TimelineTime::ZERO);
        let sequence = editor.active_sequence().expect("seq");
        assert_eq!(sequence.frame_rate, FrameRate::FPS_30);
    }

    #[test]
    fn save_and_reopen_preserves_the_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("round.vproj");

        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");
        editor.save_as(&path).expect("save");
        assert!(!editor.is_dirty());

        let (reopened, _rx2) = Editor::open(&path).expect("open");
        assert_eq!(reopened.project().clip_count(), 1);
        assert_eq!(reopened.project().media.len(), 1);
        assert!(!reopened.is_dirty());
        assert!(!reopened.can_undo(), "history must not survive a reload");
    }

    #[test]
    fn save_without_a_path_is_refused() {
        let (mut editor, _rx) = Editor::new_project("Test");
        assert!(matches!(editor.save(), Err(EditorError::NoProjectPath)));
    }

    #[test]
    fn save_as_appends_the_project_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut editor, _rx) = Editor::new_project("Test");
        editor.save_as(dir.path().join("noext")).expect("save");
        assert_eq!(
            editor.path().and_then(Path::extension),
            Some(std::ffi::OsStr::new("vproj"))
        );
    }

    #[test]
    fn the_window_title_marks_unsaved_changes() {
        let (mut editor, _rx) = Editor::new_project("My Film");
        assert_eq!(editor.window_title(), "My Film — bettercut");
        editor
            .dispatch(Command::RenameProject {
                name: "My Film".to_owned(),
            })
            .expect("ok");
        assert!(editor.window_title().contains('•'));
    }

    #[test]
    fn edits_emit_project_changed_events() {
        let (mut editor, rx) = Editor::new_project("Test");
        let _ = rx.drain(); // discard ProjectLoaded

        editor
            .dispatch(Command::RenameProject {
                name: "x".to_owned(),
            })
            .expect("ok");
        assert!(rx.drain().contains(&Event::ProjectChanged));
    }

    // ---- Milestone 3 editing (§10, §85) ----

    /// Builds a sequence at an arbitrary frame rate, so NTSC snapping can be
    /// exercised through the real command path.
    fn editor_at_rate(rate: FrameRate) -> (Editor, TrackId, MediaId) {
        use bettercut_project_format::Project;
        use bettercut_timeline::{AudioTrack, Resolution, Sequence, VideoTrack};

        let mut sequence = Sequence::new("Seq", Resolution::HD_1080, rate).expect("supported rate");
        sequence.video_tracks.push(VideoTrack::new("V1"));
        sequence.video_tracks.push(VideoTrack::new("V2"));
        sequence.audio_tracks.push(AudioTrack::new("A1"));

        let mut project = Project::new("Rate");
        project.active_sequence = Some(sequence.id);
        project.sequences = vec![sequence];

        let (mut editor, _rx) = Editor::from_project(project);
        let media = editor.import_media(
            MediaAsset::new(
                MediaKind::Video,
                "C:/media/a.mp4",
                MediaTime::from_seconds(60),
            )
            .with_video(1920, 1080, rate),
        );
        let track = editor.active_sequence().expect("seq").video_tracks[0].id;
        (editor, track, media)
    }

    #[test]
    fn moving_a_clip_is_undoable() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");
        let clip = editor.active_sequence().expect("seq").video_tracks[0].clips()[0].id;

        editor
            .move_clip(track, track, clip, TimelineTime::from_ticks(320_000))
            .expect("ok");
        assert_eq!(
            editor.active_sequence().expect("seq").video_tracks[0].clips()[0]
                .timeline
                .start
                .ticks(),
            320_000
        );

        editor.undo().expect("ok");
        assert_eq!(
            editor.active_sequence().expect("seq").video_tracks[0].clips()[0]
                .timeline
                .start
                .ticks(),
            0
        );
    }

    #[test]
    fn a_clip_can_be_moved_to_another_track_and_back() {
        let (mut editor, track, media) = editor_at_rate(FrameRate::FPS_30);
        let v2 = editor.active_sequence().expect("seq").video_tracks[1].id;

        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(96_000)).expect("valid");
        let clip = VideoClip::new(media, TimelineTime::ZERO, source).expect("valid");
        let clip_id = clip.id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .expect("ok");

        editor
            .move_clip(track, v2, clip_id, TimelineTime::from_ticks(64_000))
            .expect("ok");

        let sequence = editor.active_sequence().expect("seq");
        assert!(sequence.video_tracks[0].clips().is_empty());
        assert_eq!(sequence.video_tracks[1].clips().len(), 1);

        editor.undo().expect("ok");
        let sequence = editor.active_sequence().expect("seq");
        assert_eq!(sequence.video_tracks[0].clips().len(), 1);
        assert!(sequence.video_tracks[1].clips().is_empty());
        assert_eq!(
            sequence.video_tracks[0].clips()[0].timeline.start.ticks(),
            0
        );
    }

    /// A video clip dragged onto an audio track is a slip, not an instruction.
    #[test]
    fn a_clip_cannot_be_moved_to_a_track_of_the_wrong_kind() {
        let (mut editor, track, media) = editor_at_rate(FrameRate::FPS_30);
        let audio = editor.active_sequence().expect("seq").audio_tracks[0].id;

        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(96_000)).expect("valid");
        let clip = VideoClip::new(media, TimelineTime::ZERO, source).expect("valid");
        let clip_id = clip.id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .expect("ok");

        assert!(matches!(
            editor.move_clip(track, audio, clip_id, TimelineTime::ZERO),
            Err(EditorError::ClipKindMismatch)
        ));
        // Nothing was lost in the attempt.
        assert_eq!(editor.project().clip_count(), 1);
    }

    #[test]
    fn trimming_is_undoable_and_restores_the_source_range() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");
        let clip = editor.active_sequence().expect("seq").video_tracks[0].clips()[0].id;

        editor
            .trim_clip(track, clip, TrimEdge::End, TimelineTime::from_ticks(64_000))
            .expect("ok");

        let trimmed = &editor.active_sequence().expect("seq").video_tracks[0].clips()[0];
        assert_eq!(trimmed.timeline.end.ticks(), 64_000);
        assert_eq!(trimmed.source.end.ticks(), 64_000);

        editor.undo().expect("ok");
        let restored = &editor.active_sequence().expect("seq").video_tracks[0].clips()[0];
        assert_eq!(restored.timeline.end.ticks(), 96_000);
        assert_eq!(
            restored.source.end.ticks(),
            96_000,
            "undo restored the extent but not the source out-point"
        );
    }

    /// The media is 10 s long; a clip cannot be stretched past it.
    #[test]
    fn trimming_past_the_end_of_the_media_is_refused() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");
        let clip = editor.active_sequence().expect("seq").video_tracks[0].clips()[0].id;

        let past_the_end = TimelineTime::from_seconds(30);
        assert!(
            editor
                .trim_clip(track, clip, TrimEdge::End, past_the_end)
                .is_err()
        );
        assert_eq!(
            editor.active_sequence().expect("seq").video_tracks[0].clips()[0]
                .timeline
                .end
                .ticks(),
            96_000
        );
    }

    /// §76 + §9: the cut must land on a frame boundary even when the requested
    /// instant does not. At 29.97 that boundary is every 32,032 ticks.
    #[test]
    fn splitting_snaps_to_an_ntsc_frame_boundary() {
        let (mut editor, track, media) = editor_at_rate(FrameRate::NTSC_29_97);

        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(32_032 * 10)).expect("valid");
        let clip = VideoClip::new(media, TimelineTime::ZERO, source).expect("valid");
        let clip_id = clip.id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .expect("ok");

        let sequence = editor.active_sequence().expect("seq").id;
        // Deliberately between frames 5 and 6.
        let ragged = TimelineTime::from_ticks(32_032 * 5 + 17_000);
        editor
            .dispatch(Command::SplitClip {
                sequence,
                track,
                clip: clip_id,
                at: ragged,
                left: ClipId::new(),
                right: ClipId::new(),
            })
            .expect("ok");

        let clips = editor.active_sequence().expect("seq").video_tracks[0].clips();
        assert_eq!(clips.len(), 2);
        let cut = clips[0].timeline.end.ticks();
        assert_eq!(cut, 32_032 * 5, "cut did not snap to a frame boundary");
        assert_eq!(cut % 32_032, 0);
        assert_eq!(clips[1].timeline.start.ticks(), cut, "gap at the cut");
    }

    #[test]
    fn splitting_at_the_playhead_cuts_the_clip_underneath() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");

        editor.set_playhead(TimelineTime::from_ticks(32_000));
        let cuts = editor.split_at_playhead(&[]).expect("ok");
        assert_eq!(cuts, 1);
        assert_eq!(editor.project().clip_count(), 2);

        // One undo puts it back, even though two clips were produced.
        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 1);
    }

    #[test]
    fn splitting_off_a_clip_does_nothing() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");

        // Past the end of the only clip.
        editor.set_playhead(TimelineTime::from_ticks(320_000));
        assert_eq!(editor.split_at_playhead(&[]).expect("ok"), 0);
        assert_eq!(editor.project().clip_count(), 1);

        // The top of the undo stack is still the Add, not a no-op Split —
        // otherwise pressing S over empty space would cost the user an undo.
        assert_eq!(editor.undo_label().as_deref(), Some("Add Clip"));
    }

    /// Splitting exactly on a clip edge would create a zero-length clip.
    #[test]
    fn splitting_on_a_clip_edge_does_nothing() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");

        editor.set_playhead(TimelineTime::ZERO);
        assert_eq!(editor.split_at_playhead(&[]).expect("ok"), 0);
        editor.set_playhead(TimelineTime::from_ticks(96_000));
        assert_eq!(editor.split_at_playhead(&[]).expect("ok"), 0);
        assert_eq!(editor.project().clip_count(), 1);
    }

    /// Undo then redo must reproduce the *same* clips, not equivalent ones.
    ///
    /// Every ID a command creates is part of the request (§38.2's journal
    /// replays commands, and redo re-executes them). Minting IDs at execute
    /// time makes a redone split silently different from the original, which
    /// breaks selection and any later command naming those clips.
    #[test]
    fn redoing_a_split_reproduces_the_same_clip_ids() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 96_000))
            .expect("ok");

        editor.set_playhead(TimelineTime::from_ticks(32_000));
        editor.split_at_playhead(&[]).expect("ok");

        let after_split: Vec<_> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();

        editor.undo().expect("ok");
        editor.redo().expect("ok");

        let after_redo: Vec<_> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();

        assert_eq!(
            after_split, after_redo,
            "redo produced different clip identities than the original split"
        );
    }

    #[test]
    fn redoing_a_paste_reproduces_the_same_clip_id() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 32_000))
            .expect("ok");
        let clip = editor.active_sequence().expect("seq").video_tracks[0].clips()[0].id;

        editor.duplicate_clip(clip).expect("ok");
        let after: Vec<_> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();

        editor.undo().expect("ok");
        editor.redo().expect("ok");

        let after_redo: Vec<_> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            after, after_redo,
            "redo pasted a differently-identified clip"
        );
    }

    #[test]
    fn redoing_an_add_track_reproduces_the_same_track_id() {
        let (mut editor, _rx) = Editor::new_project("Test");
        editor.add_video_track("V2").expect("ok");

        let added = editor.active_sequence().expect("seq").video_tracks[1].id;
        editor.undo().expect("ok");
        editor.redo().expect("ok");

        assert_eq!(
            editor.active_sequence().expect("seq").video_tracks[1].id,
            added,
            "redo created a different track"
        );
    }

    #[test]
    fn ripple_delete_closes_the_gap_and_undo_reopens_it() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 32_000))
            .expect("ok");
        editor
            .add_clip(track, clip_at(media, 32_000, 32_000))
            .expect("ok");
        editor
            .add_clip(track, clip_at(media, 64_000, 32_000))
            .expect("ok");

        let middle = editor.active_sequence().expect("seq").video_tracks[0].clips()[1].id;
        editor.ripple_delete(track, middle).expect("ok");

        let starts: Vec<i64> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline.start.ticks())
            .collect();
        assert_eq!(starts, vec![0, 32_000]);

        editor.undo().expect("ok");
        let starts: Vec<i64> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline.start.ticks())
            .collect();
        assert_eq!(starts, vec![0, 32_000, 64_000]);
    }

    #[test]
    fn copy_and_paste_preserves_relative_spacing() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 32_000))
            .expect("ok");
        editor
            .add_clip(track, clip_at(media, 64_000, 32_000))
            .expect("ok");

        let ids: Vec<_> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(editor.copy_clips(&ids), 2);

        editor.set_playhead(TimelineTime::from_ticks(320_000));
        assert_eq!(editor.paste_at_playhead().expect("ok"), 2);

        let starts: Vec<i64> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline.start.ticks())
            .collect();
        // The 64,000-tick gap between the originals is preserved.
        assert_eq!(starts, vec![0, 64_000, 320_000, 384_000]);

        // Pasted clips are new entities, not aliases of the originals.
        let all: Vec<_> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.id)
            .collect();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "paste reused a clip id");

        // One undo removes the whole paste (§79).
        editor.undo().expect("ok");
        assert_eq!(editor.project().clip_count(), 2);
    }

    #[test]
    fn pasting_an_empty_clipboard_is_an_error() {
        let (mut editor, _rx) = Editor::new_project("Test");
        assert!(editor.paste_at_playhead().is_err());
    }

    #[test]
    fn duplicate_places_the_copy_immediately_after() {
        let (mut editor, _rx, track, media) = editor_with_media();
        editor
            .add_clip(track, clip_at(media, 0, 32_000))
            .expect("ok");
        let clip = editor.active_sequence().expect("seq").video_tracks[0].clips()[0].id;

        editor.duplicate_clip(clip).expect("ok");

        let starts: Vec<i64> = editor.active_sequence().expect("seq").video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline.start.ticks())
            .collect();
        assert_eq!(starts, vec![0, 32_000]);
    }

    /// Every edit routed through `dispatch` lands on the frame grid, whatever
    /// the caller asked for (§9, §76).
    #[test]
    fn every_timed_edit_snaps_to_the_frame_grid() {
        let (mut editor, track, media) = editor_at_rate(FrameRate::NTSC_29_97);

        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(32_032 * 20)).expect("valid");
        let clip = VideoClip::new(media, TimelineTime::ZERO, source).expect("valid");
        let clip_id = clip.id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .expect("ok");

        // A move to a ragged position.
        editor
            .move_clip(track, track, clip_id, TimelineTime::from_ticks(500_000))
            .expect("ok");
        let start = editor.active_sequence().expect("seq").video_tracks[0].clips()[0]
            .timeline
            .start
            .ticks();
        assert_eq!(start % 32_032, 0, "move left the clip off the frame grid");

        // A trim to a ragged position.
        let end_target = TimelineTime::from_ticks(start + 32_032 * 3 + 999);
        editor
            .trim_clip(track, clip_id, TrimEdge::End, end_target)
            .expect("ok");
        let end = editor.active_sequence().expect("seq").video_tracks[0].clips()[0]
            .timeline
            .end
            .ticks();
        assert_eq!(end % 32_032, 0, "trim left the clip off the frame grid");
    }

    #[test]
    fn commands_referencing_a_missing_track_are_rejected_before_mutating() {
        let (mut editor, _rx) = Editor::new_project("Test");
        let sequence = editor.active_sequence().expect("seq").id;
        assert!(matches!(
            editor.dispatch(Command::RemoveTrack {
                sequence,
                track: TrackId::new(),
            }),
            Err(EditorError::TrackNotFound(_))
        ));
        assert!(!editor.is_dirty());
    }
}
