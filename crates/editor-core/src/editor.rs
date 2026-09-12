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

use bettercut_foundation::{ClipId, FrameRate, MediaTime, SequenceId, TimelineTime, TrackId};
use bettercut_media::{FfmpegProber, MediaAsset, MediaProber};
use bettercut_project_format::{PROJECT_EXTENSION, Project};
use bettercut_timeline::{
    DEFAULT_TRANSITION, Interpolation, Keyframe, Movement, Resolution, Sequence, TrackKind,
    Transition, TransitionKind,
};

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

/// Commands applied so far inside [`Editor::staged`].
#[derive(Default)]
pub(crate) struct Stage {
    /// As requested, for the journal.
    commands: Vec<Command>,
    /// As executed, for the undo step and for rolling back.
    built: Vec<Box<dyn EditorCommand>>,
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

    /// How many steps Undo can take.
    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
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
        self.journal_commands(journalled);
        Ok(())
    }

    /// Run `body`, which applies commands one at a time through [`Self::stage`],
    /// and record everything it applied as one undo step (§79).
    ///
    /// For edits where a later command depends on what an earlier one did — a
    /// template adds a track and then places clips on it, and whether a
    /// transition fits depends on the clips placed before it. `dispatch_group`
    /// builds every command against the state from *before* the group, so it
    /// cannot express either.
    ///
    /// All or nothing: if `body` fails, everything it staged is undone and the
    /// project is exactly as it was.
    pub(crate) fn staged<T>(
        &mut self,
        label: impl Into<String>,
        body: impl FnOnce(&mut Self, &mut Stage) -> Result<T, EditorError>,
    ) -> Result<T, EditorError> {
        self.ensure_journal_baseline();
        let mut stage = Stage::default();
        match body(self, &mut stage) {
            Ok(value) => {
                self.commit_stage(label.into(), stage)?;
                Ok(value)
            }
            Err(err) => {
                self.roll_back(&mut stage.built);
                Err(err)
            }
        }
    }

    /// Apply one command inside [`Self::staged`], built against the current
    /// state — including whatever was staged before it.
    pub(crate) fn stage(&mut self, stage: &mut Stage, command: Command) -> Result<(), EditorError> {
        let mut built = self.build(command.clone())?;
        built.execute(&mut self.project)?;
        stage.commands.push(command);
        stage.built.push(built);
        Ok(())
    }

    fn commit_stage(&mut self, label: String, stage: Stage) -> Result<(), EditorError> {
        let Stage {
            commands,
            mut built,
        } = stage;
        if built.is_empty() {
            return Ok(());
        }
        // Undone and run again as one group, because `History` records what it
        // executes. Running a command again after undoing it is exactly what
        // redo does, so every command already supports it.
        self.roll_back(&mut built);
        let mut group = CommandGroup::new(label);
        for command in built {
            group.push(command);
        }
        self.apply(Box::new(group))?;
        self.journal_commands(commands);
        Ok(())
    }

    fn roll_back(&mut self, built: &mut [Box<dyn EditorCommand>]) {
        for command in built.iter_mut().rev() {
            if let Err(err) = command.undo(&mut self.project) {
                tracing::error!(?err, "could not roll back a staged edit");
            }
        }
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

    /// Match an empty sequence to the first video imported into it (§8).
    ///
    /// A new project is 1080p30 because it has to be *something*, but that is a
    /// guess. Importing 25 fps footage into a 30 fps sequence makes every frame
    /// land between grid points, which shows as judder — and the user has no
    /// reason to suspect a "sequence format" is the cause.
    ///
    /// Only for an empty sequence, and only from a video with a usable rate:
    /// once clips exist, the format is a decision the user has effectively made
    /// and changing it under them would move where their cuts fall.
    ///
    /// Returns the command that was applied, if any, so the caller can say what
    /// happened. Adopting silently would be its own surprise.
    pub fn adopt_format_from(
        &mut self,
        media: bettercut_foundation::MediaId,
    ) -> Option<(Resolution, FrameRate)> {
        let sequence = self.project.active()?;
        if sequence.clip_count() > 0 {
            return None;
        }
        let sequence_id = sequence.id;

        let asset = self.project.media_asset(media)?;
        if asset.kind != bettercut_media::MediaKind::Video {
            return None;
        }
        let rate = asset.frame_rate?;
        let resolution = Resolution::new(asset.width, asset.height);
        if resolution.width == 0 || resolution.height == 0 {
            return None;
        }

        // A rate the timebase cannot represent exactly is left alone rather
        // than forced: §9 would rather keep an honest 30 fps grid than adopt a
        // rate whose frame boundaries drift.
        if bettercut_foundation::ticks_per_frame(rate).is_none() {
            tracing::info!(%rate, "not adopting a frame rate the timebase cannot represent");
            return None;
        }

        let current = (sequence.resolution, sequence.frame_rate);
        if current == (resolution, rate) {
            return None;
        }

        self.dispatch(Command::SetSequenceFormat {
            sequence: sequence_id,
            resolution: resolution.into(),
            frame_rate: rate,
        })
        .ok()?;

        Some((resolution, rate))
    }

    /// Set a clip property (§59), collapsing a drag into one undo step.
    ///
    /// `continuing` is true while a slider is being dragged and false on the
    /// first change of a gesture. The caller knows which, because it owns the
    /// widget; the editor cannot tell a drag from a series of clicks.
    pub fn set_clip_property(
        &mut self,
        clip: ClipId,
        property: crate::command::ClipProperty,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let command = Command::SetClipProperty {
            sequence,
            track,
            clip,
            property,
        };

        self.dispatch_gesture(
            format!("Change {}", property.kind()),
            vec![command],
            continuing,
        )
    }

    /// Execute commands as one undo entry, continuing the entry on top of the
    /// stack while `continuing` (§11).
    ///
    /// The label is the identity of the gesture: `execute_coalescing` matches
    /// on it, and a group of one carries it just as a bare command does, so a
    /// drag that starts as a static slider edit and a drag that writes two
    /// keyframes coalesce by exactly the same rule.
    fn dispatch_gesture(
        &mut self,
        label: String,
        commands: Vec<Command>,
        continuing: bool,
    ) -> Result<(), EditorError> {
        if commands.is_empty() {
            return Ok(());
        }
        if !continuing {
            return self.dispatch_group(label, commands);
        }

        // Same shape as `dispatch_group`, but coalescing. Journalled either
        // way: a crash mid-drag should recover the value the user was looking
        // at. Deliberately `journal.append` rather than `journal_command` —
        // this runs once per frame of a drag, and a snapshot check per frame is
        // work the user would feel.
        self.ensure_journal_baseline();
        let mut group = CommandGroup::new(label.clone());
        for command in &commands {
            group.push(self.build(command.clone())?);
        }

        self.history
            .execute_coalescing(Box::new(group), &mut self.project, |top| {
                top.label() == label
            })?;

        self.dirty = true;
        for command in &commands {
            self.journal.append(command);
        }
        self.events.emit(Event::ProjectChanged);
        Ok(())
    }

    /// Apply an inspector control (§59), as keyframes where the parameter is
    /// animated and as a static value where it is not (§24).
    ///
    /// This is the whole reason animating does not need a second set of
    /// controls: the slider does not know or care which it is writing. One
    /// control per parameter, one place that decides.
    pub fn set_clip_value(
        &mut self,
        clip: ClipId,
        property: crate::command::ClipProperty,
        continuing: bool,
    ) -> Result<(), EditorError> {
        // §26: a title has the same transform controls and a different command
        // behind them. Routed here rather than at every call site, because the
        // preview's drag handles produce a `ClipProperty` and have no business
        // knowing what kind of layer they are moving.
        if self.is_text_clip(clip) {
            return match crate::command::TextProperty::from_clip_property(property) {
                Some(text) => self.set_text_property(clip, text, continuing),
                None => Err(EditorError::ClipKindMismatch),
            };
        }

        let Some(video) = self.video_clip(clip) else {
            return self.set_clip_property(clip, property, continuing);
        };
        let animated = property
            .animated()
            .into_iter()
            .flatten()
            .any(|(parameter, _)| video.keyframes.is_animated(parameter));

        let Some(at) = self.source_time_at_playhead(clip) else {
            // An animated parameter has no static value to change — writing one
            // would move a number the renderer never reads, and the control
            // would appear not to work. Refused, with a message saying where to
            // put the playhead.
            if animated {
                return Err(EditorError::PlayheadOffClip);
            }
            return self.set_clip_property(clip, property, continuing);
        };
        // Re-borrowed: `source_time_at_playhead` took the editor immutably.
        let Some(video) = self.video_clip(clip) else {
            return self.set_clip_property(clip, property, continuing);
        };

        let keyed: Vec<_> = property
            .animated()
            .into_iter()
            .flatten()
            .filter(|(parameter, _)| video.keyframes.is_animated(*parameter))
            .map(|(parameter, value)| {
                // Keep the curve the key already had; a drag changes the value,
                // not the easing the user chose for it.
                let interpolation = video
                    .keyframes
                    .get(parameter, at)
                    .map_or_else(Interpolation::default, |key| key.interpolation);
                (parameter, Keyframe::new(at, value, interpolation))
            })
            .collect();

        if keyed.is_empty() {
            return self.set_clip_property(clip, property, continuing);
        }

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let commands = keyed
            .into_iter()
            .map(|(parameter, key)| Command::SetKeyframe {
                sequence,
                track,
                clip,
                parameter,
                key,
            })
            .collect();

        self.dispatch_gesture(
            format!("Keyframe {}", property.kind()),
            commands,
            continuing,
        )
    }

    /// Remove every keyframe on a clip, leaving its values alone (§24).
    ///
    /// Deliberately *not* the same as resetting the clip. Once the keys are
    /// gone each control goes back to reading its own static value, which is
    /// what "stop animating this" means — the alternative, baking whatever was
    /// on screen at the playhead into every control, would silently make one
    /// frame of a fade permanent across the whole clip.
    ///
    /// One undo step however many keys there were (§79).
    pub fn clear_clip_keyframes(&mut self, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        let commands: Vec<Command> = video
            .keyframes
            .iter()
            .flat_map(|animation| {
                let parameter = animation.parameter;
                animation
                    .keys()
                    .iter()
                    .map(move |key| Command::RemoveKeyframe {
                        sequence,
                        track,
                        clip,
                        parameter,
                        time: key.time,
                    })
            })
            .collect();

        if commands.is_empty() {
            return Ok(());
        }
        self.dispatch_group("Remove Keyframes", commands)
    }

    /// Put a clip's look back to default — including its animation — as one
    /// undo step (§79).
    ///
    /// Clearing the keys is the part that matters: a keyed parameter ignores
    /// its static value, so resetting the numbers alone would leave a clip that
    /// still fades while every control claims it does not.
    pub fn reset_clip_look(&mut self, clip: ClipId) -> Result<(), EditorError> {
        use crate::command::ClipProperty;

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        let mut commands: Vec<Command> = video
            .keyframes
            .iter()
            .flat_map(|animation| {
                let parameter = animation.parameter;
                animation
                    .keys()
                    .iter()
                    .map(move |key| Command::RemoveKeyframe {
                        sequence,
                        track,
                        clip,
                        parameter,
                        time: key.time,
                    })
            })
            .collect();

        commands.extend(
            [
                ClipProperty::Opacity(0.0),
                ClipProperty::Scale { x: 0.0, y: 0.0 },
                ClipProperty::Position { x: 0.0, y: 0.0 },
                ClipProperty::Rotation(0.0),
                ClipProperty::Brightness(0.0),
                ClipProperty::Contrast(0.0),
                ClipProperty::Saturation(0.0),
                ClipProperty::Blur(0.0),
            ]
            .into_iter()
            .map(Self::reset_values)
            .map(|property| Command::SetClipProperty {
                sequence,
                track,
                clip,
                property,
            }),
        );

        self.dispatch_group("Reset Clip", commands)
    }

    /// Rewrite a property with every value replaced by its default.
    ///
    /// Takes the property so the *shape* is the caller's — which control, and
    /// therefore which parameters — while the values come from the model. That
    /// keeps "what does reset mean" in one place while leaving "reset what" at
    /// the call site.
    fn reset_values(property: crate::command::ClipProperty) -> crate::command::ClipProperty {
        use crate::command::ClipProperty as P;
        use bettercut_timeline::AnimatedParameter as A;

        match property {
            P::Opacity(_) => P::Opacity(A::Opacity.default_value()),
            P::Gain(_) => P::Gain(1.0),
            P::Position { .. } => P::Position {
                x: A::PositionX.default_value(),
                y: A::PositionY.default_value(),
            },
            P::Scale { .. } => P::Scale {
                x: A::ScaleX.default_value(),
                y: A::ScaleY.default_value(),
            },
            P::Rotation(_) => P::Rotation(A::Rotation.default_value()),
            P::Brightness(_) => P::Brightness(A::Brightness.default_value()),
            P::Contrast(_) => P::Contrast(A::Contrast.default_value()),
            P::Saturation(_) => P::Saturation(A::Saturation.default_value()),
            P::Blur(_) => P::Blur(A::Blur.default_value()),
        }
    }

    /// Put one inspector control back to default, keyframes and all.
    ///
    /// Same meaning as the whole-clip reset, applied to one row: as if that
    /// parameter had never been touched. Clearing its keys is part of it —
    /// resetting the number while leaving an animation driving it would look
    /// like the button had done nothing.
    pub fn reset_clip_parameter(
        &mut self,
        clip: ClipId,
        property: crate::command::ClipProperty,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        // Only this control's own parameters — `Position` owns X and Y, and
        // resetting it must not disturb scale or anything else.
        let parameters: Vec<_> = property
            .animated()
            .into_iter()
            .flatten()
            .map(|(parameter, _)| parameter)
            .collect();

        let mut commands: Vec<Command> = parameters
            .iter()
            .filter_map(|parameter| video.keyframes.track(*parameter))
            .flat_map(|animation| {
                let parameter = animation.parameter;
                animation
                    .keys()
                    .iter()
                    .map(move |key| Command::RemoveKeyframe {
                        sequence,
                        track,
                        clip,
                        parameter,
                        time: key.time,
                    })
            })
            .collect();

        commands.push(Command::SetClipProperty {
            sequence,
            track,
            clip,
            property: Self::reset_values(property),
        });

        self.dispatch_group(format!("Reset {}", property.kind()), commands)
    }

    /// Put an imported asset on the timeline, picture and sound together (§12).
    ///
    /// A video file is *two* clips — a `VideoClip` on a video track and an
    /// `AudioClip` on an audio track — because §8 keeps picture and sound on
    /// separate tracks. Placing only the picture is how an editor ends up
    /// exporting silent video, and placing an audio file as a video clip is how
    /// a track ends up holding something that cannot be drawn.
    ///
    /// Both start at the same instant, which is the point of taking the later
    /// of the two tracks' ends rather than each track's own: started
    /// independently on tracks of different lengths, a video's sound would land
    /// somewhere other than its picture.
    ///
    /// One undo step for the pair (§79), because "add this file" is one thing
    /// the user did.
    pub fn place_media(
        &mut self,
        media: bettercut_foundation::MediaId,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let asset = self
            .project
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;

        let wants_video = asset.kind.has_video();
        let wants_audio = asset.audio_codec.is_some();
        // A photo has no length of its own; it gets `STILL_DURATION`.
        let duration = asset.placement_duration();

        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let video_track = sequence.video_tracks.first().map(|t| (t.id, t.duration()));
        let audio_track = sequence.audio_tracks.first().map(|t| (t.id, t.duration()));

        if (!wants_video || video_track.is_none()) && (!wants_audio || audio_track.is_none()) {
            return Err(EditorError::NoTrackForMedia);
        }

        // The later of the two, so the pair stays together.
        let start = [
            wants_video
                .then(|| video_track.map(|(_, end)| end))
                .flatten(),
            wants_audio
                .then(|| audio_track.map(|(_, end)| end))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .fold(TimelineTime::ZERO, TimelineTime::max);

        let source = bettercut_timeline::SourceRange::new(MediaTime::ZERO, duration)?;
        let mut commands = Vec::new();
        let mut placed = Vec::new();

        // Only when both halves are going down: a link to nothing is a lie
        // that later edits would have to keep checking.
        let link = (wants_video && wants_audio && video_track.is_some() && audio_track.is_some())
            .then(bettercut_foundation::LinkId::new);

        if wants_video && let Some((track, _)) = video_track {
            let mut clip = bettercut_timeline::VideoClip::new(media, start, source)?;
            clip.link = link;
            placed.push(clip.id);
            commands.push(Command::AddClip {
                sequence: sequence_id,
                track,
                clip: ClipPayload::Video(Box::new(clip)),
            });
        }

        if wants_audio && let Some((track, _)) = audio_track {
            let mut clip = bettercut_timeline::AudioClip::new(media, start, source)?;
            clip.link = link;
            placed.push(clip.id);
            commands.push(Command::AddClip {
                sequence: sequence_id,
                track,
                clip: ClipPayload::Audio(Box::new(clip)),
            });
        }

        self.dispatch_group("Add Clip".to_owned(), commands)?;
        Ok(placed)
    }

    // ---- text overlays (§26) ----

    /// Add a title at the playhead.
    ///
    /// Placed on the first text track, at the first instant from the playhead
    /// onwards where it fits. Not simply *at* the playhead, because that is
    /// often over an existing title and refusing would be a dead end — the user
    /// asked for a title, not for a lesson in track occupancy.
    pub fn add_text(&mut self, text: impl Into<String>) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_tracks.first().map(|t| t.id))
            .ok_or(EditorError::NoTextTrack)?;

        let start = self.free_text_slot(track, self.playhead);
        let clip = bettercut_timeline::TextClip::new(text, start)?;
        let id = clip.id;

        self.dispatch(Command::AddText {
            sequence: sequence_id,
            track,
            clip: Box::new(clip),
        })?;
        Ok(id)
    }

    /// The first position at or after `from` where a title of the default
    /// length fits without overlapping.
    fn free_text_slot(&self, track: TrackId, from: TimelineTime) -> TimelineTime {
        let Some(sequence) = self.active_sequence() else {
            return from;
        };
        let Some(track) = sequence.text_track(track) else {
            return from;
        };

        let mut start = from;
        // Walk forward past whatever is in the way. The list is sorted, so this
        // steps over each obstruction once rather than rescanning.
        loop {
            let end = start + bettercut_timeline::DEFAULT_TEXT_DURATION;
            let blocking = track
                .clips()
                .iter()
                .find(|c| c.timeline.start < end && c.timeline.end > start);
            match blocking {
                Some(clip) => start = clip.timeline.end,
                None => return start,
            }
        }
    }

    pub fn remove_text(&mut self, clip: ClipId) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_track_of(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;
        self.dispatch(Command::RemoveText {
            sequence: sequence_id,
            track,
            clip,
        })
    }

    /// Change one thing about a title.
    ///
    /// `continuing` collapses a drag into a single undo step, exactly as
    /// [`Self::set_clip_property`] does — the two behave the same way because
    /// they are the same gesture on different kinds of clip (§11).
    pub fn set_text_property(
        &mut self,
        clip: ClipId,
        property: crate::command::TextProperty,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_track_of(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;

        let label = format!("Change {}", property.kind());
        let command = Command::SetTextProperty {
            sequence,
            track,
            clip,
            property,
        };
        self.dispatch_gesture(label, vec![command], continuing)
    }

    pub fn text_clip(&self, clip: ClipId) -> Option<&bettercut_timeline::TextClip> {
        self.project.active()?.text_clip(clip)
    }

    /// Whether this id names a text overlay rather than a video or audio clip.
    ///
    /// The interface asks before deciding which inspector to show: a selection
    /// is a bare id, and the two kinds of clip take different controls.
    pub fn is_text_clip(&self, clip: ClipId) -> bool {
        self.text_clip(clip).is_some()
    }

    // ---- captions (§27, Milestone 10) ----

    /// What a caption lane is called.
    ///
    /// Captions go on a lane of their own rather than onto whatever text track
    /// happens to exist: a subtitle file is dozens of clips end to end, and
    /// dropping them among someone's titles would either collide with them or
    /// scatter them into the gaps.
    pub const CAPTION_TRACK: &'static str = "Captions";

    /// Read a subtitle file onto the timeline (Milestone 10).
    ///
    /// Returns how many captions landed. Everything about the file — the
    /// format, the tolerated deviations, the sorting and de-overlapping — is
    /// [`bettercut_captions`]'s business; what happens here is turning
    /// segments into clips and making the whole import one undo step, because
    /// "undo the import" is the only thing anyone means after a bad file.
    pub fn import_captions(&mut self, path: &std::path::Path) -> Result<usize, EditorError> {
        let parsed = bettercut_captions::read(path)?;
        if parsed.skipped > 0 {
            // §50: never silently ignore. A third of someone's subtitles
            // missing with no word said is worse than a warning.
            tracing::warn!(
                skipped = parsed.skipped,
                file = %path.display(),
                "some caption blocks could not be read"
            );
        }
        if parsed.segments.is_empty() {
            return Err(EditorError::Captions(
                bettercut_captions::CaptionError::NoCaptions,
            ));
        }

        let count = parsed.segments.len();
        self.replace_captions(parsed.segments, format!("Import {count} Captions"))
    }

    /// Put `segments` on the caption lane, replacing whatever is there, as one
    /// undo step.
    ///
    /// Shared by importing a subtitle file and by timing captions from speech
    /// (§28), because the two differ only in where the segments came from — and
    /// a second copy of "clear the lane, style them, put them low in the frame"
    /// is a second place for the two to drift apart (§46).
    ///
    /// Segments with no words are allowed: timing produces exactly those, for
    /// the user to type into. A blank title draws nothing, so the picture is
    /// unchanged until there is something to say.
    pub fn replace_captions(
        &mut self,
        segments: Vec<bettercut_captions::CaptionSegment>,
        label: impl Into<String>,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.caption_track()?;

        // The lane has to be empty, or the inserts collide with whatever is
        // already there. Replacing beats appending: importing a corrected file
        // over an old one is the common case, and the old captions are still
        // one undo away.
        let existing: Vec<ClipId> = self
            .active_sequence()
            .and_then(|s| s.text_track(track))
            .map(|t| t.clips().iter().map(|c| c.id).collect())
            .unwrap_or_default();

        let style = bettercut_text::TextStyle::caption();
        let mut commands: Vec<Command> = existing
            .into_iter()
            .map(|clip| Command::RemoveText {
                sequence,
                track,
                clip,
            })
            .collect();

        let count = segments.len();
        for segment in segments {
            let duration = segment.duration();
            let mut clip =
                bettercut_timeline::TextClip::with_duration(segment.text, segment.start, duration)?;
            clip.style = style.clone();
            // Low in the frame, where a subtitle belongs — positive y is down.
            // Not at the very edge: phone players put their own controls there.
            clip.transform.position = bettercut_timeline::Vec2::new(0.0, 0.35);
            commands.push(Command::AddText {
                sequence,
                track,
                clip: Box::new(clip),
            });
        }

        self.dispatch_group(label, commands)?;
        Ok(count)
    }

    /// Replace a sound clip's volume envelope (§24 on §20a.4's clip-gain
    /// stage).
    ///
    /// Each point is a timeline instant and a level; the instants are converted
    /// to source time here, because that is what a key is anchored to — an
    /// envelope written against the timeline would slide off the words it was
    /// put on the moment the clip was trimmed.
    ///
    /// One command for the whole shape, so it is one undo step and the journal
    /// replays it in one go. An empty list clears the envelope, which is how a
    /// duck is removed.
    ///
    /// `continuing` collapses a drag into one step, the same as every other
    /// gesture (§11: the history holds intentions, not mouse samples).
    pub fn set_gain_envelope(
        &mut self,
        clip: ClipId,
        points: &[(TimelineTime, f32)],
        continuing: bool,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let audio = self.audio_clip(clip).ok_or(EditorError::ClipKindMismatch)?;

        let already_empty = !audio
            .keyframes
            .is_animated(bettercut_timeline::AnimatedParameter::Gain);
        let keys: Vec<bettercut_timeline::Keyframe> = points
            .iter()
            .map(|(at, value)| {
                bettercut_timeline::Keyframe::new(
                    audio.source_time_at(*at),
                    *value,
                    bettercut_timeline::Interpolation::Linear,
                )
            })
            .collect();
        // Clearing an envelope that is not there is not an edit.
        if already_empty && keys.is_empty() {
            return Ok(0);
        }

        let count = keys.len();
        let label = if count == 0 {
            "Clear Volume Envelope".to_owned()
        } else {
            "Volume Envelope".to_owned()
        };
        self.dispatch_gesture(
            label,
            vec![Command::SetGainEnvelope {
                sequence,
                track,
                clip,
                keys,
            }],
            continuing,
        )?;
        Ok(count)
    }

    /// Dress every caption in the lane the same way (§27).
    ///
    /// A look belongs to the *lane*, not to a caption: subtitles that changed
    /// style halfway through would read as a mistake. Styling them one at a
    /// time through the Inspector is also thirty edits for one decision, and
    /// §11 says the history holds intentions.
    ///
    /// Titles are left alone. They are placed and dressed individually, which
    /// is the difference between a title and a caption.
    ///
    /// Returns how many captions were restyled.
    pub fn set_caption_look(
        &mut self,
        look: bettercut_text::CaptionLook,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some(track) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| t.id)
        }) else {
            return Ok(0); // no lane yet: nothing to dress
        };

        let style = bettercut_text::TextStyle::look(look);
        let clips: Vec<ClipId> = self
            .active_sequence()
            .and_then(|s| s.text_track(track))
            .map(|t| {
                t.clips()
                    .iter()
                    .filter(|clip| clip.style != style)
                    .map(|clip| clip.id)
                    .collect()
            })
            .unwrap_or_default();
        if clips.is_empty() {
            return Ok(0);
        }

        let count = clips.len();
        let commands = clips
            .into_iter()
            .map(|clip| Command::SetTextProperty {
                sequence,
                track,
                clip,
                property: crate::command::TextProperty::Style(Box::new(style.clone())),
            })
            .collect();
        self.dispatch_group(format!("{} Captions", look.label()), commands)?;
        Ok(count)
    }

    /// Write the caption lane back out (Milestone 10).
    ///
    /// The format comes from the extension. Returns how many were written.
    pub fn export_captions(&self, path: &std::path::Path) -> Result<usize, EditorError> {
        let segments = self.caption_segments();
        if segments.is_empty() {
            return Err(EditorError::Captions(
                bettercut_captions::CaptionError::NoCaptions,
            ));
        }
        let format = bettercut_captions::Format::of(path);
        bettercut_captions::write(path, &segments, format)?;
        Ok(segments.len())
    }

    /// The caption lane as segments, in order.
    ///
    /// Reads whichever text track is named [`Self::CAPTION_TRACK`], falling
    /// back to every text track when there is none — someone who typed their
    /// subtitles as titles still means them as subtitles, and refusing to
    /// export because the lane has the wrong name would be pedantry.
    pub fn caption_segments(&self) -> Vec<bettercut_captions::CaptionSegment> {
        let Some(sequence) = self.project.active() else {
            return Vec::new();
        };

        let named = sequence
            .text_tracks
            .iter()
            .find(|t| t.name == Self::CAPTION_TRACK);
        let tracks: Vec<_> = match named {
            Some(track) => vec![track],
            None => sequence.text_tracks.iter().collect(),
        };

        let mut segments: Vec<_> = tracks
            .into_iter()
            .flat_map(|t| t.clips())
            .filter(|c| !c.is_blank())
            .map(|c| {
                bettercut_captions::CaptionSegment::new(
                    c.timeline.start,
                    c.timeline.end,
                    c.text.clone(),
                )
            })
            .collect();
        segments.sort_by_key(|s| s.start.ticks());
        segments
    }

    /// The caption lane, creating it if this project has none.
    fn caption_track(&mut self) -> Result<TrackId, EditorError> {
        if let Some(id) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| t.id)
        }) {
            return Ok(id);
        }

        let sequence = self.active_sequence_id()?;
        let id = TrackId::new();
        self.dispatch(Command::AddTrack {
            sequence,
            kind: crate::command::TrackKindRepr::Text,
            name: Self::CAPTION_TRACK.to_owned(),
            id,
        })?;
        Ok(id)
    }

    /// Change how fast a clip plays (§51).
    ///
    /// Speeding up shortens the clip and slowing down lengthens it. Slowing
    /// down can fail for want of room on the track, which is reported rather
    /// than resolved by moving the neighbours — see [`ops::SetClipSpeed`].
    ///
    /// `continuing` collapses a drag across the slider into one undo step, the
    /// same as every other control (§11).
    pub fn set_clip_speed(
        &mut self,
        clip: ClipId,
        speed: bettercut_foundation::Rational,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        if !self.can_retime(clip) {
            return Err(EditorError::NoMotionToRetime);
        }

        // §12: a video file is a picture clip and a sound clip. Re-timing one
        // without the other is how they drift apart, so the link decides what
        // this applies to — and one command group means one undo (§79).
        let commands: Vec<Command> = self
            .linked_with(clip)
            .into_iter()
            .filter_map(|clip| {
                let track = self.track_of(clip)?;
                Some(Command::SetClipSpeed {
                    sequence,
                    track,
                    clip,
                    speed,
                })
            })
            .collect();

        if commands.is_empty() {
            return Err(EditorError::ClipNotFound(clip));
        }
        self.dispatch_gesture("Change speed".to_owned(), commands, continuing)
    }

    /// A slow zoom across a clip — the "Ken Burns" move that keeps a photo
    /// montage from looking like a slideshow.
    ///
    /// Written as ordinary keyframes on the scale (§24), not as a mode: the
    /// keyframe editor can then adjust it, and the renderer needs to know
    /// nothing about it.
    pub fn set_movement(&mut self, clip: ClipId, movement: Movement) -> Result<(), EditorError> {
        use bettercut_timeline::AnimatedParameter;

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        let scales = [AnimatedParameter::ScaleX, AnimatedParameter::ScaleY];
        // Whatever is there now goes first, so switching between movements
        // replaces rather than layers — including a zoom the user reversed.
        let mut commands: Vec<Command> = scales
            .iter()
            .flat_map(|parameter| {
                video
                    .keyframes
                    .track(*parameter)
                    .map(|t| t.keys().to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |key| (*parameter, key.time))
            })
            .map(|(parameter, time)| Command::RemoveKeyframe {
                sequence,
                track,
                clip,
                parameter,
                time,
            })
            .collect();

        commands.extend(
            movement
                .keyframes(video.transform.scale, video.source)
                .into_iter()
                .map(|(parameter, key)| Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    key,
                }),
        );

        if commands.is_empty() {
            return Ok(());
        }
        self.dispatch_group(format!("Movement: {}", movement.label()), commands)
    }

    /// The movement on a clip, as far as it can be told from its keys.
    ///
    /// `None` for a clip whose scale is not animated, and for one animated in
    /// some way this did not write — the interface highlights nothing rather
    /// than claiming a hand-made animation is a preset.
    pub fn movement_of(&self, clip: ClipId) -> Option<Movement> {
        use bettercut_timeline::AnimatedParameter;

        let animated = self
            .video_clip(clip)?
            .keyframes
            .track(AnimatedParameter::ScaleX);
        // No track at all is the same answer as an empty one: nothing moves.
        let keys = animated.map_or(&[][..], bettercut_timeline::KeyframeTrack::keys);
        match keys {
            [] => Some(Movement::None),
            [first, .., last] if first.value < last.value => Some(Movement::ZoomIn),
            [first, .., last] if first.value > last.value => Some(Movement::ZoomOut),
            _ => None,
        }
    }

    /// Set brightness, contrast and saturation together, as one undo step.
    ///
    /// What a "look" is: three values that only mean something as a set, so
    /// they go on and come off together rather than as three edits the user
    /// has to undo one at a time. `None` applies it to the whole video (§22's
    /// master adjustment) rather than to a clip.
    ///
    /// Static values, not keyframes: a look is a setting, not an animation. On
    /// a clip whose colour *is* animated the keys still win (§24), which is why
    /// the interface does not offer this there.
    pub fn set_color_adjust(
        &mut self,
        clip: Option<ClipId>,
        color: bettercut_timeline::ColorAdjust,
    ) -> Result<(), EditorError> {
        use crate::command::ClipProperty;

        let sequence = self.active_sequence_id()?;
        let properties = [
            ClipProperty::Brightness(color.brightness),
            ClipProperty::Contrast(color.contrast),
            ClipProperty::Saturation(color.saturation),
        ];
        let commands = match clip {
            Some(clip) => {
                let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
                properties
                    .into_iter()
                    .map(|property| Command::SetClipProperty {
                        sequence,
                        track,
                        clip,
                        property,
                    })
                    .collect()
            }
            None => properties
                .into_iter()
                .map(|property| Command::SetSequenceProperty { sequence, property })
                .collect(),
        };
        self.dispatch_group("Apply Look", commands)
    }

    // ---- markers (§10) ----

    /// The active sequence's markers, in time order.
    pub fn markers(&self) -> &[bettercut_timeline::Marker] {
        self.active_sequence().map_or(&[], |s| s.markers.as_slice())
    }

    /// Add a marker at `at`, or take away the one already there — what the M
    /// key does at the playhead. Returns whether a marker is there afterwards.
    ///
    /// Snapped to the frame grid first (§9, §76), so a mark lands where a cut
    /// could.
    pub fn toggle_marker(&mut self, at: TimelineTime) -> Result<bool, EditorError> {
        let at = self.snap_to_frame(self.active_sequence_id()?, at);
        let mut markers = self.markers().to_vec();
        let added = match markers.iter().position(|m| m.time == at) {
            Some(index) => {
                markers.remove(index);
                false
            }
            None => {
                markers.push(bettercut_timeline::Marker::at(at));
                true
            }
        };
        self.replace_markers(markers)?;
        Ok(added)
    }

    /// Add markers at every one of `times`, keeping the ones already there.
    /// One undo step for the lot. Returns how many were new.
    pub fn add_markers(&mut self, times: &[TimelineTime]) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let before = self.markers().len();
        let mut markers = self.markers().to_vec();
        markers.extend(
            times
                .iter()
                .map(|t| bettercut_timeline::Marker::at(self.snap_to_frame(sequence, *t))),
        );
        let markers = bettercut_timeline::marker::normalized(markers);
        let added = markers.len().saturating_sub(before);
        if added > 0 {
            self.replace_markers(markers)?;
        }
        Ok(added)
    }

    pub fn clear_markers(&mut self) -> Result<(), EditorError> {
        if self.markers().is_empty() {
            return Ok(());
        }
        self.replace_markers(Vec::new())
    }

    fn replace_markers(
        &mut self,
        markers: Vec<bettercut_timeline::Marker>,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetMarkers { sequence, markers })
    }

    /// Set an audio track's volume and pan (§20a.4).
    ///
    /// `continuing` collapses a drag into one undo step (§11).
    pub fn set_track_mix(
        &mut self,
        track: TrackId,
        gain: f32,
        pan: f32,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch_gesture(
            "Change track volume".to_owned(),
            vec![Command::SetTrackMix {
                sequence,
                track,
                gain,
                pan,
            }],
            continuing,
        )
    }

    /// Set how a clip's sound fades in and out.
    ///
    /// Asked of a picture, it reaches the sound linked to it (§12): the user
    /// selected "the clip", and on a video file the only thing a fade on sound
    /// can mean is that clip's sound. A clip with no sound at all is refused.
    ///
    /// `continuing` collapses a drag into one undo step (§11).
    pub fn set_clip_fades(
        &mut self,
        clip: ClipId,
        fade_in: TimelineTime,
        fade_out: TimelineTime,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let commands: Vec<Command> = self
            .linked_with(clip)
            .into_iter()
            .filter(|c| self.audio_clip(*c).is_some())
            .filter_map(|clip| {
                Some(Command::SetClipFades {
                    sequence,
                    track: self.track_of(clip)?,
                    clip,
                    fade_in,
                    fade_out,
                })
            })
            .collect();
        if commands.is_empty() {
            return Err(EditorError::ClipKindMismatch);
        }
        self.dispatch_gesture("Change fade".to_owned(), commands, continuing)
    }

    /// Whether a clip has motion to re-time (§51).
    ///
    /// A held frame and a photo do not: both are one picture, and a speed
    /// change takes a clip's length from its source range — which would shrink
    /// a two-second hold to the single frame it holds.
    pub fn can_retime(&self, clip: ClipId) -> bool {
        let Some(video) = self.video_clip(clip) else {
            // Sound re-times fine, and so does anything this cannot see.
            return true;
        };
        if video.frozen {
            return false;
        }
        self.project
            .media_asset(video.media_id)
            .is_none_or(|asset| !asset.is_still())
    }

    /// `clip` together with anything sharing its link (§12).
    ///
    /// Just `clip` when it is unlinked, which is every clip that was not
    /// placed from a file carrying both picture and sound.
    pub fn linked_with(&self, clip: ClipId) -> Vec<ClipId> {
        let Some(sequence) = self.project.active() else {
            return vec![clip];
        };

        let link = sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(clip).and_then(|c| c.link))
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).and_then(|c| c.link))
            });
        let Some(link) = link else {
            return vec![clip];
        };

        let video = sequence
            .video_tracks
            .iter()
            .flat_map(|t| t.clips())
            .filter(|c| c.link == Some(link))
            .map(|c| c.id);
        let audio = sequence
            .audio_tracks
            .iter()
            .flat_map(|t| t.clips())
            .filter(|c| c.link == Some(link))
            .map(|c| c.id);
        video.chain(audio).collect()
    }

    /// Put a transition on the end of a clip (§25).
    ///
    /// The duration asked for is a request: what gets stored is clamped to what
    /// the two clips can actually supply. See [`Self::transition_room`] for
    /// showing that limit before the user runs into it.
    pub fn set_transition(
        &mut self,
        clip: ClipId,
        kind: TransitionKind,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        // Reuse the length already there when only the kind is changing, so
        // switching a 2-second dissolve to a fade does not silently shorten it.
        let duration = self
            .video_clip(clip)
            .and_then(|c| c.transition_out)
            .map(|t| t.duration)
            .unwrap_or(DEFAULT_TRANSITION);
        self.dispatch(Command::SetTransition {
            sequence,
            track,
            clip,
            transition: Some(Transition::new(kind, duration)),
        })
    }

    /// Change how long a clip's transition runs, keeping its kind.
    ///
    /// A no-op when there is no transition there: setting a length on nothing
    /// is not a request to create one, and guessing which kind they meant would
    /// be a worse answer than doing nothing.
    pub fn set_transition_duration(
        &mut self,
        clip: ClipId,
        duration: TimelineTime,
    ) -> Result<(), EditorError> {
        let Some(existing) = self.video_clip(clip).and_then(|c| c.transition_out) else {
            return Ok(());
        };
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        self.dispatch(Command::SetTransition {
            sequence,
            track,
            clip,
            transition: Some(Transition::new(existing.kind, duration)),
        })
    }

    pub fn remove_transition(&mut self, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        self.dispatch(Command::SetTransition {
            sequence,
            track,
            clip,
            transition: None,
        })
    }

    /// The longest transition of `kind` this cut can support, or `None` when
    /// there is no cut at the end of this clip at all.
    ///
    /// The interface asks this so it can bound the slider and say why a
    /// crossfade is unavailable, rather than offering one and rejecting it.
    pub fn transition_room(&self, clip: ClipId, kind: TransitionKind) -> Option<TimelineTime> {
        let sequence = self.active_sequence_id().ok()?;
        let track = self.track_of(clip)?;
        ops::transition_room(&self.project, sequence, track, clip, kind)
    }

    /// Add a keyframe for every parameter of one inspector control, or delete
    /// them if they are already there (§24).
    ///
    /// One button rather than the usual stopwatch-plus-diamond pair: the first
    /// click starts the animation from wherever the control is now, later
    /// clicks add or remove a key at the playhead, and there is no mode to be
    /// in (§41 — the interface explains itself).
    pub fn toggle_keyframe(
        &mut self,
        clip: ClipId,
        property: crate::command::ClipProperty,
    ) -> Result<(), EditorError> {
        let at = self
            .source_time_at_playhead(clip)
            .ok_or(EditorError::PlayheadOffClip)?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;

        let parameters: Vec<_> = property.animated().into_iter().flatten().collect();
        // Removing only when *every* parameter of the control has a key there,
        // so a half-keyed row fills in rather than emptying out.
        let remove = !parameters.is_empty()
            && parameters
                .iter()
                .all(|(parameter, _)| video.keyframes.get(*parameter, at).is_some());

        let commands: Vec<Command> = parameters
            .into_iter()
            .map(|(parameter, value)| {
                if remove {
                    Command::RemoveKeyframe {
                        sequence,
                        track,
                        clip,
                        parameter,
                        time: at,
                    }
                } else {
                    Command::SetKeyframe {
                        sequence,
                        track,
                        clip,
                        parameter,
                        key: Keyframe::new(at, value, Interpolation::default()),
                    }
                }
            })
            .collect();

        let verb = if remove { "Delete" } else { "Add" };
        self.dispatch_group(format!("{verb} {} Keyframe", property.kind()), commands)
    }

    /// Adjust the finished picture rather than one clip (§22).
    ///
    /// Same controls, different subject. `continuing` behaves as it does for a
    /// clip: true while a slider is being dragged, so the whole drag is one
    /// undo step (§11).
    pub fn set_sequence_value(
        &mut self,
        property: crate::command::ClipProperty,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch_gesture(
            format!("Change {} for the whole video", property.kind()),
            vec![Command::SetSequenceProperty { sequence, property }],
            continuing,
        )
    }

    /// Put one whole-video control back to its default.
    pub fn reset_sequence_parameter(
        &mut self,
        property: crate::command::ClipProperty,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetSequenceProperty {
            sequence,
            property: Self::reset_values(property),
        })
    }

    /// Take an asset out of the library (§12).
    ///
    /// Fails while any clip uses it. That refusal is the model's, and it is the
    /// right one: removing the asset silently would leave those clips pointing
    /// at nothing, and removing the clips too would throw away an edit the user
    /// did not ask to lose.
    pub fn remove_media(
        &mut self,
        media: bettercut_foundation::MediaId,
    ) -> Result<(), EditorError> {
        self.dispatch(Command::RemoveMedia { media })
    }

    /// Whether any clip still references this asset, so the interface can say
    /// why removing is unavailable before the user tries.
    pub fn media_is_used(&self, media: bettercut_foundation::MediaId) -> bool {
        self.project.media_is_used(media)
    }

    /// The video clip behind an id, for reading its animation.
    pub fn video_clip(&self, clip: ClipId) -> Option<&bettercut_timeline::VideoClip> {
        self.project
            .active()?
            .video_tracks
            .iter()
            .find_map(|track| track.get(clip))
    }

    /// The audio clip behind an id, for reading its gain and fades.
    pub fn audio_clip(&self, clip: ClipId) -> Option<&bettercut_timeline::AudioClip> {
        self.project
            .active()?
            .audio_tracks
            .iter()
            .find_map(|track| track.get(clip))
    }

    /// Where in a clip's source media the playhead is sitting.
    ///
    /// `None` when the playhead is not over the clip at all, which is the
    /// signal that there is nothing to key: a keyframe is always placed at the
    /// frame the user is looking at.
    pub fn source_time_at_playhead(&self, clip: ClipId) -> Option<MediaTime> {
        let clip = self.video_clip(clip)?;
        let playhead = self.playhead();
        clip.timeline
            .contains(playhead)
            .then(|| clip.source_time_at(playhead))
    }

    /// Add or replace one keyframe (§24).
    ///
    /// The single-parameter primitive. The inspector goes through
    /// [`Self::set_clip_value`] instead, which knows which of its parameters
    /// are animated; this is for callers that already do.
    pub fn set_keyframe(
        &mut self,
        clip: ClipId,
        parameter: bettercut_timeline::AnimatedParameter,
        key: Keyframe,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        self.dispatch(Command::SetKeyframe {
            sequence,
            track,
            clip,
            parameter,
            key,
        })
    }

    /// Delete the keyframe at `time` (§24).
    pub fn remove_keyframe(
        &mut self,
        clip: ClipId,
        parameter: bettercut_timeline::AnimatedParameter,
        time: MediaTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        self.dispatch(Command::RemoveKeyframe {
            sequence,
            track,
            clip,
            parameter,
            time,
        })
    }

    /// Point one asset at a file the user located (§66).
    ///
    /// Reads the new file's size here, outside the command, so the command
    /// itself stays deterministic for §38.2's replay.
    pub fn relink_media(
        &mut self,
        media: bettercut_foundation::MediaId,
        path: impl AsRef<Path>,
    ) -> Result<(), EditorError> {
        let path = path.as_ref().to_path_buf();
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.dispatch(Command::RelinkMedia {
            media,
            path,
            file_size,
        })
    }

    /// "Locate folder" (§66): relink every missing asset found in `folder`.
    ///
    /// Matching is by file name, confirmed by size where one was recorded. That
    /// is deliberately conservative — a same-named file of a different size is
    /// a different file, and silently swapping it under existing cuts would be
    /// worse than leaving the media missing.
    ///
    /// One undo step for the whole folder (§79): the user performed one action.
    /// Returns how many were relinked.
    pub fn relink_from_folder(&mut self, folder: impl AsRef<Path>) -> Result<usize, EditorError> {
        let folder = folder.as_ref();

        // Non-recursive. Walking a whole drive from a mis-chosen folder would
        // hang the UI, and §66 asks for "locate folder", not "search the disk".
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Ok(0);
        };
        let candidates: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();

        let mut commands = Vec::new();
        for asset in self.project.media.iter().filter(|m| m.missing) {
            let Some(found) = candidates
                .iter()
                .find(|c| asset.matches_relink_candidate(c))
            else {
                continue;
            };
            commands.push(Command::RelinkMedia {
                media: asset.id,
                path: found.clone(),
                file_size: std::fs::metadata(found).map(|m| m.len()).unwrap_or(0),
            });
        }

        let count = commands.len();
        if count == 0 {
            return Ok(0);
        }

        let label = if count == 1 {
            "Relink Media".to_owned()
        } else {
            format!("Relink {count} Files")
        };
        self.dispatch_group(label, commands)?;
        Ok(count)
    }

    /// Re-check every asset against the filesystem (§66).
    ///
    /// Returns how many are still missing. Not a command: it observes the world
    /// rather than changing the user's project, and replaying an observation
    /// would be meaningless.
    pub fn refresh_missing_media(&mut self) -> usize {
        let missing = self.project.refresh_missing_media();
        for asset in self.project.media.iter().filter(|m| m.missing) {
            self.events.emit(Event::MediaMissing(asset.id));
        }
        self.events.emit(Event::ProjectChanged);
        missing
    }

    /// Set the active sequence's output format (§8, §36).
    pub fn set_sequence_format(
        &mut self,
        resolution: Resolution,
        frame_rate: FrameRate,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetSequenceFormat {
            sequence,
            resolution: resolution.into(),
            frame_rate,
        })
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
        self.journal_commands([command]);
    }

    /// Append several commands, then snapshot if one is due.
    ///
    /// The check comes after *all* of them. The project already contains the
    /// whole group by the time it is journalled, so a snapshot taken partway
    /// through would hold the rest of the group too — and replay would then
    /// apply those commands a second time on top of it.
    fn journal_commands(&mut self, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            self.journal.append(&command);
        }

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

            Command::RelinkMedia {
                media,
                path,
                file_size,
            } => Ok(Box::new(ops::RelinkMedia::new(media, path, file_size))),

            Command::RemoveMedia { media } => Ok(Box::new(ops::RemoveMedia::new(media))),

            Command::SetSequenceProperty { sequence, property } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetSequenceProperty::new(sequence, property)))
            }

            Command::SetClipProperty {
                sequence,
                track,
                clip,
                property,
            } => Ok(Box::new(ops::SetClipProperty::new(
                sequence, track, clip, property,
            ))),

            Command::AddText {
                sequence,
                track,
                clip,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::AddText::new(sequence, track, *clip)))
            }

            Command::RemoveText {
                sequence,
                track,
                clip,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::RemoveText::new(sequence, track, clip)))
            }

            Command::SetTextProperty {
                sequence,
                track,
                clip,
                property,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetTextProperty::new(
                    sequence, track, clip, property,
                )))
            }

            Command::Unlink { sequence, link } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::Unlink::new(sequence, link)))
            }

            Command::SetClipSpeed {
                sequence,
                track,
                clip,
                speed,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetClipSpeed::new(
                    sequence, track, clip, speed,
                )))
            }

            Command::SetMarkers { sequence, markers } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetMarkers::new(sequence, markers)))
            }

            Command::SetTrackMix {
                sequence,
                track,
                gain,
                pan,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetTrackMix::new(sequence, track, gain, pan)))
            }

            Command::SetClipFades {
                sequence,
                track,
                clip,
                fade_in,
                fade_out,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetClipFades::new(
                    sequence, track, clip, fade_in, fade_out,
                )))
            }

            Command::SetTransition {
                sequence,
                track,
                clip,
                transition,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetTransition::new(
                    sequence, track, clip, transition,
                )))
            }

            Command::SetGainEnvelope {
                sequence,
                track,
                clip,
                keys,
            } => Ok(Box::new(ops::SetGainEnvelope::new(
                sequence, track, clip, keys,
            ))),

            Command::SetKeyframe {
                sequence,
                track,
                clip,
                parameter,
                key,
            } => Ok(Box::new(ops::SetKeyframe::set(
                sequence, track, clip, parameter, key,
            ))),

            Command::RemoveKeyframe {
                sequence,
                track,
                clip,
                parameter,
                time,
            } => Ok(Box::new(ops::SetKeyframe::remove(
                sequence, track, clip, parameter, time,
            ))),

            Command::SetSequenceFormat {
                sequence,
                resolution,
                frame_rate,
            } => Ok(Box::new(ops::SetSequenceFormat::new(
                sequence,
                resolution.into(),
                frame_rate,
            ))),

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
                relink,
            } => {
                self.require_track(sequence, track)?;
                // §76: "The split point must be snapped to a frame boundary in
                // TimelineTime ticks before the command is constructed."
                Ok(Box::new(
                    ops::SplitClip::new(
                        sequence,
                        track,
                        clip,
                        self.snap_to_frame(sequence, at),
                        left,
                        right,
                    )
                    .relinked(relink),
                ))
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

    pub(crate) fn active_sequence_id(&self) -> Result<SequenceId, EditorError> {
        self.project
            .active()
            .map(|s| s.id)
            .ok_or_else(|| EditorError::SequenceNotFound(SequenceId::new()))
    }
}

/// Milestone 3 editing (§10, §85).
impl Editor {
    /// Move a clip, and whatever it is linked to by the same amount (§12).
    ///
    /// A partner stays on its own track and moves in time only: dragging the
    /// picture from V1 to V2 is a decision about the picture, and the sound has
    /// no V2 to go to. It moves by the same *delta* rather than to the same
    /// start, so a pair that has drifted apart — trimmed separately and then
    /// re-linked — keeps whatever offset it had.
    ///
    /// One undo step for the lot (§79), and all or nothing: if the sound cannot
    /// move where the picture is going, neither does.
    pub fn move_clip(
        &mut self,
        from_track: TrackId,
        to_track: TrackId,
        clip: ClipId,
        new_start: TimelineTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let primary = Command::MoveClip {
            sequence,
            from_track,
            to_track,
            clip,
            new_start,
        };

        let mut commands = self.move_commands(sequence, clip, primary, new_start);
        if commands.len() == 1 {
            return self.dispatch(commands.remove(0));
        }
        self.dispatch_group("Move Clip".to_owned(), commands)
    }

    /// Moving one clip: the move itself, then its partners by the same delta.
    ///
    /// Shared with the keyboard nudge, so a nudged pair stays together exactly
    /// as a dragged one does (§12).
    fn move_commands(
        &self,
        sequence: SequenceId,
        clip: ClipId,
        primary: Command,
        new_start: TimelineTime,
    ) -> Vec<Command> {
        let mut commands = vec![primary];
        let Some(old_start) = self.clip_start(clip) else {
            return commands;
        };
        let delta = new_start.ticks() - old_start.ticks();

        for partner in self.linked_with(clip).into_iter().filter(|c| *c != clip) {
            let (Some(track), Some(start)) = (self.track_of(partner), self.clip_start(partner))
            else {
                continue;
            };
            commands.push(Command::MoveClip {
                sequence,
                from_track: track,
                to_track: track,
                clip: partner,
                new_start: TimelineTime::from_ticks(start.ticks() + delta),
            });
        }
        commands
    }

    /// Nudge clips along the timeline by whole frames (§57).
    ///
    /// Everything selected moves together, as one undo step, with linked
    /// partners coming along. Moving right is applied right-to-left (and the
    /// reverse going left) so a run of selected clips does not collide with
    /// itself on the way. Nothing moves past the start of the timeline: the
    /// whole nudge is shortened instead, so a selection keeps its spacing.
    pub fn nudge_clips(&mut self, clips: &[ClipId], frames: i64) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        if frames == 0 || clips.is_empty() {
            return Ok(0);
        }
        let per_frame = self
            .active_sequence()
            .map_or(1, |s| s.ticks_per_frame().max(1));

        let mut targets: Vec<(ClipId, TrackId, TimelineTime)> = clips
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|clip| Some((clip, self.track_of(clip)?, self.clip_start(clip)?)))
            .collect();
        if targets.is_empty() {
            return Ok(0);
        }

        let wanted = frames * per_frame;
        let earliest = targets.iter().map(|(_, _, start)| start.ticks()).min();
        let delta = match earliest {
            Some(earliest) if earliest + wanted < 0 => -earliest,
            _ => wanted,
        };
        if delta == 0 {
            return Ok(0);
        }

        targets.sort_by_key(|(_, _, start)| start.ticks());
        if delta > 0 {
            targets.reverse();
        }

        let count = targets.len();
        let commands: Vec<Command> = targets
            .into_iter()
            .flat_map(|(clip, track, start)| {
                let new_start = TimelineTime::from_ticks(start.ticks() + delta);
                self.move_commands(
                    sequence,
                    clip,
                    Command::MoveClip {
                        sequence,
                        from_track: track,
                        to_track: track,
                        clip,
                        new_start,
                    },
                    new_start,
                )
            })
            .collect();

        self.dispatch_group("Nudge".to_owned(), commands)?;
        Ok(count)
    }

    /// Where a video or audio clip starts, for the linked edits above.
    fn clip_start(&self, clip: ClipId) -> Option<TimelineTime> {
        let sequence = self.project.active()?;
        sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(clip).map(|c| c.timeline.start))
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).map(|c| c.timeline.start))
            })
            .or_else(|| sequence.text_clip(clip).map(|c| c.timeline.start))
    }

    /// The same, for the end.
    fn clip_end(&self, clip: ClipId) -> Option<TimelineTime> {
        let sequence = self.project.active()?;
        sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(clip).map(|c| c.timeline.end))
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).map(|c| c.timeline.end))
            })
            .or_else(|| sequence.text_clip(clip).map(|c| c.timeline.end))
    }

    /// Detach a clip from whatever it is linked to (§12).
    pub fn unlink(&mut self, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let link = self.link_of(clip).ok_or(EditorError::NothingLinked)?;
        self.dispatch(Command::Unlink { sequence, link })
    }

    /// The link a clip carries, if any.
    pub fn link_of(&self, clip: ClipId) -> Option<bettercut_foundation::LinkId> {
        let sequence = self.project.active()?;
        sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(clip).and_then(|c| c.link))
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).and_then(|c| c.link))
            })
    }

    /// Trim one edge of a clip, and the same edge of whatever it is linked to
    /// (§12).
    ///
    /// By the same delta, for the reason given on [`Self::move_clip`]. All or
    /// nothing: a file's sound track is often a few frames shorter than its
    /// picture, and trimming the picture's end outward past where the sound
    /// runs out refuses the whole trim rather than leaving the pair uneven.
    pub fn trim_clip(
        &mut self,
        track: TrackId,
        clip: ClipId,
        edge: TrimEdge,
        to: TimelineTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let primary = Command::TrimClip {
            sequence,
            track,
            clip,
            edge,
            to,
        };

        let edge_of = |editor: &Self, clip: ClipId| match edge {
            TrimEdge::Start => editor.clip_start(clip),
            TrimEdge::End => editor.clip_end(clip),
        };
        let Some(was) = edge_of(self, clip) else {
            return self.dispatch(primary);
        };
        let delta = to.ticks() - was.ticks();

        let mut commands = vec![primary];
        for partner in self.linked_with(clip).into_iter().filter(|c| *c != clip) {
            let (Some(track), Some(at)) = (self.track_of(partner), edge_of(self, partner)) else {
                continue;
            };
            commands.push(Command::TrimClip {
                sequence,
                track,
                clip: partner,
                edge,
                to: TimelineTime::from_ticks(at.ticks() + delta),
            });
        }

        if commands.len() == 1 {
            return self.dispatch(commands.remove(0));
        }
        self.dispatch_group("Trim Clip".to_owned(), commands)
    }

    /// Open a gap of `duration` across every track at `at` (§10's insert).
    ///
    /// Everything from `at` onwards moves right by the same amount — clips on
    /// every track, and markers — so the edit keeps its sync. A clip the
    /// instant falls inside is split there first, since half of it belongs on
    /// each side of the gap.
    ///
    /// One undo step for the lot (§79). Returns how many clips moved.
    pub fn insert_time(
        &mut self,
        at: TimelineTime,
        duration: TimelineTime,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self.snap_to_frame(sequence, at);
        let duration =
            TimelineTime::from_ticks(self.snap_to_frame(sequence, duration).ticks().max(0));
        if duration == TimelineTime::ZERO {
            return Ok(0);
        }
        self.staged("Insert Gap", |editor, stage| {
            editor.stage_insert_time(stage, sequence, at, duration)
        })
    }

    /// The gap itself, inside someone else's edit.
    ///
    /// Shared with the freeze frame, which is a gap with a held frame dropped
    /// into it — and has to be one undo step, not two.
    fn stage_insert_time(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        at: TimelineTime,
        duration: TimelineTime,
    ) -> Result<usize, EditorError> {
        // Split first, so a clip the gap opens inside becomes two clips with a
        // clean edge at `at`. Staged rather than dispatched, so the splits and
        // the moves that depend on them are one undo step.
        let straddling: Vec<ClipId> = self
            .clips_from(at, true)
            .into_iter()
            .map(|(_, clip, _)| clip)
            .collect();
        for command in self.split_commands(sequence, at, &straddling) {
            self.stage(stage, command)?;
        }

        // Now everything from `at` onwards — including the right halves just
        // made — moves right. Latest first: moving an earlier clip into space a
        // later one has not vacated yet would be refused.
        let mut moves = self.clips_from(at, false);
        moves.sort_by_key(|(_, _, start)| std::cmp::Reverse(start.ticks()));
        let count = moves.len();
        for (track, clip, start) in moves {
            self.stage(
                stage,
                Command::MoveClip {
                    sequence,
                    from_track: track,
                    to_track: track,
                    clip,
                    new_start: start + duration,
                },
            )?;
        }

        // Markers move with the picture they mark, or a beat grid would be
        // left pointing at the wrong frames from the gap onwards.
        let markers = self.markers();
        if markers.iter().any(|m| m.time >= at) {
            let moved = markers
                .iter()
                .map(|marker| {
                    let mut marker = marker.clone();
                    if marker.time >= at {
                        marker.time += duration;
                    }
                    marker
                })
                .collect();
            self.stage(
                stage,
                Command::SetMarkers {
                    sequence,
                    markers: moved,
                },
            )?;
        }

        Ok(count)
    }

    /// Hold the frame under the playhead for `duration` (§10's freeze frame).
    ///
    /// A gap is opened at the playhead and a clip that holds that one frame is
    /// dropped into it, so everything after — including the clip's own sound —
    /// keeps its place relative to everything else. Holding the picture
    /// *without* making room would leave the sound running underneath and the
    /// two out of step from there on.
    ///
    /// The held clip keeps the shot's framing and grade, so the freeze looks
    /// like the frame it came from, and it can be adjusted like any other clip
    /// afterwards. One undo step for the whole thing (§79).
    pub fn freeze_frame(
        &mut self,
        clip: ClipId,
        duration: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self.playhead;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let source = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        if !source.timeline.contains(at) {
            // Nothing to hold: the playhead is not over this clip.
            return Err(EditorError::PlayheadOffClip);
        }
        let duration =
            TimelineTime::from_ticks(self.snap_to_frame(sequence, duration).ticks().max(0));
        if duration == TimelineTime::ZERO {
            return Err(EditorError::PlayheadOffClip);
        }

        // The frame to hold, decided before anything moves. One frame of source
        // is enough to name it; the clip's length on the timeline is what says
        // how long it is held.
        let at = self.snap_to_frame(sequence, at);
        let frame = MediaTime::from_ticks(
            self.active_sequence()
                .map_or(1, |s| s.ticks_per_frame().max(1)),
        );
        let held = source.source_time_at(at);
        let range = bettercut_timeline::SourceRange::new(
            held,
            MediaTime::from_ticks(held.ticks() + frame.ticks()),
        )?;

        let mut frozen = bettercut_timeline::VideoClip::new(source.media_id, at, range)?;
        frozen.frozen = true;
        frozen.timeline = bettercut_timeline::TimelineRange::new(at, at + duration)?;
        // The shot's own look, so the hold does not jump to a different framing
        // or grade at the moment it starts.
        frozen.transform = source.transform;
        frozen.opacity = source.opacity;
        frozen.color = source.color;
        frozen.blur = source.blur;
        let id = frozen.id;

        self.staged("Freeze Frame", |editor, stage| {
            editor.stage_insert_time(stage, sequence, at, duration)?;
            editor.stage(
                stage,
                Command::AddClip {
                    sequence,
                    track,
                    clip: ClipPayload::Video(Box::new(frozen)),
                },
            )?;
            Ok(id)
        })
    }

    /// Clips at or after `at` on every track — or, with `straddling`, only the
    /// ones the instant falls strictly inside.
    fn clips_from(
        &self,
        at: TimelineTime,
        straddling: bool,
    ) -> Vec<(TrackId, ClipId, TimelineTime)> {
        let Some(sequence) = self.project.active() else {
            return Vec::new();
        };
        let wanted = |range: bettercut_timeline::TimelineRange| {
            if straddling {
                range.start < at && range.end > at
            } else {
                range.start >= at
            }
        };

        let mut found = Vec::new();
        for track in &sequence.video_tracks {
            for clip in track.clips().iter().filter(|c| wanted(c.timeline)) {
                found.push((track.id, clip.id, clip.timeline.start));
            }
        }
        for track in &sequence.audio_tracks {
            for clip in track.clips().iter().filter(|c| wanted(c.timeline)) {
                found.push((track.id, clip.id, clip.timeline.start));
            }
        }
        for track in &sequence.text_tracks {
            for clip in track.clips().iter().filter(|c| wanted(c.timeline)) {
                found.push((track.id, clip.id, clip.timeline.start));
            }
        }
        found
    }

    /// Trim one edge of every selected clip to the playhead (§57).
    ///
    /// Only clips the playhead is actually inside: trimming a clip the
    /// playhead is nowhere near would send its edge off across the timeline,
    /// which is never what the key means. Returns how many were trimmed.
    pub fn trim_to_playhead(
        &mut self,
        clips: &[ClipId],
        edge: TrimEdge,
    ) -> Result<usize, EditorError> {
        let at = self.playhead;
        let mut trimmed = 0;
        for clip in clips {
            let (Some(start), Some(end), Some(track)) = (
                self.clip_start(*clip),
                self.clip_end(*clip),
                self.track_of(*clip),
            ) else {
                continue;
            };
            if at <= start || at >= end {
                continue;
            }
            // One clip at a time, each with its partners: trimming two clips to
            // the same instant is two separate edits, and one failing must not
            // undo the other.
            if self.trim_clip(track, *clip, edge, at).is_ok() {
                trimmed += 1;
            }
        }
        Ok(trimmed)
    }

    /// Split every selected clip at the playhead, or — when nothing is
    /// selected — whatever clip the playhead is currently over (§76).
    ///
    /// Returns how many clips were cut. Splitting the clip under the playhead
    /// with no selection is what makes `S` a one-key operation, which is the
    /// single most-used edit in a rough cut.
    pub fn split_at_playhead(&mut self, selected: &[ClipId]) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let commands = self.split_commands(sequence, self.playhead, selected);
        let count = commands.len();
        if count == 0 {
            return Ok(0);
        }

        // One undo step, however many tracks were cut (§79).
        let label = if count == 1 {
            "Split Clip".to_owned()
        } else {
            format!("Split {count} Clips")
        };
        self.dispatch_group(label, commands)?;
        Ok(count)
    }

    /// Re-scale every clip for the shape of the frame (§36).
    ///
    /// Changing a sequence from landscape to vertical leaves every clip in it
    /// pillarboxed — the canvas changed and the clips did not. §36 is right
    /// that resizing must not touch the source media, but the clips still have
    /// to be *told*, and doing it one clip at a time through the Fill button is
    /// the sort of work an editor should be doing for you.
    ///
    /// `fill` covers the frame and crops what does not fit; otherwise every
    /// clip is shown whole, with bars where the shapes disagree.
    ///
    /// A clip whose scale is animated is left alone. Its keyframes *are* the
    /// framing — a Ken Burns move, a punch-in — and overwriting them with one
    /// number would throw away work that cannot be guessed back. Those are
    /// counted separately so the interface can say so.
    ///
    /// Returns `(re-scaled, left animated)`.
    pub fn reframe_clips(&mut self, fill: bool) -> Result<(usize, usize), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let output = aspect_of(sequence.resolution.width, sequence.resolution.height);

        let mut wanted: Vec<(ClipId, f32)> = Vec::new();
        let mut animated = 0;
        for track in &sequence.video_tracks {
            for clip in track.clips() {
                if clip
                    .keyframes
                    .is_animated(bettercut_timeline::AnimatedParameter::ScaleX)
                    || clip
                        .keyframes
                        .is_animated(bettercut_timeline::AnimatedParameter::ScaleY)
                {
                    animated += 1;
                    continue;
                }
                let Some(asset) = self.project.media_asset(clip.media_id) else {
                    continue; // §66: media that has gone still has a clip
                };
                let (Some(source), Some(output)) = (aspect_of(asset.width, asset.height), output)
                else {
                    continue; // a file whose size we never learned
                };
                let scale = if fill {
                    bettercut_timeline::fill_scale(source, output)
                } else {
                    1.0
                };
                // Already framed that way: re-setting it would put a step in
                // the history that changes nothing.
                if (clip.transform.scale.x - scale).abs() < 0.001
                    && (clip.transform.scale.y - scale).abs() < 0.001
                {
                    continue;
                }
                wanted.push((clip.id, scale));
            }
        }

        let label = if fill {
            "Fill the Frame"
        } else {
            "Fit in the Frame"
        };
        let count = wanted.len();
        self.staged(label, |editor, stage| {
            for (clip, scale) in wanted {
                let track = editor
                    .track_of(clip)
                    .ok_or(EditorError::ClipNotFound(clip))?;
                editor.stage(
                    stage,
                    Command::SetClipProperty {
                        sequence: sequence_id,
                        track,
                        clip,
                        property: crate::command::ClipProperty::Scale { x: scale, y: scale },
                    },
                )?;
            }
            Ok(())
        })?;
        Ok((count, animated))
    }

    /// Cut one clip at several instants at once, as one undo step (§79).
    ///
    /// What scene detection produces: a list of instants inside one clip. Doing
    /// them one at a time would work, but it would leave the user pressing undo
    /// a dozen times to get back one action — and §11 says the history holds
    /// intentions, and "cut this at its scene changes" is one intention.
    ///
    /// Instants outside the clip, or on its edges, are ignored rather than
    /// refused: detection reports what it found in the file, and whether all of
    /// it lands inside this particular clip is not something it knows.
    ///
    /// Returns how many cuts were made — counting a picture and its linked
    /// sound as one.
    pub fn split_clip_at(
        &mut self,
        clip: ClipId,
        times: &[TimelineTime],
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let (Some(track), Some(start), Some(end)) = (
            self.track_of(clip),
            self.clip_start(clip),
            self.clip_end(clip),
        ) else {
            return Err(EditorError::ClipNotFound(clip));
        };

        let mut times: Vec<TimelineTime> = times
            .iter()
            .map(|at| self.snap_to_frame(sequence, *at))
            .filter(|at| *at > start && *at < end)
            .collect();
        // Sorted so that `dedup` — which only drops neighbours — sees repeats
        // next to each other. The cuts themselves do not care about order (each
        // one looks up whatever covers its instant), but the label does: it
        // counts the pieces, and counting the same instant twice would promise
        // a piece the user never gets.
        times.sort_unstable();
        times.dedup();

        let label = format!("Split into {} Clips", times.len() + 1);
        self.staged(label, |editor, stage| {
            let mut cuts = 0;
            for at in times {
                // Whatever now covers `at` on this track: after the first cut
                // that is the right half, not the clip that was passed in.
                let Some((_, target, _)) = editor
                    .clips_from(at, true)
                    .into_iter()
                    .find(|(on, _, _)| *on == track)
                else {
                    continue;
                };
                let commands = editor.split_commands(sequence, at, &[target]);
                if commands.is_empty() {
                    continue;
                }
                for command in commands {
                    editor.stage(stage, command)?;
                }
                cuts += 1;
            }
            Ok(cuts)
        })
    }

    /// The splits that cutting at `at` would make: one per clip the instant
    /// falls strictly inside, with fresh links for the halves (§12).
    ///
    /// Separate from the dispatch so the same cuts can be made inside a larger
    /// edit — opening a gap splits whatever straddles it, and that has to land
    /// in the same undo step as the moves that follow.
    fn split_commands(
        &self,
        sequence_id: SequenceId,
        at: TimelineTime,
        selected: &[ClipId],
    ) -> Vec<Command> {
        // §12: a selected clip brings its linked partner. Splitting a video's
        // picture and leaving its sound whole would give two halves of picture
        // over one unbroken sound — the next move of either half would pull
        // the pair apart.
        let selected: Vec<ClipId> = selected
            .iter()
            .flat_map(|clip| self.linked_with(*clip))
            .collect();

        let Some(sequence) = self.project.sequence(sequence_id) else {
            return Vec::new();
        };

        // Find every clip that `at` falls strictly inside.
        let mut cuts: Vec<(TrackId, ClipId, Option<bettercut_foundation::LinkId>)> = Vec::new();
        let mut consider = |track: TrackId,
                            clip: ClipId,
                            range: bettercut_timeline::TimelineRange,
                            link: Option<bettercut_foundation::LinkId>| {
            // Strictly inside: splitting on an edge would make a zero-length clip.
            if at > range.start
                && at < range.end
                && (selected.is_empty() || selected.contains(&clip))
            {
                cuts.push((track, clip, link));
            }
        };

        for track in &sequence.video_tracks {
            for clip in track.clips() {
                consider(track.id, clip.id, clip.timeline, clip.link);
            }
        }
        for track in &sequence.audio_tracks {
            for clip in track.clips() {
                consider(track.id, clip.id, clip.timeline, clip.link);
            }
        }
        // §26: a title splits like anything else, and leaving it out here is
        // how a feature that works in the model never reaches the shortcut.
        for track in &sequence.text_tracks {
            for clip in track.clips() {
                consider(track.id, clip.id, clip.timeline, None);
            }
        }

        // One pair of fresh links per linked group being cut: every left half
        // in the group shares the first, every right half the second. A group
        // only partly under the cut still gets new links for the halves that
        // were cut, so nothing ends up tied to a clip it no longer matches.
        let mut relinks: std::collections::HashMap<
            bettercut_foundation::LinkId,
            (bettercut_foundation::LinkId, bettercut_foundation::LinkId),
        > = std::collections::HashMap::new();
        for (_, _, link) in &cuts {
            if let Some(link) = link {
                relinks.entry(*link).or_insert_with(|| {
                    (
                        bettercut_foundation::LinkId::new(),
                        bettercut_foundation::LinkId::new(),
                    )
                });
            }
        }

        cuts.into_iter()
            .map(|(track, clip, link)| Command::SplitClip {
                sequence: sequence_id,
                track,
                clip,
                at,
                left: ClipId::new(),
                right: ClipId::new(),
                relink: link.and_then(|link| relinks.get(&link).copied()),
            })
            .collect()
    }

    /// Take `ranges` out of `clip` — and whatever is linked to it — and close
    /// each gap, as one undo step (§78's "generate split/delete commands as one
    /// group").
    ///
    /// Each range is split out at both ends and ripple-deleted, working from
    /// the last range to the first so an earlier cut never moves a later one
    /// out from under the edit. The picture and its sound are cut at the same
    /// instants and pulled left by the same amounts, so they stay in sync and
    /// stay linked (§12). Ranges are snapped to the frame grid first (§76).
    ///
    /// Returns how many ranges were taken out.
    pub fn remove_ranges(
        &mut self,
        clip: ClipId,
        ranges: &[bettercut_timeline::TimelineRange],
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut ranges: Vec<bettercut_timeline::TimelineRange> = ranges
            .iter()
            .filter_map(|r| {
                bettercut_timeline::TimelineRange::new(
                    self.snap_to_frame(sequence, r.start),
                    self.snap_to_frame(sequence, r.end),
                )
                .ok()
            })
            .collect();
        // Latest first, and overlapping ranges merged: the same instant cut
        // twice would be a split of a clip that is no longer there.
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        // `current` comes after `kept` in the list, so it starts no later:
        // they overlap when it reaches past where `kept` begins.
        ranges.dedup_by(|current, kept| {
            if current.end > kept.start {
                kept.start = kept.start.min(current.start);
                kept.end = kept.end.max(current.end);
                true
            } else {
                false
            }
        });
        let members: Vec<(TrackId, ClipId)> = self
            .linked_with(clip)
            .into_iter()
            .filter_map(|c| Some((self.track_of(c)?, c)))
            .collect();
        if members.is_empty() {
            return Err(EditorError::ClipNotFound(clip));
        }

        let count = ranges.len();
        let label = if count == 1 {
            "Remove Silence".to_owned()
        } else {
            format!("Remove {count} Silences")
        };
        self.staged(label, |editor, stage| {
            let mut current = members;
            let mut removed = 0;
            for range in ranges {
                // One pair of links per split, shared by every member cut
                // there, so the pieces of the picture and of its sound stay
                // tied to each other (§12).
                let end_links = (
                    bettercut_foundation::LinkId::new(),
                    bettercut_foundation::LinkId::new(),
                );
                let start_links = (
                    bettercut_foundation::LinkId::new(),
                    bettercut_foundation::LinkId::new(),
                );
                let mut next = Vec::new();
                let mut cut_any = false;

                for (track, id) in current {
                    let Some((span, linked)) = editor.span_and_link(id) else {
                        continue;
                    };
                    if range.end <= span.start || range.start >= span.end {
                        next.push((track, id));
                        continue;
                    }
                    let (from, to) = (range.start.max(span.start), range.end.min(span.end));
                    let mut piece = id;
                    if to < span.end {
                        let left = ClipId::new();
                        editor.stage(
                            stage,
                            Command::SplitClip {
                                sequence,
                                track,
                                clip: piece,
                                at: to,
                                left,
                                right: ClipId::new(),
                                relink: linked.then_some(end_links),
                            },
                        )?;
                        piece = left;
                    }
                    if from > span.start {
                        let (left, right) = (ClipId::new(), ClipId::new());
                        editor.stage(
                            stage,
                            Command::SplitClip {
                                sequence,
                                track,
                                clip: piece,
                                at: from,
                                left,
                                right,
                                relink: linked.then_some(start_links),
                            },
                        )?;
                        next.push((track, left));
                        piece = right;
                    }
                    editor.stage(
                        stage,
                        Command::RippleDeleteClip {
                            sequence,
                            track,
                            clip: piece,
                        },
                    )?;
                    cut_any = true;
                }
                if cut_any {
                    removed += 1;
                }
                current = next;
            }
            Ok(removed)
        })
    }

    /// A video or audio clip's span, and whether it is linked (§12).
    fn span_and_link(&self, clip: ClipId) -> Option<(bettercut_timeline::TimelineRange, bool)> {
        let sequence = self.project.active()?;
        sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(clip).map(|c| (c.timeline, c.link.is_some())))
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).map(|c| (c.timeline, c.link.is_some())))
            })
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
            // §26: titles are clips on tracks like any other, and the
            // operations that look a clip up by id — move, trim, split — have
            // to find them or they silently do nothing.
            .or_else(|| sequence.text_track_of(clip))
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
                // The clipboard holds media clips; §26's titles have their own
                // add and remove, and never reach a `ClipPayload`.
                bettercut_timeline::TrackKind::Text => None,
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
            ClipPayload::Text(c) => c.timeline.end,
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

/// The shape of a picture, or `None` when its size is not known.
///
/// Floats are fine here: §74 bans them in timeline *position* arithmetic, and
/// this is geometry.
fn aspect_of(width: u32, height: u32) -> Option<f32> {
    (width > 0 && height > 0).then(|| width as f32 / height as f32)
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
                relink: None,
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
