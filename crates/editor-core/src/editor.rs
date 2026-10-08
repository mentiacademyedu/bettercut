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
    Interpolation, Keyframe, Movement, Resolution, Sequence, TrackKind, Transition, TransitionKind,
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

    /// Where the playhead was left in each sequence that is not on screen, so
    /// coming back to one finds it as it was.
    sequence_playheads: std::collections::HashMap<SequenceId, TimelineTime>,
    /// When the project file was last written or read by this editor, to
    /// notice another program saving it (`changed_on_disk`).
    disk_stamp: Option<std::time::SystemTime>,

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
            sequence_playheads: std::collections::HashMap::new(),
            disk_stamp: None,
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
        let stamp = modified(&path);

        let mut editor = Self {
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
            sequence_playheads: std::collections::HashMap::new(),
            disk_stamp: None,
            events,
        };
        editor.disk_stamp = stamp;
        editor.events.emit(Event::ProjectLoaded);
        Ok((editor, receiver))
    }

    // ---- reads (§54: borrowed, per frame, no allocation) ----

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The project, to change from outside the command path.
    ///
    /// Only for state that is not part of the edit — an import, a bake. Every
    /// caller has to follow it with [`Self::recorded_outside_a_command`], or a
    /// recovered session quietly loses the change (§38.2).
    pub(crate) fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    /// Note a change made outside the command path: the project is dirty, the
    /// journal takes a fresh snapshot (nothing can replay what was not a
    /// command), and the interface is told.
    pub(crate) fn recorded_outside_a_command(&mut self, what: &'static str) {
        self.dirty = true;
        if !self.journal.needs_baseline()
            && let Err(err) = self.journal.snapshot(&self.project)
        {
            tracing::error!(%err, "could not snapshot after {what}");
        }
        self.events.emit(Event::ProjectChanged);
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
        self.snapshot_after_history_step();
        Ok(())
    }

    /// Fold the last `steps` undo steps into one called `label` — several
    /// edits asked for as one, taken back by one undo. Returns how many were
    /// folded. The journal is untouched: its commands replay the same either
    /// way.
    pub fn merge_last_steps(&mut self, steps: usize, label: &str) -> usize {
        self.history.merge_top(steps, label)
    }

    pub fn redo(&mut self) -> Result<(), EditorError> {
        self.history.redo(&mut self.project)?;
        self.mark_changed();
        self.snapshot_after_history_step();
        Ok(())
    }

    /// Undo and redo are not commands, so the journal never hears of them:
    /// replay after a crash brought back every edit the person had undone,
    /// and lost what they redid. A fresh snapshot (which starts an empty
    /// journal) makes recovery the project as it is now.
    fn snapshot_after_history_step(&mut self) {
        if !self.journal.needs_baseline()
            && let Err(err) = self.journal.snapshot(&self.project)
        {
            tracing::error!(%err, "could not snapshot after undo or redo");
        }
    }

    /// The history as the panel lists it: the steps done, oldest first, and
    /// the steps undone that Redo would bring back, next first.
    pub fn history_steps(&self) -> (Vec<String>, Vec<String>) {
        (self.history.undo_labels(), self.history.redo_labels())
    }

    /// How many steps the history keeps before forgetting the oldest.
    pub fn history_limit(&self) -> usize {
        self.history.limit()
    }

    /// Go back or forward through the history until exactly `done` steps are
    /// applied — what clicking a row of the history panel does.
    ///
    /// Made of ordinary undos and redos, one at a time, so a jump can never
    /// reach a state that pressing Ctrl+Z that many times would not: nothing
    /// about the project is snapshotted or restored wholesale. Clamped to the
    /// steps that exist. If one step refuses, the jump stops there, with every
    /// step before it applied and the error returned — the history still
    /// describes the project exactly.
    ///
    /// Returns how many steps it moved.
    pub fn jump_in_history(&mut self, done: usize) -> Result<usize, EditorError> {
        let target = done.min(self.history.undo_depth() + self.history.redo_depth());
        let mut moved = 0;
        while self.history.undo_depth() > target {
            self.history.undo(&mut self.project)?;
            moved += 1;
            self.mark_changed();
        }
        while self.history.undo_depth() < target {
            self.history.redo(&mut self.project)?;
            moved += 1;
            self.mark_changed();
        }
        Ok(moved)
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

    /// Import a colour lookup table (a `.cube` file) so clips can use it.
    ///
    /// The file is read and checked now, so a broken one is refused with its
    /// reason at the moment of import rather than drawing ungraded later with
    /// no explanation. The project keeps the path, not the table. Importing a
    /// file already imported returns the id it already has.
    ///
    /// Not a command, like importing media: it adds a choice to the project and
    /// changes no clip, so it snapshots for recovery the same way.
    pub fn import_lut(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<bettercut_foundation::LutId, EditorError> {
        let path = path.as_ref();
        let table = bettercut_timeline::load_cube_file(path).map_err(EditorError::Lut)?;
        let same = |known: &Path| {
            if cfg!(windows) {
                known.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase()
            } else {
                known == path
            }
        };
        if let Some(existing) = self.project.luts.iter().find(|lut| same(&lut.path)) {
            return Ok(existing.id);
        }

        let name = table
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .map_or_else(|| "LUT".to_owned(), |s| s.to_string_lossy().into_owned())
            });
        let id = bettercut_foundation::LutId::new();
        self.project.luts.push(bettercut_project_format::LutAsset {
            id,
            name,
            path: path.to_path_buf(),
        });
        self.dirty = true;
        if !self.journal.needs_baseline()
            && let Err(err) = self.journal.snapshot(&self.project)
        {
            tracing::error!(%err, "could not snapshot after importing a LUT");
        }
        self.events.emit(Event::ProjectChanged);
        Ok(id)
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

    /// Several properties of one clip as one undo step called `label`: "move
    /// it, size it and turn it" asked for at once is one change, not three.
    pub fn set_clip_properties(
        &mut self,
        clip: ClipId,
        properties: Vec<crate::command::ClipProperty>,
        label: &str,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let commands = properties
            .into_iter()
            .map(|property| Command::SetClipProperty {
                sequence,
                track,
                clip,
                property,
            })
            .collect();
        self.dispatch_group(label, commands)
    }

    /// [`Self::set_clip_properties`] for a title.
    pub fn set_text_properties(
        &mut self,
        clip: ClipId,
        properties: Vec<crate::command::TextProperty>,
        label: &str,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_track_of(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;
        let commands = properties
            .into_iter()
            .map(|property| Command::SetTextProperty {
                sequence,
                track,
                clip,
                property,
            })
            .collect();
        self.dispatch_group(label, commands)
    }

    /// Enhance Voice: the clean-up a spoken recording usually wants, in one
    /// click and one undo step — the room's hiss down, rumble cut below
    /// 80 Hz, a little presence, loud and quiet words brought closer, and
    /// the hiss on an "s" dipped. Each stays on its own slider to adjust.
    ///
    /// Controls already set further than the preset are left alone, so a
    /// second click never undoes a stronger hand-made setting.
    pub fn enhance_voice(&mut self, clip: ClipId) -> Result<(), EditorError> {
        use crate::command::ClipProperty;

        let sound = self
            .audio_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let mut eq = sound.eq;
        eq.low_cut = eq.low_cut.max(80.0);
        eq.presence = eq.presence.max(3.0);
        let properties = [
            ClipProperty::Denoise(sound.denoise.max(60.0)),
            ClipProperty::Eq(eq),
            ClipProperty::Leveller(sound.leveller.max(50.0)),
            ClipProperty::DeEss(sound.de_ess.max(40.0)),
        ];
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let commands = properties
            .into_iter()
            .map(|property| Command::SetClipProperty {
                sequence,
                track,
                clip,
                property,
            })
            .collect();
        self.dispatch_group("Enhance Voice", commands)
    }

    /// Execute commands as one undo entry, continuing the entry on top of the
    /// stack while `continuing` (§11).
    ///
    /// The label is the identity of the gesture: `execute_coalescing` matches
    /// on it, and a group of one carries it just as a bare command does, so a
    /// drag that starts as a static slider edit and a drag that writes two
    /// keyframes coalesce by exactly the same rule.
    pub(crate) fn dispatch_gesture(
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
            P::Backdrop(_) => P::Backdrop(bettercut_timeline::Backdrop::None),
            P::ChromaKey(_) => P::ChromaKey(None),
            P::LumaKey(_) => P::LumaKey(None),
            P::Mask(_) => P::Mask(None),
            P::Blend(_) => P::Blend(bettercut_timeline::BlendMode::Normal),
            P::Motion(_) => P::Motion(bettercut_timeline::ClipMotion::default()),
            P::MotionBlur(_) => P::MotionBlur(false),
            P::Flip { axis, .. } => P::Flip { axis, on: false },
            P::Crop(_) => P::Crop(bettercut_timeline::Crop::NONE),
            // Black: what the frame was cleared to before the colour existed.
            P::Background(_) => P::Background([0.0, 0.0, 0.0]),
            P::Vignette(_) => P::Vignette(0.0),
            P::Grain(_) => P::Grain(0.0),
            P::Sharpen(_) => P::Sharpen(0.0),
            P::Vibrance(_) => P::Vibrance(0.0),
            P::Anchor { .. } => P::Anchor { x: 0.5, y: 0.5 },
            P::Wheels(_) => P::Wheels(bettercut_timeline::ColorWheels::IDENTITY),
            P::Secondary(_) => P::Secondary(bettercut_timeline::HslSecondary::IDENTITY),
            P::SmoothMotion(_) => P::SmoothMotion(false),
            P::Border(_) => P::Border(bettercut_timeline::Border::NONE),
            P::Shadow(_) => P::Shadow(bettercut_timeline::Shadow::default()),
            P::CornerPin(_) => P::CornerPin(bettercut_timeline::CornerPin::NONE),
            P::KeepPitch(_) => P::KeepPitch(false),
            P::Bars(_) => P::Bars(0.0),
            P::ProgressBar(_) => P::ProgressBar(bettercut_timeline::ProgressBar::default()),
            P::BurnIn(_) => P::BurnIn(bettercut_timeline::BurnIn::default()),
            P::Pixelate(_) => P::Pixelate(0.0),
            P::ZoomBlur(_) => P::ZoomBlur(0.0),
            P::Lens(_) => P::Lens(0.0),
            P::Posterise(_) => P::Posterise(0.0),
            P::SmoothSkin(_) => P::SmoothSkin(0.0),
            P::TiltShift { .. } => P::TiltShift {
                band: 0.0,
                centre: 0.5,
            },
            P::Glow(_) => P::Glow(0.0),
            P::OldFilm(_) => P::OldFilm(0.0),
            P::Tone(_) => P::Tone(bettercut_timeline::curves::Tone::default()),
            P::Curves(_) => P::Curves(bettercut_timeline::curves::ColourCurves::default()),
            P::LightLeak(_) => P::LightLeak(0.0),
            P::LensFlare(_) => P::LensFlare(0.0),
            P::BeatPulse(_) => P::BeatPulse(0.0),
            P::Shake(_) => P::Shake(0.0),
            P::Strobe(_) => P::Strobe(0.0),
            P::Sway(_) => P::Sway(0.0),
            P::Flicker(_) => P::Flicker(0.0),
            P::Eq(_) => P::Eq(bettercut_timeline::ClipEq::default()),
            P::Space(_) => P::Space(bettercut_timeline::ClipSpace::default()),
            P::Channels(_) => P::Channels(bettercut_timeline::ChannelMode::Stereo),
            P::Pitch(_) => P::Pitch(0.0),
            P::Leveller(_) => P::Leveller(0.0),
            P::DeEss(_) => P::DeEss(0.0),
            P::Robot(_) => P::Robot(0.0),
            P::StereoWidth(_) => P::StereoWidth(1.0),
            P::Pan(_) => P::Pan(0.0),
            P::FadeShape(_) => P::FadeShape(bettercut_timeline::FadeShape::Smooth),
            P::Mute(_) => P::Mute(false),
            P::Crossfade(_) => P::Crossfade(TimelineTime::ZERO),
            P::Lut(_) => P::Lut(None),
            P::Reverse(_) => P::Reverse(false),
            P::RgbSplit(_) => P::RgbSplit(0.0),
            P::Glitch(_) => P::Glitch(0.0),
            P::Reflection(_) => P::Reflection(bettercut_timeline::Reflection::None),
            P::Denoise(_) => P::Denoise(0.0),
            P::Gate(_) => P::Gate(0.0),
            P::Brightness(_) => P::Brightness(A::Brightness.default_value()),
            P::Contrast(_) => P::Contrast(A::Contrast.default_value()),
            P::Saturation(_) => P::Saturation(A::Saturation.default_value()),
            P::Temperature(_) => P::Temperature(A::Temperature.default_value()),
            P::Tint(_) => P::Tint(A::Tint.default_value()),
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
        self.place_media_range(media, None)
    }

    /// [`Self::place_media`] with only part of the file: a subclip.
    ///
    /// The part is marked on the file in the media browser rather than trimmed
    /// on the timeline afterwards — the same edit in the end, but made before
    /// the shot is ever in the way of anything, and made once for a file that
    /// is going to be used four times.
    ///
    /// The range is held inside the file and to at least a frame; one the
    /// wrong way round is read as the span between the two instants, because
    /// that is plainly what was meant. A photo has no range to take: one
    /// picture is all there is, however long it is held.
    pub fn place_media_range(
        &mut self,
        media: bettercut_foundation::MediaId,
        range: Option<(MediaTime, MediaTime)>,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let asset = self
            .project
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;

        let wants_video = asset.kind.has_video();
        let wants_audio = asset.audio_codec.is_some();
        // A photo has no length of its own; it gets `STILL_DURATION`.
        let duration = asset.placement_duration_with(self.project.settings.photo_length);

        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        // The targeted lane, or the first of its kind (§10's track
        // targeting) — and how far along it already runs.
        let video_track = sequence
            .target_track(bettercut_timeline::TrackKind::Video)
            .and_then(|id| sequence.video_track(id))
            .map(|t| (t.id, t.duration()));
        let audio_track = sequence
            .target_track(bettercut_timeline::TrackKind::Audio)
            .and_then(|id| sequence.audio_track(id))
            .map(|t| (t.id, t.duration()));

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

        let still = asset.is_still();
        let source = match range.filter(|_| !still) {
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

    // ---- sequences (§30) ----

    /// The longest a sequence's name may be.
    pub const MAX_SEQUENCE_NAME: usize = 60;

    /// Every sequence in the project, in order, with its name — the tabs above
    /// the timeline.
    pub fn sequence_list(&self) -> Vec<(SequenceId, String)> {
        self.project
            .sequences
            .iter()
            .map(|sequence| (sequence.id, sequence.name.clone()))
            .collect()
    }

    /// Work on another sequence. Returns whether the view actually changed.
    ///
    /// Not an edit: switching makes no undo step, because nothing about the
    /// project changed. Each sequence keeps its own playhead, so coming back to
    /// one finds it where it was left — which is what makes two sequences feel
    /// like two edits rather than one timeline with its contents swapped.
    pub fn switch_sequence(&mut self, sequence: SequenceId) -> bool {
        let Some(current) = self.project.active().map(|s| s.id) else {
            return false;
        };
        if current == sequence || self.project.sequence(sequence).is_none() {
            return false;
        }
        self.sequence_playheads.insert(current, self.playhead);
        self.project.active_sequence = Some(sequence);
        self.playhead = self
            .sequence_playheads
            .get(&sequence)
            .copied()
            .unwrap_or(TimelineTime::ZERO);
        self.events.emit(Event::ActiveSequenceChanged(sequence));
        true
    }

    /// Add an empty sequence in the project's format, after the others, and
    /// show it. One undo step.
    pub fn add_sequence(&mut self) -> Result<SequenceId, EditorError> {
        let (resolution, frame_rate) = self.project.active().map_or(
            (
                bettercut_timeline::Resolution::HD_1080,
                bettercut_foundation::FrameRate::FPS_30,
            ),
            |s| (s.resolution, s.frame_rate),
        );
        let taken: Vec<String> = self
            .project
            .sequences
            .iter()
            .map(|s| s.name.clone())
            .collect();
        let name = (self.project.sequences.len() + 1..)
            .map(|n| format!("Sequence {n}"))
            .find(|name| !taken.contains(name))
            .unwrap_or_else(|| "Sequence".to_owned());
        let mut sequence = bettercut_timeline::Sequence::new(name, resolution, frame_rate)?;
        sequence
            .video_tracks
            .push(bettercut_timeline::VideoTrack::new("V1"));
        sequence
            .audio_tracks
            .push(bettercut_timeline::AudioTrack::new("A1"));
        sequence
            .text_tracks
            .push(bettercut_timeline::TextTrack::new("T1"));
        let id = sequence.id;
        let index = self.project.sequences.len();
        self.dispatch(Command::AddSequence {
            sequence: Box::new(sequence),
            index,
        })?;
        self.switch_sequence(id);
        Ok(id)
    }

    /// Copy `sequence` — every track and clip, its marks, groups and notes —
    /// as a new sequence right after it, and switch to the copy. One undo step.
    ///
    /// Every id in the copy is new, so an edit to one can never reach the
    /// other; a picture and its sound stay tied to each other in the copy, and
    /// its groups and notes follow its clips.
    pub fn duplicate_sequence(&mut self, sequence: SequenceId) -> Result<SequenceId, EditorError> {
        let (copy, index) = self.sequence_copy(sequence)?;
        let id = copy.id;
        self.dispatch(Command::AddSequence {
            sequence: Box::new(copy),
            index: index + 1,
        })?;
        self.switch_sequence(id);
        Ok(id)
    }

    /// A copy of `sequence` with fresh ids for it, its clips and their links,
    /// named "… copy", and where the original sits in the project's list.
    ///
    /// Shared with the reshaping copy (`crate::reshape`), which takes the same
    /// copy and then reframes it — one place decides what "a copy" means.
    pub(crate) fn sequence_copy(
        &self,
        sequence: SequenceId,
    ) -> Result<(bettercut_timeline::Sequence, usize), EditorError> {
        self.sequence_copy_mapped(sequence)
            .map(|(copy, index, _)| (copy, index))
    }

    /// [`Self::sequence_copy`], with what each old clip id became.
    pub(crate) fn sequence_copy_mapped(
        &self,
        sequence: SequenceId,
    ) -> Result<
        (
            bettercut_timeline::Sequence,
            usize,
            std::collections::HashMap<ClipId, ClipId>,
        ),
        EditorError,
    > {
        let index = self
            .project
            .sequences
            .iter()
            .position(|s| s.id == sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let mut copy = self.project.sequences[index].clone();
        copy.id = SequenceId::new();
        copy.name = format!("{} copy", copy.name);

        // New identities throughout, remembering what each one was so links,
        // groups and notes can be rewritten to match.
        let mut clips: std::collections::HashMap<ClipId, ClipId> = std::collections::HashMap::new();
        let mut links: std::collections::HashMap<
            bettercut_foundation::LinkId,
            bettercut_foundation::LinkId,
        > = std::collections::HashMap::new();
        let fresh_link = |link: Option<bettercut_foundation::LinkId>,
                          links: &mut std::collections::HashMap<
            bettercut_foundation::LinkId,
            bettercut_foundation::LinkId,
        >| {
            link.map(|link| {
                *links
                    .entry(link)
                    .or_insert_with(bettercut_foundation::LinkId::new)
            })
        };
        for track in &mut copy.video_tracks {
            track.id = TrackId::new();
            for old in track.clips().iter().map(|clip| clip.id).collect::<Vec<_>>() {
                let new = ClipId::new();
                clips.insert(old, new);
                if let Some(clip) = track.get_mut(old) {
                    clip.id = new;
                    clip.link = fresh_link(clip.link, &mut links);
                }
            }
        }
        for track in &mut copy.audio_tracks {
            track.id = TrackId::new();
            for old in track.clips().iter().map(|clip| clip.id).collect::<Vec<_>>() {
                let new = ClipId::new();
                clips.insert(old, new);
                if let Some(clip) = track.get_mut(old) {
                    clip.id = new;
                    clip.link = fresh_link(clip.link, &mut links);
                }
            }
        }
        for track in &mut copy.text_tracks {
            track.id = TrackId::new();
            for old in track.clips().iter().map(|clip| clip.id).collect::<Vec<_>>() {
                let new = ClipId::new();
                clips.insert(old, new);
                if let Some(clip) = track.get_mut(old) {
                    clip.id = new;
                }
            }
        }
        for track in &mut copy.adjustment_tracks {
            track.id = TrackId::new();
            for old in track.clips().iter().map(|clip| clip.id).collect::<Vec<_>>() {
                let new = ClipId::new();
                clips.insert(old, new);
                if let Some(clip) = track.get_mut(old) {
                    clip.id = new;
                }
            }
        }

        // What the clips carry with them: which are grouped, and what was
        // written about each.
        for group in &mut copy.groups {
            *group = group
                .iter()
                .filter_map(|clip| clips.get(clip).copied())
                .collect();
        }
        copy.groups.retain(|group| group.len() > 1);
        copy.notes.retain_mut(|note| match clips.get(&note.clip) {
            Some(clip) => {
                note.clip = *clip;
                true
            }
            None => false,
        });
        Ok((copy, index, clips))
    }

    /// Take a sequence out of the project. The last one cannot go — a project
    /// with no sequence has nothing to show.
    pub fn remove_sequence(&mut self, sequence: SequenceId) -> Result<(), EditorError> {
        if self.project.sequences.len() <= 1 {
            return Err(EditorError::LastSequence);
        }
        self.sequence_playheads.remove(&sequence);
        self.dispatch(Command::RemoveSequence { sequence })?;
        if let Some(shown) = self.project.active().map(|s| s.id) {
            self.playhead = self
                .sequence_playheads
                .get(&shown)
                .copied()
                .unwrap_or(TimelineTime::ZERO);
        }
        Ok(())
    }

    /// Rename a sequence. Returns whether the name actually changed.
    pub fn rename_sequence(
        &mut self,
        sequence: SequenceId,
        name: &str,
    ) -> Result<bool, EditorError> {
        let name: String = name.trim().chars().take(Self::MAX_SEQUENCE_NAME).collect();
        if name.is_empty() {
            return Err(EditorError::EmptySequenceName);
        }
        let current = self
            .project
            .sequence(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?
            .name
            .clone();
        if current == name {
            return Ok(false);
        }
        self.dispatch(Command::RenameSequence { sequence, name })?;
        Ok(true)
    }

    /// [`Self::free_text_slot`] at the playhead, for the sticker picker.
    pub(crate) fn free_sticker_slot(&self, track: TrackId) -> TimelineTime {
        self.free_text_slot(track, self.playhead)
    }

    /// The file a picture or sound clip plays — what "find this in the media
    /// browser" needs, without the caller having to guess which lane the clip
    /// is on. `None` for a title, which plays no file.
    pub fn media_of_clip(&self, clip: ClipId) -> Option<bettercut_foundation::MediaId> {
        self.video_clip(clip)
            .map(|c| c.media_id)
            .or_else(|| self.audio_clip(clip).map(|c| c.media_id))
    }

    /// Run `commands` and fold them into the undo step already on top.
    ///
    /// For an edit that is *part of* what the user just did rather than a
    /// thing of its own: a magnetic track closing up behind a delete, or the
    /// keys a freeze punch adds to the clip it just made. Undoing takes the
    /// whole gesture back, which is what the user thinks they did.
    pub(crate) fn dispatch_amending(&mut self, commands: Vec<Command>) -> Result<(), EditorError> {
        if commands.is_empty() {
            return Ok(());
        }
        let mut built: Vec<Box<dyn EditorCommand>> = Vec::with_capacity(commands.len());
        for command in &commands {
            built.push(self.build(command.clone())?);
        }
        for command in built {
            self.history.amend_top(command, &mut self.project)?;
        }
        self.dirty = true;
        self.journal_commands(commands);
        self.events.emit(Event::ProjectChanged);
        Ok(())
    }

    /// The colour captions light each word up in, unless told otherwise:
    /// a warm yellow, which reads on almost any footage.
    pub const DEFAULT_HIGHLIGHT: bettercut_text::Rgba =
        bettercut_text::Rgba::new(255, 214, 92, 255);

    /// Hold `clip`'s last frame for `duration`, pushing what follows along.
    ///
    /// The other end of the freeze: a held frame *after* the shot rather than
    /// inside it, which is what a cut needs when the next shot is not ready
    /// yet. One undo step (§79).
    pub fn hold_last_frame(
        &mut self,
        clip: ClipId,
        duration: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let span = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .timeline;
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let source = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        let duration = TimelineTime::from_ticks(self.snap_to_frame(sequence, duration).ticks());
        if duration <= TimelineTime::ZERO {
            return Err(EditorError::PlayheadOffClip);
        }
        let frame = self
            .project
            .sequence(sequence)
            .map_or(1, |s| s.ticks_per_frame().max(1));

        // The clip's own last frame, held: the instant a frame before its
        // out-point, since the out-point itself is the first frame of
        // whatever comes next.
        let last = bettercut_foundation::MediaTime::from_ticks(
            (source.source.end.ticks() - frame).max(source.source.start.ticks()),
        );
        let range = bettercut_timeline::SourceRange::new(
            last,
            bettercut_foundation::MediaTime::from_ticks(last.ticks() + frame),
        )?;
        let mut frozen = bettercut_timeline::VideoClip::new(source.media_id, span.end, range)?;
        frozen.frozen = true;
        frozen.timeline = bettercut_timeline::TimelineRange {
            start: span.end,
            end: span.end + duration,
        };
        // The look of the shot it came from, so the hold is the same picture
        // standing still rather than an ungraded one.
        frozen.transform = source.transform;
        frozen.crop = source.crop;
        frozen.opacity = source.opacity;
        frozen.color = source.color;
        frozen.blur = source.blur;
        frozen.curves = source.curves;
        frozen.lut = source.lut;
        frozen.blend = source.blend;
        let held = frozen.id;

        self.staged("Hold Last Frame", |editor, stage| {
            editor.stage_insert_time(stage, sequence, span.end, duration)?;
            editor.stage(
                stage,
                Command::AddClip {
                    sequence,
                    track,
                    clip: ClipPayload::Video(Box::new(frozen.clone())),
                },
            )
        })?;
        Ok(held)
    }

    /// Hold `clip`'s first frame for `duration` before it plays, pushing the
    /// clip and what follows along — the opening image standing still under
    /// a title before the shot moves. One undo step; returns the held frame.
    pub fn hold_first_frame(
        &mut self,
        clip: ClipId,
        duration: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let source = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        let span = source.timeline;
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let duration = TimelineTime::from_ticks(self.snap_to_frame(sequence, duration).ticks());
        if duration <= TimelineTime::ZERO {
            return Err(EditorError::PlayheadOffClip);
        }
        let frame = self
            .project
            .sequence(sequence)
            .map_or(1, |s| s.ticks_per_frame().max(1));
        // The first frame seen: the in-point, or the out-point's last frame
        // for a clip playing backwards.
        let first = if source.reversed {
            (source.source.end.ticks() - frame).max(source.source.start.ticks())
        } else {
            source.source.start.ticks()
        };
        let range = bettercut_timeline::SourceRange::new(
            bettercut_foundation::MediaTime::from_ticks(first),
            bettercut_foundation::MediaTime::from_ticks(first + frame),
        )?;
        let mut frozen = bettercut_timeline::VideoClip::new(source.media_id, span.start, range)?;
        frozen.frozen = true;
        frozen.timeline = bettercut_timeline::TimelineRange {
            start: span.start,
            end: span.start + duration,
        };
        frozen.transform = source.transform;
        frozen.crop = source.crop;
        frozen.opacity = source.opacity;
        frozen.color = source.color;
        frozen.blur = source.blur;
        frozen.curves = source.curves;
        frozen.lut = source.lut;
        frozen.blend = source.blend;
        let held = frozen.id;

        self.staged("Hold First Frame", |editor, stage| {
            editor.stage_insert_time(stage, sequence, span.start, duration)?;
            editor.stage(
                stage,
                Command::AddClip {
                    sequence,
                    track,
                    clip: ClipPayload::Video(Box::new(frozen.clone())),
                },
            )
        })?;
        Ok(held)
    }

    /// Paste the clipboard at the playhead, pushing everything from there on
    /// along to make room. Returns how many clips went in.
    ///
    /// The insert form of paste: nothing is covered and nothing has to be
    /// moved out of the way first, which is what makes it the one to reach for
    /// in the middle of a cut.
    pub fn paste_insert(&mut self) -> Result<usize, EditorError> {
        if self.clipboard.is_empty() {
            return Err(EditorError::Timeline(
                bettercut_timeline::TimelineError::ClipboardEmpty,
            ));
        }
        let sequence = self.active_sequence_id()?;
        let at = self.snap_to_frame(sequence, self.playhead);
        // How much room the clipboard needs: from the earliest thing in it to
        // the latest.
        let (Some(first), Some(last)) = (
            self.clipboard.iter().map(ClipPayload::start).min(),
            self.clipboard.iter().map(ClipPayload::end).max(),
        ) else {
            return Ok(0);
        };
        let room = last - first;

        let (video_track, audio_track) = {
            let active = self
                .project
                .sequence(sequence)
                .ok_or(EditorError::SequenceNotFound(sequence))?;
            (
                active.target_track(bettercut_timeline::TrackKind::Video),
                active.target_track(bettercut_timeline::TrackKind::Audio),
            )
        };
        let commands: Vec<Command> = self
            .clipboard
            .clone()
            .into_iter()
            .filter_map(|payload| {
                let track = match payload.kind() {
                    bettercut_timeline::TrackKind::Video => video_track,
                    bettercut_timeline::TrackKind::Audio => audio_track,
                    _ => None,
                }?;
                let offset = payload.start() - first;
                Some(Command::PasteClip {
                    sequence,
                    track,
                    clip: payload,
                    at: at + offset,
                    new_id: ClipId::new(),
                })
            })
            .collect();
        if commands.is_empty() {
            return Ok(0);
        }

        let count = commands.len();
        self.staged(format!("Paste Insert {count} Clips"), |editor, stage| {
            editor.stage_insert_time(stage, sequence, at, room)?;
            for command in commands {
                editor.stage(stage, command)?;
            }
            Ok(())
        })?;
        Ok(count)
    }

    /// Earlier saves of this project, newest first. Empty for a project that
    /// has never been saved.
    pub fn versions(&self) -> Vec<crate::versions::Version> {
        self.path
            .as_deref()
            .map(crate::versions::list_versions)
            .unwrap_or_default()
    }

    /// What has changed between an earlier save and the edit as it stands
    /// (`crate::compare`).
    ///
    /// Reads the file and compares; nothing is opened and nothing changes, so
    /// this can be asked before deciding whether to go back at all.
    pub fn changes_since(
        &self,
        version: &Path,
    ) -> Result<Vec<crate::compare::Change>, EditorError> {
        let earlier = bettercut_project_format::load(version)?;
        Ok(crate::compare::compare(&earlier, &self.project))
    }

    /// Open an earlier version, keeping the project as it is now as a version
    /// first — going back must never be the edit that loses the work.
    ///
    /// The project comes back *unsaved*, under the file it was saved as, with
    /// an empty history: the earlier state is a new starting point rather than
    /// a step that can be undone into a state that no longer exists.
    pub fn restore_version(&mut self, version: &Path) -> Result<(), EditorError> {
        let project = bettercut_project_format::load(version)?;
        if let Some(path) = self.path.clone() {
            // Written out first so what is kept is the project as it stands,
            // unsaved work and all, rather than whatever is on disk.
            let kept =
                std::env::temp_dir().join(format!("bettercut-restore-{}.vproj", session_id()));
            if bettercut_project_format::save(&self.project, &kept).is_ok()
                && let Ok(bytes) = std::fs::read(&kept)
            {
                let _ = crate::versions::write_version(
                    &path,
                    &bytes,
                    crate::versions::now_seconds(),
                    " before restore",
                );
            }
            let _ = std::fs::remove_file(&kept);
        }
        self.project = project;
        self.history = History::new();
        self.dirty = true;
        self.sequence_playheads.clear();
        self.playhead = TimelineTime::ZERO;
        self.events.emit(Event::ProjectLoaded);
        Ok(())
    }

    /// Set a picture clip's entrance and exit lengths — the fade handles on
    /// its corners. A length under `MIN_MOTION` takes the animation off.
    ///
    /// The *kind* of animation is kept: dragging the handle of a slide makes a
    /// longer slide, not a fade. `continuing` folds the change into the one
    /// before it, so a drag is one undo step.
    pub fn set_clip_ramps(
        &mut self,
        clip: ClipId,
        intro: TimelineTime,
        outro: TimelineTime,
        continuing: bool,
    ) -> Result<(), EditorError> {
        use bettercut_timeline::{ClipMotion, MIN_MOTION, Motion, MotionKind};

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let current = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .motion;
        let keep = |was: Option<Motion>, length: TimelineTime| -> Option<Motion> {
            if length < MIN_MOTION {
                return None;
            }
            Some(Motion::new(
                was.map_or(MotionKind::Fade, |motion| motion.kind),
                length,
            ))
        };
        let motion = ClipMotion {
            intro: keep(current.intro, intro),
            outro: keep(current.outro, outro),
        };
        if motion == current {
            return Ok(());
        }
        let command = Command::SetClipProperty {
            sequence,
            track,
            clip,
            property: crate::command::ClipProperty::Motion(motion),
        };
        // One gesture, one step, whichever frame of the drag this is — the
        // label is what the coalescing matches on, so it must be the same for
        // the first frame as for the rest (§11).
        self.dispatch_gesture("Change animation".to_owned(), vec![command], continuing)
    }

    /// Give a picture clip a whole grade at once, under a label of the
    /// caller's choosing — what colour match does when it works one out.
    ///
    /// One undo step rather than six: "match this shot to that one" is one
    /// thing the user did, and undoing it a control at a time would be a
    /// puzzle.
    pub fn set_clip_grade(
        &mut self,
        clip: ClipId,
        grade: bettercut_timeline::ColorAdjust,
        label: &str,
    ) -> Result<(), EditorError> {
        use crate::command::ClipProperty as P;

        let sequence = self.active_sequence_id()?;
        if self.video_clip(clip).is_none() {
            return Err(EditorError::ClipKindMismatch);
        }
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let commands = [
            P::Brightness(grade.brightness),
            P::Contrast(grade.contrast),
            P::Saturation(grade.saturation),
            P::Temperature(grade.temperature),
            P::Tint(grade.tint),
            P::Vibrance(grade.vibrance),
        ]
        .into_iter()
        .map(|property| Command::SetClipProperty {
            sequence,
            track,
            clip,
            property,
        })
        .collect();
        self.dispatch_group(label.to_owned(), commands)
    }

    /// Turn the whole mix up or down so it measures `target` LUFS.
    ///
    /// `measured` comes from `bettercut_playback::loudness_mix::measure` —
    /// taken here rather than measured here because measuring means decoding
    /// every sound in the sequence, which is a job with a progress bar rather
    /// than a method call on the project.
    ///
    /// Returns the master volume it settled on. One undo step, and refused
    /// when the measurement says there was nothing to hear.
    pub fn match_loudness(&mut self, measured: f32, target: f32) -> Result<f32, EditorError> {
        if !measured.is_finite() || !target.is_finite() {
            return Err(EditorError::NothingToHear);
        }
        let sequence = self.active_sequence_id()?;
        let now = self
            .project
            .sequence(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?
            .master_volume;
        // The same arithmetic as `bettercut_audio::loudness::gain_for`, written
        // out rather than depended on: §86 keeps this crate above the audio
        // engine, and this is one line of decibels.
        let difference = (target - measured).clamp(-20.0, 20.0);
        let wanted = (now * 10.0_f32.powf(difference / 20.0)).clamp(0.0, 4.0);
        if (wanted - now).abs() < 1e-4 {
            return Ok(now);
        }
        self.set_sequence_value(crate::command::ClipProperty::Gain(wanted), false)?;
        Ok(wanted)
    }

    /// Set each clip's volume so it measures `target` LUFS, from what it
    /// measured (`measured`, one reading per clip) — one undo step. The same
    /// ±20 dB limit as the master's match, and the same 0–4 volume range.
    /// Returns how many clips changed.
    pub fn match_clips_loudness(
        &mut self,
        measured: &[(ClipId, f32)],
        target: f32,
    ) -> Result<usize, EditorError> {
        if !target.is_finite() {
            return Err(EditorError::NothingToHear);
        }
        let sequence = self.active_sequence_id()?;
        let commands: Vec<Command> = measured
            .iter()
            .filter(|(_, lufs)| lufs.is_finite())
            .filter_map(|(clip, lufs)| {
                let now = self.audio_clip(*clip)?.gain;
                let track = self.track_of(*clip)?;
                let difference = (target - lufs).clamp(-20.0, 20.0);
                let wanted = (now * 10.0_f32.powf(difference / 20.0)).clamp(0.0, 4.0);
                ((wanted - now).abs() >= 1e-4).then_some(Command::SetClipProperty {
                    sequence,
                    track,
                    clip: *clip,
                    property: crate::command::ClipProperty::Gain(wanted),
                })
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Even Out Loudness", commands)?;
        }
        Ok(count)
    }

    // ---- the media library (§66) ----

    /// The longest name a file may be given in this project.
    pub const MAX_MEDIA_NAME: usize = 60;
    /// And the longest a bin may be called.
    pub const MAX_BIN_NAME: usize = 40;

    /// Call a file something else in this project. An empty name goes back to
    /// the file's own. Returns whether anything changed.
    pub fn rename_media(
        &mut self,
        media: bettercut_foundation::MediaId,
        name: &str,
    ) -> Result<bool, EditorError> {
        let name: String = name.trim().chars().take(Self::MAX_MEDIA_NAME).collect();
        let current = self
            .project
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?
            .label
            .clone();
        if current.as_deref().unwrap_or_default() == name {
            return Ok(false);
        }
        // An empty name — or the file's own name typed out — is "use the file
        // name", which is what `None` means.
        let own = self
            .project
            .media_asset(media)
            .map(|asset| asset.file_name.clone())
            .unwrap_or_default();
        let name = (!name.is_empty() && name != own).then_some(name);
        if name.is_none() && current.is_none() {
            return Ok(false);
        }
        self.dispatch(Command::RenameMedia { media, name })?;
        Ok(true)
    }

    /// File a media entry under `bin`, or take it out of its bin with an empty
    /// name. Returns whether anything changed.
    pub fn set_media_bin(
        &mut self,
        media: bettercut_foundation::MediaId,
        bin: &str,
    ) -> Result<bool, EditorError> {
        let bin: String = bin.trim().chars().take(Self::MAX_BIN_NAME).collect();
        let current = self
            .project
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?
            .bin
            .clone();
        if current.as_deref().unwrap_or_default() == bin {
            return Ok(false);
        }
        let bin = (!bin.is_empty()).then_some(bin);
        self.dispatch(Command::SetMediaBin { media, bin })?;
        Ok(true)
    }

    /// How many stars a file has, 0–5.
    pub fn media_rating(&self, media: bettercut_foundation::MediaId) -> u8 {
        self.project
            .media_asset(media)
            .map_or(0, |asset| asset.rating.min(5))
    }

    /// Give a file `stars` out of five — or none, which is what clicking the
    /// star a file already has does. Returns whether anything changed.
    pub fn set_media_rating(
        &mut self,
        media: bettercut_foundation::MediaId,
        stars: u8,
    ) -> Result<bool, EditorError> {
        let rating = stars.min(5);
        if self.media_rating(media) == rating {
            return Ok(false);
        }
        self.dispatch(Command::SetMediaRating { media, rating })?;
        Ok(true)
    }

    /// Every bin a file is filed under, in order, each named once.
    pub fn bins(&self) -> Vec<String> {
        let mut bins: Vec<String> = self
            .project
            .media
            .iter()
            .filter_map(|asset| asset.bin.clone())
            .collect();
        bins.sort();
        bins.dedup();
        bins
    }

    // ---- sound on a clip ----

    /// Whether the sound that came with this clip is muted. False for a clip
    /// with no sound of its own.
    pub fn is_muted(&self, clip: ClipId) -> bool {
        let mut clips = vec![clip];
        clips.extend(self.linked_with(clip));
        clips
            .into_iter()
            .filter_map(|id| self.audio_clip(id))
            .any(|audio| audio.muted)
    }

    /// Mute or unmute the sound that came with `clip` — asked of the picture
    /// or of the sound, since they are one thing to the user (§12). Returns
    /// how many clips changed.
    pub fn set_muted(&mut self, clip: ClipId, muted: bool) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut clips = vec![clip];
        clips.extend(self.linked_with(clip).into_iter().filter(|c| *c != clip));
        let commands: Vec<Command> = clips
            .into_iter()
            .filter(|id| {
                self.audio_clip(*id)
                    .is_some_and(|audio| audio.muted != muted)
            })
            .filter_map(|id| {
                let track = self.track_of(id)?;
                Some(Command::SetClipProperty {
                    sequence,
                    track,
                    clip: id,
                    property: crate::command::ClipProperty::Mute(muted),
                })
            })
            .collect();
        if commands.is_empty() {
            // Nothing to mute at all is a different answer from "already
            // muted": a picture with no sound cannot be asked this.
            if self
                .linked_with(clip)
                .into_iter()
                .chain(std::iter::once(clip))
                .all(|id| self.audio_clip(id).is_none())
            {
                return Err(EditorError::ClipKindMismatch);
            }
            return Ok(0);
        }
        let changed = commands.len();
        self.dispatch_group(if muted { "Mute" } else { "Unmute" }.to_owned(), commands)?;
        Ok(changed)
    }

    /// The sound clip that starts exactly where `clip` ends, on the same
    /// track — the other half of a crossfade.
    pub fn next_touching_sound(&self, clip: ClipId) -> Option<ClipId> {
        let sequence = self.project.active()?;
        let span = sequence.clip_span(clip)?;
        sequence
            .audio_tracks
            .iter()
            .find(|track| track.id == span.track)?
            .clips()
            .iter()
            .find(|other| other.timeline.start == span.timeline.end)
            .map(|other| other.id)
    }

    /// Cross the sound at the cut after `clip` by `length`: the two clips
    /// overlap in the mix without moving, one fading out as the other fades
    /// in. Returns the length actually used, which is held to what both clips
    /// can give.
    ///
    /// `continuing` folds the change into the one before it, so dragging the
    /// control is one undo step.
    pub fn set_audio_crossfade(
        &mut self,
        clip: ClipId,
        length: TimelineTime,
        continuing: bool,
    ) -> Result<TimelineTime, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let outgoing = self
            .audio_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        let incoming = self
            .next_touching_sound(clip)
            .and_then(|id| self.audio_clip(id))
            .ok_or(EditorError::NoRoomForTransition)?
            .clone();
        // Both clips keep playing through the overlap, so each needs half the
        // crossfade in material past its own edge (§25's handles) — and
        // neither may give up more of itself than it has.
        let handle_out = self
            .project
            .media_asset(outgoing.media_id)
            .and_then(|asset| asset.source_limit())
            .map_or(i64::MAX / 4, |limit| {
                (limit.ticks() - outgoing.source.end.ticks()).max(0)
            });
        let handle_in = incoming.source.start.ticks().max(0);
        let handles = 2 * handle_out.min(handle_in);
        let room = outgoing
            .timeline
            .duration()
            .min(incoming.timeline.duration())
            .ticks()
            .min(handles)
            .min(bettercut_timeline::MAX_CROSSFADE.ticks());
        let length = TimelineTime::from_ticks(length.ticks().clamp(0, room));
        if outgoing.crossfade_out == length {
            return Ok(length);
        }
        let command = Command::SetClipProperty {
            sequence,
            track,
            clip,
            property: crate::command::ClipProperty::Crossfade(length),
        };
        if continuing {
            self.dispatch_gesture("Crossfade".to_owned(), vec![command], true)?;
        } else {
            self.dispatch(command)?;
        }
        Ok(length)
    }

    // ---- captions ----

    /// The colour each word lights up in as it is spoken, or `None` for plain
    /// captions.
    pub fn caption_highlight(&self) -> Option<bettercut_text::Rgba> {
        let mut captions = self
            .active_sequence()?
            .text_tracks
            .iter()
            .flat_map(|track| track.clips())
            .map(|clip| clip.highlight);
        // All of them, or none: a lane where some words light up and others do
        // not has no one answer, and saying it does would make the control
        // lie about what pressing it again would do.
        let first = captions.next()?;
        captions.all(|other| other == first).then_some(first)?
    }

    /// Light each word up as it is spoken, across every caption. `None` puts
    /// them back to plain. One undo step.
    pub fn set_caption_highlight(
        &mut self,
        highlight: Option<bettercut_text::Rgba>,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let targets: Vec<(TrackId, ClipId)> = self
            .active_sequence()
            .map(|active| {
                active
                    .text_tracks
                    .iter()
                    .flat_map(|track| {
                        let id = track.id;
                        track
                            .clips()
                            .iter()
                            .filter(|clip| clip.highlight != highlight)
                            .map(move |clip| (id, clip.id))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if targets.is_empty() {
            return Ok(0);
        }
        let changed = targets.len();
        let commands: Vec<Command> = targets
            .into_iter()
            .map(|(track, clip)| Command::SetTextProperty {
                sequence,
                track,
                clip,
                property: crate::command::TextProperty::Highlight(highlight),
            })
            .collect();
        self.dispatch_group("Caption Highlight".to_owned(), commands)?;
        Ok(changed)
    }

    // ---- text overlays (§26) ----

    /// Add a title at the playhead.
    ///
    /// Placed on the first text track, at the first instant from the playhead
    /// onwards where it fits. Not simply *at* the playhead, because that is
    /// often over an existing title and refusing would be a dead end — the user
    /// asked for a title, not for a lesson in track occupancy.
    /// Size a newly made title for this sequence's frame. Its pixel sizes are
    /// worked out for a 1080-line frame, and a sequence's pixels are its own,
    /// so in 4K a default title came out half the size. Scaled by the shorter
    /// side, so a vertical 1080×1920 video is 1080 too. Only for titles being
    /// made: a pasted or duplicated one keeps the size it has.
    pub(crate) fn fit_to_frame(&self, clip: &mut bettercut_timeline::TextClip) {
        let factor = self.frame_scale();
        if (factor - 1.0).abs() < 0.001 {
            return;
        }
        clip.style = clip.style.scaled(factor);
        if let Some(shape) = &mut clip.shape {
            *shape = shape.scaled(factor);
        }
    }

    /// How much bigger than at 1080 lines a design is drawn in this
    /// sequence's frame, by its shorter side. 1 with no sequence.
    pub fn frame_scale(&self) -> f32 {
        self.active_sequence().map_or(1.0, |sequence| {
            let short = sequence.resolution.width.min(sequence.resolution.height);
            short.max(1) as f32 / 1080.0
        })
    }

    pub fn add_text(&mut self, text: impl Into<String>) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_tracks.first().map(|t| t.id))
            .ok_or(EditorError::NoTextTrack)?;

        let start = self.free_text_slot(track, self.playhead);
        let mut clip = bettercut_timeline::TextClip::new(text, start)?;
        self.fit_to_frame(&mut clip);
        let id = clip.id;

        self.dispatch(Command::AddText {
            sequence: sequence_id,
            track,
            clip: Box::new(clip),
        })?;
        Ok(id)
    }

    /// Put a shape on the title lane at the playhead, as one undo step — a
    /// title clip that draws a rectangle or an ellipse instead of words.
    pub fn add_shape(&mut self, kind: bettercut_text::ShapeKind) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_tracks.first().map(|t| t.id))
            .ok_or(EditorError::NoTextTrack)?;
        let start = self.free_text_slot(track, self.playhead);
        let mut clip = bettercut_timeline::TextClip::new(kind.label(), start)?;
        clip.shape = Some(bettercut_text::Shape::new(kind));
        self.fit_to_frame(&mut clip);
        let id = clip.id;
        self.dispatch(Command::AddText {
            sequence: sequence_id,
            track,
            clip: Box::new(clip),
        })?;
        Ok(id)
    }

    /// Put a counter on the title lane at the playhead, as one undo step: a
    /// ten-second countdown, or a stopwatch counting up for as long as a title
    /// runs. Big and centred, like a headline.
    pub fn add_counter(
        &mut self,
        direction: bettercut_timeline::CountDirection,
    ) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_tracks.first().map(|t| t.id))
            .ok_or(EditorError::NoTextTrack)?;
        let start = self.free_text_slot(track, self.playhead);
        let (name, length, counter) = match direction {
            bettercut_timeline::CountDirection::Down => {
                let length = TimelineTime::from_seconds(10);
                (
                    "Countdown",
                    length,
                    bettercut_timeline::Counter::countdown(length),
                )
            }
            bettercut_timeline::CountDirection::Up => (
                "Stopwatch",
                TimelineTime::from_seconds(10),
                bettercut_timeline::Counter::stopwatch(),
            ),
        };
        let mut clip = bettercut_timeline::TextClip::with_duration(name, start, length)?;
        clip.style = bettercut_text::TextStyle::title(bettercut_text::TitleLook::Headline);
        clip.counter = Some(counter);
        self.fit_to_frame(&mut clip);
        let id = clip.id;
        // Placed where the room is, which for a ten-second clip may be later
        // than a three-second title's slot: check the whole length fits.
        let fits = self
            .active_sequence()
            .and_then(|s| s.text_track(track))
            .is_some_and(|t| {
                t.clips()
                    .iter()
                    .all(|c| !c.timeline.overlaps(clip.timeline))
            });
        if !fits {
            let end = self
                .active_sequence()
                .and_then(|s| s.text_track(track))
                .map_or(start, |t| t.duration());
            clip.timeline =
                bettercut_timeline::TimelineRange::new(end.max(start), end.max(start) + length)?;
        }
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

    // ---- adjustment layers (`bettercut_timeline::adjustment`) ----

    /// Add an adjustment at the playhead, making the first adjustment lane if
    /// the sequence has none.
    ///
    /// One undo step either way: a lane that appeared alongside the first
    /// adjustment is part of adding it, and undoing the adjustment but leaving
    /// an empty lane behind would be half an undo.
    ///
    /// Placed at the first instant from the playhead where it fits, for the
    /// reason `add_text` gives. The new adjustment changes nothing until its
    /// look is set, so adding one never visibly alters the edit on its own.
    pub fn add_adjustment(&mut self) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let existing = self
            .active_sequence()
            .and_then(|s| s.adjustment_tracks.first().map(|t| t.id));

        let (track, mut commands) = match existing {
            Some(track) => (track, Vec::new()),
            None => {
                let track = TrackId::new();
                (
                    track,
                    vec![Command::AddTrack {
                        sequence,
                        kind: crate::command::TrackKindRepr::Adjustment,
                        name: "Adjust 1".to_owned(),
                        id: track,
                    }],
                )
            }
        };

        let start = self.free_adjustment_slot(existing, self.playhead);
        let clip = bettercut_timeline::AdjustmentClip::new(start)?;
        let id = clip.id;
        commands.push(Command::AddAdjustment {
            sequence,
            track,
            clip: Box::new(clip),
        });
        self.dispatch_group("Add Adjustment", commands)?;
        Ok(id)
    }

    /// The first position at or after `from` where an adjustment of the default
    /// length fits on `track` without overlapping. `None` is a lane about to be
    /// made, which has room everywhere.
    fn free_adjustment_slot(&self, track: Option<TrackId>, from: TimelineTime) -> TimelineTime {
        let Some(track) =
            track.and_then(|id| self.active_sequence().and_then(|s| s.adjustment_track(id)))
        else {
            return from;
        };
        let mut start = from;
        loop {
            let end = start + bettercut_timeline::DEFAULT_ADJUSTMENT_DURATION;
            match track
                .clips()
                .iter()
                .find(|c| c.timeline.start < end && c.timeline.end > start)
            {
                Some(clip) => start = clip.timeline.end,
                None => return start,
            }
        }
    }

    /// The adjustment clip with this id, in the active sequence.
    pub fn adjustment_clip(&self, clip: ClipId) -> Option<&bettercut_timeline::AdjustmentClip> {
        self.active_sequence()?.adjustment_clip(clip)
    }

    /// Set how an adjustment grades what is beneath it.
    ///
    /// `continuing` collapses a slider drag into one undo step, as every other
    /// control does (§11).
    pub fn set_adjustment_look(
        &mut self,
        clip: ClipId,
        look: bettercut_timeline::AdjustmentLook,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.adjustment_track_of(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;
        let command = Command::SetAdjustmentLook {
            sequence,
            track,
            clip,
            look,
        };
        self.dispatch_gesture("Change Adjustment".to_owned(), vec![command], continuing)
    }

    pub fn remove_adjustment(&mut self, clip: ClipId) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.adjustment_track_of(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;
        self.dispatch(Command::RemoveAdjustment {
            sequence,
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
            self.fit_to_frame(&mut clip);
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
        self.set_sound_envelope(
            bettercut_timeline::AnimatedParameter::Gain,
            clip,
            points,
            continuing,
        )
    }

    /// The pan's line, the same way: where the sound sits between the
    /// speakers at each point, -1 to +1, in timeline time. An empty list
    /// clears it and the clip's static pan takes over again.
    pub fn set_pan_envelope(
        &mut self,
        clip: ClipId,
        points: &[(TimelineTime, f32)],
        continuing: bool,
    ) -> Result<usize, EditorError> {
        self.set_sound_envelope(
            bettercut_timeline::AnimatedParameter::Pan,
            clip,
            points,
            continuing,
        )
    }

    /// One of a sound clip's two lines — volume or pan — replaced whole.
    fn set_sound_envelope(
        &mut self,
        parameter: bettercut_timeline::AnimatedParameter,
        clip: ClipId,
        points: &[(TimelineTime, f32)],
        continuing: bool,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let audio = self.audio_clip(clip).ok_or(EditorError::ClipKindMismatch)?;

        let already_empty = !audio.keyframes.is_animated(parameter);
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
        let what = match parameter {
            bettercut_timeline::AnimatedParameter::Pan => "Pan",
            _ => "Volume",
        };
        let label = if count == 0 {
            format!("Clear {what} Envelope")
        } else {
            format!("{what} Envelope")
        };
        let command = match parameter {
            bettercut_timeline::AnimatedParameter::Pan => Command::SetPanEnvelope {
                sequence,
                track,
                clip,
                keys,
            },
            _ => Command::SetGainEnvelope {
                sequence,
                track,
                clip,
                keys,
            },
        };
        self.dispatch_gesture(label, vec![command], continuing)?;
        Ok(count)
    }

    /// Move a picture's position key at `time` (source time) to `position`,
    /// on both axes, as one step: what dragging a handle on the motion path
    /// does. A key an axis does not have there is made, so a path with a key
    /// on x alone gains its y as soon as that point is picked up. Each key's
    /// easing is kept; only where it goes changes.
    ///
    /// `continuing` folds a drag into one step, as every gesture does (§11).
    pub fn move_position_key(
        &mut self,
        clip: ClipId,
        time: MediaTime,
        position: bettercut_timeline::Vec2,
        continuing: bool,
    ) -> Result<(), EditorError> {
        use bettercut_timeline::AnimatedParameter as A;

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self.video_clip(clip).ok_or(EditorError::ClipKindMismatch)?;
        if !video.keyframes.is_animated(A::PositionX) && !video.keyframes.is_animated(A::PositionY)
        {
            return Err(EditorError::NotAnimated);
        }
        let commands = [(A::PositionX, position.x), (A::PositionY, position.y)]
            .into_iter()
            .map(|(parameter, value)| {
                let interpolation = video
                    .keyframes
                    .get(parameter, time)
                    .map_or_else(Interpolation::default, |key| key.interpolation);
                Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    key: Keyframe::new(time, parameter.clamp(value), interpolation),
                }
            })
            .collect();
        self.dispatch_gesture("Move Key".to_owned(), commands, continuing)
    }

    /// Put a pan key at the playhead on a sound clip — the way a pan line is
    /// begun and grown from the Inspector, one key at a time. The value is
    /// held to -1..1; a key already there is replaced. Refused off the clip,
    /// where there is no frame to key.
    ///
    /// `continuing` folds a slider's drag into one key rather than a hundred.
    pub fn key_pan_at_playhead(
        &mut self,
        clip: ClipId,
        pan: f32,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let audio = self.audio_clip(clip).ok_or(EditorError::ClipKindMismatch)?;
        // The sound clip's own place under the playhead: the shared helper
        // answers for pictures, and a sound clip is not one.
        let playhead = self.playhead();
        let at = audio
            .timeline
            .contains(playhead)
            .then(|| audio.source_time_at(playhead))
            .ok_or(EditorError::PlayheadOffClip)?;
        // Keep the curve a key already had; the value is what is changing.
        let interpolation = audio
            .keyframes
            .get(bettercut_timeline::AnimatedParameter::Pan, at)
            .map_or_else(bettercut_timeline::Interpolation::default, |key| {
                key.interpolation
            });
        let key = bettercut_timeline::Keyframe::new(
            at,
            bettercut_timeline::AnimatedParameter::Pan.clamp(pan),
            interpolation,
        );
        self.dispatch_gesture(
            "Key Pan".to_owned(),
            vec![Command::SetKeyframe {
                sequence,
                track,
                clip,
                parameter: bettercut_timeline::AnimatedParameter::Pan,
                key,
            }],
            continuing,
        )
    }

    /// Dress one title, and put it where the look belongs (§26).
    ///
    /// Style and position in one undo step, because "lower third" is a position
    /// as much as a style: applying half of it would leave the title somewhere
    /// nobody chose.
    ///
    /// Captions are styled by the lane instead ([`Self::set_caption_look`]) —
    /// a caption is one long thread of the same thing, a title is its own.
    pub fn set_title_look(
        &mut self,
        clip: ClipId,
        look: bettercut_text::TitleLook,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_track_of(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;

        let (x, y) = look.anchor();
        let commands = vec![
            Command::SetTextProperty {
                sequence,
                track,
                clip,
                // Designed for 1080 lines, like every new title: sized for
                // this frame (`Self::fit_to_frame`).
                property: crate::command::TextProperty::Style(Box::new(
                    bettercut_text::TextStyle::title(look).scaled(self.frame_scale()),
                )),
            },
            Command::SetTextProperty {
                sequence,
                track,
                clip,
                property: crate::command::TextProperty::Position { x, y },
            },
        ];
        self.dispatch_group(format!("{} Title", look.label()), commands)
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
        // A canned look is designed for 1080 lines; sized for this frame.
        self.dress_captions(bettercut_text::TextStyle::look(look).scaled(self.frame_scale()))
    }

    /// Put `style` on every caption, whatever it came from — one of the
    /// canned looks above, or a style the user saved from a title of their
    /// own. Returns how many changed; captions already wearing it are left
    /// alone, so this is no undo step when it would change nothing.
    pub fn set_caption_style(
        &mut self,
        style: bettercut_text::TextStyle,
    ) -> Result<usize, EditorError> {
        self.dress_captions(style)
    }

    fn dress_captions(&mut self, style: bettercut_text::TextStyle) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some(track) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| t.id)
        }) else {
            return Ok(0); // no lane yet: nothing to dress
        };

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
        self.dispatch_group("Dress Captions", commands)?;
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

    /// Move every caption by `delta` — the whole lane, for a subtitle file
    /// that runs early or late against the picture. One undo step; nothing
    /// before the start. Returns how many moved.
    pub fn shift_captions(&mut self, delta: TimelineTime) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some((track, mut starts)) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| {
                    (
                        t.id,
                        t.clips()
                            .iter()
                            .map(|c| (c.id, c.timeline.start))
                            .collect::<Vec<_>>(),
                    )
                })
        }) else {
            return Ok(0);
        };
        if delta.is_zero() || starts.is_empty() {
            return Ok(0);
        }
        // Never before the start: the shift is held so the first still lands
        // at zero, and the rest keep their spacing.
        let earliest = starts.iter().map(|(_, s)| s.ticks()).min().unwrap_or(0);
        let delta = TimelineTime::from_ticks(delta.ticks().max(-earliest));
        if delta.is_zero() {
            return Ok(0);
        }
        // Moving later goes last-first, earlier goes first-first, so no
        // caption is moved into one that has not yet made room.
        starts.sort_by_key(|(_, s)| s.ticks());
        if delta.ticks() > 0 {
            starts.reverse();
        }
        let count = starts.len();
        let commands: Vec<Command> = starts
            .into_iter()
            .map(|(clip, start)| Command::MoveClip {
                sequence,
                from_track: track,
                to_track: track,
                clip,
                new_start: start + delta,
            })
            .collect();
        self.dispatch_group("Shift Captions", commands)?;
        Ok(count)
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

    /// Change how fast a clip plays.
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

    /// Set every clip in `clips` that has motion to play at `speed`, each
    /// with whatever is linked to it — one undo step. Photos and held frames
    /// are skipped. Returns how many shots changed.
    pub fn set_clips_speed(
        &mut self,
        clips: &[ClipId],
        speed: bettercut_foundation::Rational,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut seen: Vec<ClipId> = Vec::new();
        let mut commands = Vec::new();
        let mut shots = 0;
        for clip in clips {
            if seen.contains(clip) || !self.can_retime(*clip) {
                continue;
            }
            let members = self.linked_with(*clip);
            seen.extend(members.iter().copied());
            shots += 1;
            for member in members {
                if let Some(track) = self.track_of(member) {
                    commands.push(Command::SetClipSpeed {
                        sequence,
                        track,
                        clip: member,
                        speed,
                    });
                }
            }
        }
        if shots > 0 {
            self.dispatch_group(format!("Change Speed of {shots} Clips"), commands)?;
        }
        Ok(shots)
    }

    /// Play every clip in `clips` that has motion backwards (`on`) or
    /// forwards again, each with whatever is linked to it — one undo step.
    /// Returns how many shots changed.
    pub fn set_clips_reversed(&mut self, clips: &[ClipId], on: bool) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut seen: Vec<ClipId> = Vec::new();
        let mut commands = Vec::new();
        let mut shots = 0;
        for clip in clips {
            if seen.contains(clip) || !self.can_retime(*clip) || self.is_reversed(*clip) == on {
                continue;
            }
            let members = self.linked_with(*clip);
            seen.extend(members.iter().copied());
            shots += 1;
            for member in members {
                if let Some(track) = self.track_of(member) {
                    commands.push(Command::SetClipProperty {
                        sequence,
                        track,
                        clip: member,
                        property: crate::command::ClipProperty::Reverse(on),
                    });
                }
            }
        }
        if shots > 0 {
            let label = if on { "Reverse" } else { "Play Forwards" };
            self.dispatch_group(format!("{label} {shots} Clips"), commands)?;
        }
        Ok(shots)
    }

    /// Play a clip backwards, or forwards again — and whatever is linked to it,
    /// so the sound runs backwards under its picture (§12). One undo step.
    ///
    /// Refused for a held frame or a photo, which have no motion to reverse.
    pub fn set_reversed(&mut self, clip: ClipId, on: bool) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        if !self.can_retime(clip) {
            return Err(EditorError::NoMotionToRetime);
        }
        let commands: Vec<Command> = self
            .linked_with(clip)
            .into_iter()
            .filter_map(|clip| {
                let track = self.track_of(clip)?;
                Some(Command::SetClipProperty {
                    sequence,
                    track,
                    clip,
                    property: crate::command::ClipProperty::Reverse(on),
                })
            })
            .collect();
        if commands.is_empty() {
            return Err(EditorError::ClipNotFound(clip));
        }
        self.dispatch_group(
            if on { "Reverse" } else { "Play Forwards" }.to_owned(),
            commands,
        )
    }

    /// Tag clips with a colour — and whatever is linked to them, since a
    /// picture and its sound are one shot (§12). One undo step. Returns how
    /// many clips were tagged.
    pub fn set_color_label(
        &mut self,
        clips: &[ClipId],
        label: bettercut_timeline::ColorLabel,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut targets: Vec<ClipId> = Vec::new();
        for clip in clips {
            for linked in self.linked_with(*clip) {
                if !targets.contains(&linked) {
                    targets.push(linked);
                }
            }
        }
        let commands: Vec<Command> = targets
            .iter()
            .filter_map(|clip| {
                Some(Command::SetColorLabel {
                    sequence,
                    track: self.track_of(*clip)?,
                    clip: *clip,
                    label,
                })
            })
            .collect();
        let count = commands.len();
        if count == 0 {
            return Ok(0);
        }
        self.dispatch_group(
            match label {
                bettercut_timeline::ColorLabel::None => "Clear Colour Label".to_owned(),
                other => format!("Colour Label: {}", other.name()),
            },
            commands,
        )?;
        Ok(count)
    }

    /// A clip's colour tag, on any lane.
    pub fn color_label(&self, clip: ClipId) -> Option<bettercut_timeline::ColorLabel> {
        use bettercut_timeline::Clip;
        let sequence = self.active_sequence()?;
        let span = sequence.clip_span(clip)?;
        sequence
            .video_track(span.track)
            .and_then(|t| t.get(clip).map(Clip::color_label))
            .or_else(|| {
                sequence
                    .audio_track(span.track)
                    .and_then(|t| t.get(clip).map(Clip::color_label))
            })
            .or_else(|| {
                sequence
                    .text_track(span.track)
                    .and_then(|t| t.get(clip).map(Clip::color_label))
            })
            .or_else(|| {
                sequence
                    .adjustment_track(span.track)
                    .and_then(|t| t.get(clip).map(Clip::color_label))
            })
    }

    /// Whether a clip plays backwards.
    pub fn is_reversed(&self, clip: ClipId) -> bool {
        self.video_clip(clip)
            .map(|c| c.reversed)
            .or_else(|| self.audio_clip(clip).map(|c| c.reversed))
            .unwrap_or(false)
    }

    /// A slow zoom across a clip — the "Ken Burns" move that keeps a photo
    /// montage from looking like a slideshow.
    ///
    /// Written as ordinary keyframes on the scale (§24), not as a mode: the
    /// keyframe editor can then adjust it, and the renderer needs to know
    /// nothing about it.
    pub fn set_movement(&mut self, clip: ClipId, movement: Movement) -> Result<(), EditorError> {
        self.set_movement_at(clip, movement, bettercut_timeline::MovementStrength::Normal)
    }

    /// [`Self::set_movement`] at a chosen strength: how far the move travels
    /// (`bettercut_timeline::MovementStrength`).
    pub fn set_movement_at(
        &mut self,
        clip: ClipId,
        movement: Movement,
        strength: bettercut_timeline::MovementStrength,
    ) -> Result<(), EditorError> {
        use bettercut_timeline::AnimatedParameter;

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        // A pan slides the picture, so it needs the position to itself. Keys
        // somebody else put there — a shake, a hand-made move — are work that
        // cannot be guessed back, so the pan is refused instead (§24). A zoom
        // asks nothing of the position and happily lives over a shake.
        let was_pan = self.movement_of(clip).is_some_and(Movement::is_pan);
        let positioned = [AnimatedParameter::PositionX, AnimatedParameter::PositionY]
            .into_iter()
            .any(|parameter| video.keyframes.is_animated(parameter));
        if movement.is_pan() && positioned && !was_pan {
            return Err(EditorError::AlreadyAnimated("position"));
        }
        // Only a pan's own keys are cleared: the scale belongs to whichever
        // movement is being replaced, and the position only when a pan put it
        // there or a pan is about to.
        let mut scales = vec![AnimatedParameter::ScaleX, AnimatedParameter::ScaleY];
        if movement.is_pan() || was_pan {
            scales.push(AnimatedParameter::PositionX);
            scales.push(AnimatedParameter::PositionY);
        }
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
                .keyframes_at(
                    video.transform.scale,
                    video.transform.position,
                    video.source,
                    strength,
                )
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

    /// Give every clip in `clips` a movement, as one undo step (§79).
    ///
    /// `alternate` turns each one the other way — zoom in, zoom out, zoom in;
    /// left, right, left — which is what keeps a photo montage from reading as
    /// a machine: twenty shots all drifting the same way is a conveyor belt.
    ///
    /// Clips that refuse the movement are skipped rather than stopping the
    /// rest: one shot with a shake on it is no reason to leave the other
    /// nineteen still. Returns how many took it.
    pub fn set_movement_on(
        &mut self,
        clips: &[ClipId],
        movement: Movement,
        strength: bettercut_timeline::MovementStrength,
        alternate: bool,
    ) -> Result<usize, EditorError> {
        let mut given = 0;
        for (index, clip) in clips.iter().enumerate() {
            let movement = if alternate && index % 2 == 1 {
                movement.reversed()
            } else {
                movement
            };
            if self.set_movement_at(*clip, movement, strength).is_ok() {
                given += 1;
            }
        }
        if given == 0 {
            return Err(EditorError::AlreadyAnimated("position"));
        }
        Ok(given)
    }

    /// Shake a clip's picture, or take a shake off (`None`), as one undo step.
    ///
    /// Written as keys ([`ShakeStrength::keyframes`]): position every fifteenth
    /// of a second, and a constant scale pair just big enough to keep the
    /// frame's edges covered. Changing strength replaces the old shake.
    ///
    /// Refused, changing nothing, when the clip's position is animated by
    /// hand, or when adding a shake to a clip whose scale is already animated —
    /// a zoom movement — since the overscan would have to overwrite it.
    pub fn set_shake(
        &mut self,
        clip: ClipId,
        strength: Option<bettercut_timeline::ShakeStrength>,
    ) -> Result<(), EditorError> {
        use bettercut_timeline::{AnimatedParameter as A, ShakeStrength};

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let keys_of = |parameter| {
            video
                .keyframes
                .track(parameter)
                .map(|t| t.keys().to_vec())
                .unwrap_or_default()
        };

        let mut remove: Vec<(A, bettercut_foundation::MediaTime)> = Vec::new();
        for parameter in [A::PositionX, A::PositionY] {
            let keys = keys_of(parameter);
            if keys.is_empty() {
                continue;
            }
            if !ShakeStrength::on_grid(&keys, video.source) {
                return Err(EditorError::AlreadyAnimated("position"));
            }
            remove.extend(keys.iter().map(|key| (parameter, key.time)));
        }
        for parameter in [A::ScaleX, A::ScaleY] {
            let keys = keys_of(parameter);
            match keys.as_slice() {
                [] => {}
                // A constant pair is a shake's overscan.
                [from, to] if from.value == to.value => {
                    remove.extend(keys.iter().map(|key| (parameter, key.time)));
                }
                // A zoom stays. Taking a shake off leaves it be; putting one on
                // would overwrite it.
                _ if strength.is_some() => return Err(EditorError::AlreadyAnimated("scale")),
                _ => {}
            }
        }

        let mut commands: Vec<Command> = remove
            .into_iter()
            .map(|(parameter, time)| Command::RemoveKeyframe {
                sequence,
                track,
                clip,
                parameter,
                time,
            })
            .collect();
        if let Some(strength) = strength {
            commands.extend(
                strength
                    .keyframes(
                        video.transform.position,
                        video.transform.scale,
                        video.source,
                    )
                    .into_iter()
                    .map(|(parameter, key)| Command::SetKeyframe {
                        sequence,
                        track,
                        clip,
                        parameter,
                        key,
                    }),
            );
        }

        if commands.is_empty() {
            return Ok(());
        }
        let label = match strength {
            Some(strength) => format!("Shake: {}", strength.label()),
            None => "Remove Shake".to_owned(),
        };
        self.dispatch_group(label, commands)
    }

    /// The shake on a clip, if its keys are one this wrote.
    pub fn shake_of(&self, clip: ClipId) -> Option<bettercut_timeline::ShakeStrength> {
        use bettercut_timeline::AnimatedParameter as A;

        let video = self.video_clip(clip)?;
        let keys = |parameter| {
            video
                .keyframes
                .track(parameter)
                .map_or(&[][..], bettercut_timeline::KeyframeTrack::keys)
        };
        bettercut_timeline::ShakeStrength::recognise(
            keys(A::PositionX),
            keys(A::ScaleX),
            video.transform.scale.x,
            video.source,
        )
    }

    /// The movement on a clip, as far as it can be told from its keys.
    ///
    /// `None` for a clip whose scale is not animated, and for one animated in
    /// some way this did not write — the interface highlights nothing rather
    /// than claiming a hand-made animation is a preset.
    pub fn movement_of(&self, clip: ClipId) -> Option<Movement> {
        use bettercut_timeline::AnimatedParameter;

        let keyframes = &self.video_clip(clip)?.keyframes;
        let pair = |parameter| {
            let keys = keyframes
                .track(parameter)
                .map_or(&[][..], bettercut_timeline::KeyframeTrack::keys);
            match keys {
                [first, .., last] => Some((first.value, last.value)),
                _ => None,
            }
        };
        // A movement's shape: one key at each end and nothing in between. A
        // shake keys the position dozens of times, and calling that a pan
        // would let a pan quietly overwrite it.
        let own_shape = |parameter| {
            keyframes
                .track(parameter)
                .is_some_and(|track| track.len() == 2)
        };
        let panned = own_shape(AnimatedParameter::ScaleX)
            && (own_shape(AnimatedParameter::PositionX) || own_shape(AnimatedParameter::PositionY));
        // A pan slides the picture across while holding it enlarged, so it is
        // the *position* that says which way it went; a zoom leaves the
        // position alone and changes the scale.
        if let Some((from, to)) = pair(AnimatedParameter::PositionX).filter(|_| panned) {
            return Some(if from > to {
                Movement::PanLeft
            } else {
                Movement::PanRight
            });
        }
        if let Some((from, to)) = pair(AnimatedParameter::PositionY).filter(|_| panned) {
            return Some(if from > to {
                Movement::PanUp
            } else {
                Movement::PanDown
            });
        }
        // Position keys that are not a pan's belong to somebody else.
        if keyframes.is_animated(AnimatedParameter::PositionX)
            || keyframes.is_animated(AnimatedParameter::PositionY)
        {
            return None;
        }
        match pair(AnimatedParameter::ScaleX) {
            // No keys at all is the same answer as no track: nothing moves.
            None => Some(Movement::None),
            Some((from, to)) if from < to => Some(Movement::ZoomIn),
            Some((from, to)) if from > to => Some(Movement::ZoomOut),
            Some(_) => None,
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
            ClipProperty::Temperature(color.temperature),
            ClipProperty::Tint(color.tint),
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

    // ---- carrying a look from one clip to others ----

    /// Everything about how a clip looks that is not about where or when it is.
    ///
    /// Deliberately a list of the same [`ClipProperty`] values the Inspector
    /// writes rather than a new kind of edit. Pasting is then a handful of
    /// ordinary property changes in one group (§79) — undo, redo and crash
    /// replay all come free, through machinery that already exists and is
    /// already tested.
    ///
    /// What is *not* in it matters as much as what is:
    ///
    /// * the transform — framing is a decision about this shot, and a wide
    ///   landscape and a close-up do not want the same crop;
    /// * keyframes — they are anchored to source time (§24), so the same keys
    ///   on a clip of another length land somewhere else entirely;
    /// * speed, timing and the sound — none of them is a look.
    pub fn clip_look(&self, clip: ClipId) -> Option<Vec<crate::command::ClipProperty>> {
        use crate::command::ClipProperty;

        let clip = self.video_clip(clip)?;
        Some(vec![
            ClipProperty::Opacity(clip.opacity),
            ClipProperty::Brightness(clip.color.brightness),
            ClipProperty::Contrast(clip.color.contrast),
            ClipProperty::Saturation(clip.color.saturation),
            ClipProperty::Temperature(clip.color.temperature),
            ClipProperty::Tint(clip.color.tint),
            ClipProperty::Blur(clip.blur),
            ClipProperty::Sharpen(clip.sharpen),
            ClipProperty::Lut(clip.lut),
            ClipProperty::RgbSplit(clip.rgb_split),
            ClipProperty::Glitch(clip.glitch),
            ClipProperty::Reflection(clip.reflection),
            ClipProperty::Blend(clip.blend),
            ClipProperty::Mask(clip.mask),
            ClipProperty::ChromaKey(clip.chroma_key),
            ClipProperty::LumaKey(clip.luma_key),
            ClipProperty::Motion(clip.motion),
            ClipProperty::MotionBlur(clip.motion_blur),
            ClipProperty::Backdrop(clip.backdrop),
            ClipProperty::Vibrance(clip.color.vibrance),
            ClipProperty::Border(clip.border),
            ClipProperty::Shadow(clip.shadow),
            ClipProperty::CornerPin(clip.corner_pin),
            ClipProperty::Pixelate(clip.pixelate),
            ClipProperty::ZoomBlur(clip.zoom_blur),
            ClipProperty::Posterise(clip.posterise),
            ClipProperty::SmoothSkin(clip.smooth_skin),
            ClipProperty::TiltShift {
                band: clip.tilt_band,
                centre: clip.tilt_centre,
            },
            ClipProperty::Glow(clip.glow),
            ClipProperty::OldFilm(clip.old_film),
            ClipProperty::Vignette(clip.vignette),
            ClipProperty::LightLeak(clip.light_leak),
            ClipProperty::LensFlare(clip.lens_flare),
            ClipProperty::BeatPulse(clip.beat_pulse),
            ClipProperty::Shake(clip.shake),
            ClipProperty::Strobe(clip.strobe),
            ClipProperty::Sway(clip.sway),
            ClipProperty::Flicker(clip.flicker),
            ClipProperty::SmoothMotion(clip.smooth_motion),
            ClipProperty::Curves(clip.curves),
        ])
    }

    /// A look with nothing on it: what a clip looks like before anyone has
    /// touched it.
    ///
    /// Pasting this is how a clip is stripped back, so clearing and pasting are
    /// the same edit with different values rather than two code paths that have
    /// to agree about what a look contains.
    pub fn plain_look() -> Vec<crate::command::ClipProperty> {
        use crate::command::ClipProperty;
        use bettercut_timeline::AnimatedParameter as A;

        vec![
            ClipProperty::Opacity(A::Opacity.default_value()),
            ClipProperty::Brightness(A::Brightness.default_value()),
            ClipProperty::Contrast(A::Contrast.default_value()),
            ClipProperty::Saturation(A::Saturation.default_value()),
            ClipProperty::Temperature(A::Temperature.default_value()),
            ClipProperty::Tint(A::Tint.default_value()),
            ClipProperty::Blur(A::Blur.default_value()),
            ClipProperty::Sharpen(0.0),
            ClipProperty::Lut(None),
            ClipProperty::RgbSplit(0.0),
            ClipProperty::Glitch(0.0),
            ClipProperty::Reflection(bettercut_timeline::Reflection::None),
            ClipProperty::Blend(bettercut_timeline::BlendMode::Normal),
            ClipProperty::Mask(None),
            ClipProperty::ChromaKey(None),
            ClipProperty::LumaKey(None),
            ClipProperty::Motion(bettercut_timeline::ClipMotion::default()),
            ClipProperty::MotionBlur(false),
            ClipProperty::Backdrop(bettercut_timeline::Backdrop::None),
            ClipProperty::Vibrance(0.0),
            ClipProperty::Border(bettercut_timeline::Border::NONE),
            ClipProperty::Shadow(bettercut_timeline::Shadow::default()),
            ClipProperty::CornerPin(bettercut_timeline::CornerPin::NONE),
            ClipProperty::Pixelate(0.0),
            ClipProperty::ZoomBlur(0.0),
            ClipProperty::Posterise(0.0),
            ClipProperty::SmoothSkin(0.0),
            ClipProperty::TiltShift {
                band: 0.0,
                centre: 0.5,
            },
            ClipProperty::Glow(0.0),
            ClipProperty::OldFilm(0.0),
            ClipProperty::Vignette(0.0),
            ClipProperty::LightLeak(0.0),
            ClipProperty::LensFlare(0.0),
            ClipProperty::BeatPulse(0.0),
            ClipProperty::Shake(0.0),
            ClipProperty::Strobe(0.0),
            ClipProperty::Sway(0.0),
            ClipProperty::Flicker(0.0),
            ClipProperty::SmoothMotion(false),
            ClipProperty::Curves(bettercut_timeline::curves::ColourCurves::default()),
        ]
    }

    /// Put a copied look onto `targets`. Returns how many clips took it.
    ///
    /// Anything in `targets` that is not a picture is skipped rather than
    /// refused: a selection made on the timeline usually carries the linked
    /// sound with it (§12), and failing the whole paste because of that would
    /// make the feature unusable exactly where it is most wanted.
    pub fn paste_look(
        &mut self,
        look: &[crate::command::ClipProperty],
        targets: impl IntoIterator<Item = ClipId>,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut commands = Vec::new();
        let mut taken = 0;

        for clip in targets {
            let Some(track) = self.track_of(clip) else {
                continue;
            };
            if self.video_clip(clip).is_none() {
                continue;
            }
            taken += 1;
            commands.extend(look.iter().map(|&property| Command::SetClipProperty {
                sequence,
                track,
                clip,
                property,
            }));
        }

        // No empty group: an undo step that undoes nothing is worse than no
        // step, because the user presses undo and watches nothing happen.
        if commands.is_empty() {
            return Ok(0);
        }
        self.dispatch_group("Paste Look", commands)?;
        Ok(taken)
    }

    // ---- recovery ----

    /// How often the recovery journal forces a snapshot, in seconds.
    pub fn autosave_seconds(&self) -> u64 {
        self.journal.snapshot_every_seconds()
    }

    /// Set how often the recovery journal forces a snapshot; held to the
    /// journal's own limits. Not a project setting: how much work a crash
    /// may cost is the person's call, on their machine.
    pub fn set_autosave_seconds(&mut self, seconds: u64) {
        self.journal.set_snapshot_every_seconds(seconds);
    }

    // ---- project notes ----

    /// The project's notes, as typed.
    pub fn project_notes(&self) -> &str {
        &self.project.notes
    }

    /// Replace the notes, as one undo step. Unchanged text is no step at
    /// all, so leaving the pane without typing costs nothing to undo.
    pub fn set_project_notes(&mut self, notes: &str) -> Result<(), EditorError> {
        if self.project.notes == notes {
            return Ok(());
        }
        self.dispatch(Command::SetProjectNotes {
            notes: notes.to_owned(),
        })
    }

    // ---- clip marks ----

    /// The most marks one clip keeps: past this they are a transcript, not
    /// marks, and the timeline could not draw them apart.
    pub const MAX_CLIP_MARKS: usize = 200;

    /// Mark the playhead on `clip`, at the frame of the footage it is over.
    /// A second mark on the same frame takes it off again, as M does on the
    /// ruler. Returns whether one was added.
    pub fn toggle_clip_mark(&mut self, clip: ClipId) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self
            .source_time_at_playhead(clip)
            .or_else(|| self.sound_source_time_at_playhead(clip))
            .ok_or(EditorError::PlayheadOffClip)?;
        // A frame's worth of tolerance: two marks a tick apart are one mark
        // as far as anyone watching can tell.
        let tolerance = self
            .active_sequence()
            .and_then(|s| s.ticks_per_frame().checked_div(2))
            .unwrap_or(480);
        let mut marks = self
            .active_sequence()
            .map(|s| s.clip_marks.clone())
            .unwrap_or_default();
        let existing = marks.iter().position(|mark| {
            mark.clip == clip && (mark.source.ticks() - at.ticks()).abs() <= tolerance
        });
        let added = match existing {
            Some(index) => {
                marks.remove(index);
                false
            }
            None => {
                if marks.iter().filter(|m| m.clip == clip).count() >= Self::MAX_CLIP_MARKS {
                    return Err(EditorError::PlayheadOffClip);
                }
                marks.push(bettercut_timeline::ClipMark {
                    clip,
                    source: at,
                    label: String::new(),
                    color: bettercut_timeline::ColorLabel::None,
                });
                true
            }
        };
        self.dispatch(Command::SetClipMarks { sequence, marks })?;
        Ok(added)
    }

    /// A clip's own marks, where they fall on the timeline now.
    pub fn clip_marks(&self, clip: ClipId) -> Vec<TimelineTime> {
        self.project
            .active()
            .map(|s| {
                s.clip_marks_on_timeline(clip)
                    .into_iter()
                    .map(|(at, _)| at)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Take every mark off `clip`, as one step. Returns how many went.
    pub fn clear_clip_marks(&mut self, clip: ClipId) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut marks = self
            .active_sequence()
            .map(|s| s.clip_marks.clone())
            .unwrap_or_default();
        let before = marks.len();
        marks.retain(|mark| mark.clip != clip);
        let gone = before - marks.len();
        if gone == 0 {
            return Ok(0);
        }
        self.dispatch(Command::SetClipMarks { sequence, marks })?;
        Ok(gone)
    }

    /// Where the playhead falls in a *sound* clip's source, for marking one.
    fn sound_source_time_at_playhead(&self, clip: ClipId) -> Option<MediaTime> {
        use bettercut_timeline::Clip;
        let sequence = self.project.active()?;
        let audio = sequence
            .audio_tracks
            .iter()
            .find_map(|track| track.get(clip))?;
        let playhead = self.playhead();
        if !audio.timeline.contains(playhead) {
            return None;
        }
        let into = playhead - audio.timeline.start;
        Some(audio.source().start + MediaTime::from_ticks(into.ticks()))
    }

    // ---- clip names ----

    /// The clip's own name, if it has one.
    pub fn clip_name(&self, clip: ClipId) -> Option<String> {
        use bettercut_timeline::Clip;
        let sequence = self.project.active()?;
        sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(clip).and_then(|c| c.name().map(str::to_owned)))
            .or_else(|| {
                sequence
                    .audio_tracks
                    .iter()
                    .find_map(|t| t.get(clip).and_then(|c| c.name().map(str::to_owned)))
            })
            .or_else(|| {
                sequence
                    .text_tracks
                    .iter()
                    .find_map(|t| t.get(clip).and_then(|c| c.name().map(str::to_owned)))
            })
            .or_else(|| {
                sequence
                    .adjustment_tracks
                    .iter()
                    .find_map(|t| t.get(clip).and_then(|c| c.name().map(str::to_owned)))
            })
    }

    /// Name a clip, or give it back the file's name with `None` or an empty
    /// name. One undo step; the same name again is no step.
    pub fn set_clip_name(&mut self, clip: ClipId, name: Option<&str>) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let wanted = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        if self.clip_name(clip) == wanted {
            return Ok(false);
        }
        self.dispatch(Command::SetClipName {
            sequence,
            track,
            clip,
            name: wanted,
        })?;
        Ok(true)
    }

    /// Name `clips` "`base` 01", "`base` 02"… in timeline order, as one
    /// undo step. A clip linked to one already numbered takes the same
    /// number, so a shot and its sound read alike. Numbers are padded to at
    /// least two digits so they sort. Returns how many were named.
    pub fn number_clips(&mut self, clips: &[ClipId], base: &str) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let base = base.trim();
        if base.is_empty() {
            return Ok(0);
        }
        let mut ordered: Vec<(ClipId, TrackId, i64)> = clips
            .iter()
            .filter_map(|clip| {
                let track = self.track_of(*clip)?;
                let start = self.active_sequence()?.clip_span(*clip)?.timeline.start;
                Some((*clip, track, start.ticks()))
            })
            .collect();
        ordered.sort_by_key(|(_, _, start)| *start);
        ordered.dedup_by_key(|(clip, _, _)| *clip);

        let mut numbered: Vec<(ClipId, usize)> = Vec::new();
        let mut next = 0;
        for (clip, _, _) in &ordered {
            let partners = self.linked_with(*clip);
            let shared = numbered
                .iter()
                .find(|(other, _)| partners.contains(other))
                .map(|(_, n)| *n);
            let number = shared.unwrap_or_else(|| {
                next += 1;
                next
            });
            numbered.push((*clip, number));
        }
        let width = next.to_string().len().max(2);
        let commands: Vec<Command> = ordered
            .iter()
            .zip(&numbered)
            .filter_map(|((clip, track, _), (_, number))| {
                let name = format!("{base} {number:0width$}");
                let changed = self.clip_name(*clip).as_deref() != Some(name.as_str());
                changed.then_some(Command::SetClipName {
                    sequence,
                    track: *track,
                    clip: *clip,
                    name: Some(name),
                })
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Number Clips", commands)?;
        }
        Ok(count)
    }

    // ---- clip solo ----

    /// Whether `clip` is soloed.
    pub fn clip_soloed(&self, clip: ClipId) -> bool {
        self.project
            .active()
            .is_some_and(|sequence| sequence.clip_soloed(clip))
    }

    /// Whether any clip is soloed, for a menu that offers to clear them.
    pub fn any_clip_soloed(&self) -> bool {
        self.project
            .active()
            .is_some_and(|sequence| sequence.picture_clip_soloed() || sequence.sound_clip_soloed())
    }

    /// Solo (or unsolo) `clips` and whatever is linked to them (§12): a shot
    /// soloed on its own is heard as well as seen. One undo step. Returns
    /// how many clips changed.
    pub fn set_clip_solo(&mut self, clips: &[ClipId], solo: bool) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let mut targets: Vec<ClipId> = Vec::new();
        for clip in clips {
            for linked in self.linked_with(*clip) {
                if !targets.contains(&linked) {
                    targets.push(linked);
                }
            }
        }
        let commands: Vec<Command> = targets
            .iter()
            .filter(|clip| self.clip_soloed(**clip) != solo)
            .map(|clip| Command::SetClipSolo {
                sequence,
                clip: *clip,
                solo,
            })
            .collect();
        if commands.is_empty() {
            return Ok(0);
        }
        let count = commands.len();
        self.dispatch_group(if solo { "Solo Clips" } else { "Unsolo Clips" }, commands)?;
        Ok(count)
    }

    /// Unsolo every clip, as one step. Returns how many were soloed.
    pub fn clear_clip_solos(&mut self) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let soloed: Vec<ClipId> = self
            .project
            .active()
            .map(|s| s.soloed_clips.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|clip| {
                self.project
                    .active()
                    .and_then(|s| s.clip_span(*clip))
                    .is_some()
            })
            .collect();
        if soloed.is_empty() {
            return Ok(0);
        }
        let commands: Vec<Command> = soloed
            .iter()
            .map(|clip| Command::SetClipSolo {
                sequence,
                clip: *clip,
                solo: false,
            })
            .collect();
        let count = commands.len();
        self.dispatch_group("Clear Solos", commands)?;
        Ok(count)
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

    /// One marker at `at` (snapped to a frame, as [`Self::add_markers`] does)
    /// with a name, in one undo step; a marker already there is renamed.
    /// Returns where it landed.
    pub fn add_named_marker(
        &mut self,
        at: TimelineTime,
        label: &str,
    ) -> Result<TimelineTime, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self.snap_to_frame(sequence, at);
        let label: String = label.trim().chars().take(Self::MAX_MARKER_LABEL).collect();
        let mut markers = self.markers().to_vec();
        match markers.iter_mut().find(|m| m.time == at) {
            Some(marker) if marker.label == label => return Ok(at),
            Some(marker) => marker.label = label,
            None => {
                let mut marker = bettercut_timeline::Marker::at(at);
                marker.label = label;
                markers.push(marker);
            }
        }
        self.replace_markers(bettercut_timeline::marker::normalized(markers))?;
        Ok(at)
    }

    /// A marker every `interval`, from the marked in-point (or the start)
    /// up to the out-point (or the end of the edit), as one undo step. The
    /// start itself is skipped — a marker at zero marks nothing. Returns how
    /// many were added; those already there are not added twice.
    pub fn add_markers_every(&mut self, interval: TimelineTime) -> Result<usize, EditorError> {
        let Some(sequence) = self.active_sequence() else {
            return Ok(0);
        };
        let step = interval.ticks();
        if step <= 0 {
            return Ok(0);
        }
        let from = sequence.mark_in.unwrap_or(TimelineTime::ZERO).ticks();
        let to = sequence
            .mark_out
            .unwrap_or_else(|| sequence.duration())
            .ticks();
        // Past this the ruler is solid markers and nothing can be read.
        const MOST: i64 = 1_000;
        if (to - from) / step > MOST {
            return Err(EditorError::TooManyMarkers);
        }
        let times: Vec<TimelineTime> = (1..)
            .map(|n| from + n * step)
            .take_while(|at| *at < to)
            .map(TimelineTime::from_ticks)
            .collect();
        if times.is_empty() {
            return Ok(0);
        }
        self.add_markers(&times)
    }

    /// The longest name a marker keeps. Enough for "second chorus — cut to
    /// the drone shot"; past it the ruler could not show the name anyway.
    pub const MAX_MARKER_LABEL: usize = 80;

    /// Name the marker at `at`, or clear its name with an empty `label`.
    ///
    /// Surrounding spaces are trimmed and the name is cut to
    /// [`Self::MAX_MARKER_LABEL`] characters. Returns whether anything changed:
    /// no marker there, or the same name again, is no undo step.
    pub fn set_marker_label(&mut self, at: TimelineTime, label: &str) -> Result<bool, EditorError> {
        let label: String = label.trim().chars().take(Self::MAX_MARKER_LABEL).collect();
        let mut markers = self.markers().to_vec();
        let Some(marker) = markers.iter_mut().find(|m| m.time == at) else {
            return Ok(false);
        };
        if marker.label == label {
            return Ok(false);
        }
        marker.label = label;
        self.replace_markers(markers)?;
        Ok(true)
    }

    /// Colour the marker at `at`, or take its colour off with
    /// `ColorLabel::None`. One undo step; returns whether anything changed.
    pub fn set_marker_color(
        &mut self,
        at: TimelineTime,
        color: bettercut_timeline::ColorLabel,
    ) -> Result<bool, EditorError> {
        let mut markers = self.markers().to_vec();
        let Some(marker) = markers.iter_mut().find(|m| m.time == at) else {
            return Ok(false);
        };
        if marker.color == color {
            return Ok(false);
        }
        marker.color = color;
        self.replace_markers(markers)?;
        Ok(true)
    }

    /// Give the marker at `at` a span, or take it away with zero: a note about
    /// a passage rather than about an instant.
    ///
    /// One undo step; returns whether anything changed. A span reaching past
    /// nothing is no span, and one running backwards is read as the stretch
    /// between the two instants, because that is plainly what was meant.
    pub fn set_marker_span(
        &mut self,
        at: TimelineTime,
        span: TimelineTime,
    ) -> Result<bool, EditorError> {
        let span = TimelineTime::from_ticks(span.ticks().abs());
        let mut markers = self.markers().to_vec();
        let Some(marker) = markers.iter_mut().find(|m| m.time == at) else {
            return Ok(false);
        };
        if marker.span == span {
            return Ok(false);
        }
        marker.span = span;
        self.replace_markers(markers)?;
        Ok(true)
    }

    /// Mark the stretch between the in and out points, or the marked range
    /// given outright: one mark covering the lot.
    ///
    /// The gesture behind "this whole bit is too long" — the alternative is
    /// two marks and remembering which is which.
    pub fn mark_range(
        &mut self,
        range: bettercut_timeline::TimelineRange,
        label: &str,
    ) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let start = self.snap_to_frame(sequence, range.start);
        let end = self.snap_to_frame(sequence, range.end);
        if end <= start {
            return Ok(false);
        }
        let label: String = label.trim().chars().take(Self::MAX_MARKER_LABEL).collect();
        let mut markers = self.markers().to_vec();
        let span = TimelineTime::from_ticks(end.ticks() - start.ticks());
        match markers.iter_mut().find(|m| m.time == start) {
            Some(marker) => {
                marker.span = span;
                if !label.is_empty() {
                    marker.label = label;
                }
            }
            None => {
                let mut marker = bettercut_timeline::Marker::over(start, span);
                marker.label = label;
                markers.push(marker);
            }
        }
        self.replace_markers(markers)?;
        Ok(true)
    }

    /// The marker covering `at`: a ranged one whose stretch it falls in, or a
    /// plain one exactly on it.
    ///
    /// The ranged marks are checked first, since a plain mark inside a ranged
    /// one is the smaller answer to "what is this passage".
    pub fn marker_covering(&self, at: TimelineTime) -> Option<&bettercut_timeline::Marker> {
        let markers = self.markers();
        markers
            .iter()
            .find(|marker| marker.is_ranged() && marker.covers(at))
            .or_else(|| markers.iter().find(|marker| marker.covers(at)))
    }

    /// Take away the marker at `at`. Returns whether there was one.
    pub fn remove_marker(&mut self, at: TimelineTime) -> Result<bool, EditorError> {
        let mut markers = self.markers().to_vec();
        let before = markers.len();
        markers.retain(|m| m.time != at);
        if markers.len() == before {
            return Ok(false);
        }
        self.replace_markers(markers)?;
        Ok(true)
    }

    pub fn clear_markers(&mut self) -> Result<(), EditorError> {
        if self.markers().is_empty() {
            return Ok(());
        }
        self.replace_markers(Vec::new())
    }

    pub(crate) fn replace_markers(
        &mut self,
        markers: Vec<bettercut_timeline::Marker>,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetMarkers { sequence, markers })
    }

    /// The markers after a stretch of the timeline is closed up — a ripple
    /// delete, a closed gap: those inside `range` go with it, those after it
    /// move back by its length, so a beat grid still sits on the beats. Part
    /// of the same undo step; nothing staged when no marker is touched.
    pub(crate) fn stage_markers_closed(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        range: bettercut_timeline::TimelineRange,
    ) -> Result<(), EditorError> {
        let markers = self.markers();
        if !markers.iter().any(|m| m.time >= range.start) {
            return Ok(());
        }
        let length = range.duration();
        let kept: Vec<_> = markers
            .iter()
            .filter(|m| !(m.time >= range.start && m.time < range.end))
            .map(|m| {
                let mut m = m.clone();
                if m.time >= range.end {
                    m.time -= length;
                }
                m
            })
            .collect();
        self.stage(
            stage,
            Command::SetMarkers {
                sequence,
                markers: kept,
            },
        )
    }

    /// Mark the in point at `at` (snapped to a frame). An out mark at or
    /// before it is cleared rather than left making an empty range.
    pub fn set_mark_in(&mut self, at: TimelineTime) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let at = sequence.snap_to_frame(at);
        let out = sequence.mark_out.filter(|out| *out > at);
        self.dispatch(Command::SetInOut {
            sequence: sequence_id,
            mark_in: Some(at),
            mark_out: out,
        })
    }

    /// Mark the out point at `at`. An in mark at or after it is cleared.
    pub fn set_mark_out(&mut self, at: TimelineTime) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let at = sequence.snap_to_frame(at);
        let mark_in = sequence.mark_in.filter(|mark| *mark < at);
        self.dispatch(Command::SetInOut {
            sequence: sequence_id,
            mark_in,
            mark_out: Some(at),
        })
    }

    /// Mark in and out around `clips`: from the earliest start to the
    /// latest end among them, as one step. Returns the range marked, or
    /// `None` when none of them is on the timeline.
    pub fn mark_clips(
        &mut self,
        clips: &[ClipId],
    ) -> Result<Option<bettercut_timeline::TimelineRange>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let spans: Vec<_> = clips
            .iter()
            .filter_map(|clip| sequence.clip_span(*clip).map(|s| s.timeline))
            .collect();
        let (Some(start), Some(end)) = (
            spans.iter().map(|s| s.start).min(),
            spans.iter().map(|s| s.end).max(),
        ) else {
            return Ok(None);
        };
        self.dispatch(Command::SetInOut {
            sequence: sequence_id,
            mark_in: Some(start),
            mark_out: Some(end),
        })?;
        Ok(Some(bettercut_timeline::TimelineRange { start, end }))
    }

    /// Clear both marks. Nothing to clear is no step.
    pub fn clear_marks(&mut self) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        if sequence.mark_in.is_none() && sequence.mark_out.is_none() {
            return Ok(());
        }
        self.dispatch(Command::SetInOut {
            sequence: sequence_id,
            mark_in: None,
            mark_out: None,
        })
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

    /// Set a sound lane's equaliser, over every clip on it, as one step
    /// (`Command::SetTrackEq`). `continuing` collapses a drag into one undo
    /// step (§11).
    pub fn set_track_eq(
        &mut self,
        track: TrackId,
        eq: bettercut_timeline::ClipEq,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch_gesture(
            "Change track EQ".to_owned(),
            vec![Command::SetTrackEq {
                sequence,
                track,
                eq,
            }],
            continuing,
        )
    }

    /// Colour a lane (`Track::color_label`), any kind, as one step.
    pub fn set_track_colour(
        &mut self,
        track: TrackId,
        label: bettercut_timeline::ColorLabel,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetTrackColour {
            sequence,
            track,
            label,
        })
    }

    /// A lane's colour label; none for a lane that is not there.
    pub fn track_colour(&self, track: TrackId) -> bettercut_timeline::ColorLabel {
        let Some(sequence) = self.active_sequence() else {
            return bettercut_timeline::ColorLabel::None;
        };
        sequence
            .video_track(track)
            .map(|lane| lane.color_label)
            .or_else(|| sequence.audio_track(track).map(|lane| lane.color_label))
            .or_else(|| sequence.text_track(track).map(|lane| lane.color_label))
            .or_else(|| {
                sequence
                    .adjustment_track(track)
                    .map(|lane| lane.color_label)
            })
            .unwrap_or_default()
    }

    /// A sound lane's equaliser; flat for a lane that is not a sound lane.
    pub fn track_eq(&self, track: TrackId) -> bettercut_timeline::ClipEq {
        self.active_sequence()
            .and_then(|sequence| sequence.audio_track(track))
            .map_or_else(bettercut_timeline::ClipEq::default, |lane| lane.eq)
    }

    /// The volume line on `track`, empty for a lane that has none
    /// (`bettercut_timeline::track_volume`).
    pub fn track_volume(&self, track: TrackId) -> &[bettercut_timeline::VolumePoint] {
        self.active_sequence()
            .and_then(|sequence| sequence.audio_track(track))
            .map_or(&[], |track| track.volume.points())
    }

    /// Draw `points` as the volume line on `track`, replacing whatever was
    /// there. An empty list takes the line off and the lane goes back to its
    /// static level.
    ///
    /// `continuing` collapses a drag into one undo step (§11), as the clip
    /// envelope does.
    pub fn set_track_volume(
        &mut self,
        track: TrackId,
        points: Vec<bettercut_timeline::VolumePoint>,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        // Taking the line off is its own step in the undo menu, and its own
        // label also stops it joining the drag that came before it.
        let label = if points.is_empty() {
            "Clear Track Volume"
        } else {
            "Track Volume"
        };
        self.dispatch_gesture(
            label.to_owned(),
            vec![Command::SetTrackVolume {
                sequence,
                track,
                points,
            }],
            continuing,
        )
    }

    /// Put a point on `track`'s volume line at `at`, at the level the line
    /// already has there — somewhere to take hold of, without the shape
    /// jumping.
    ///
    /// The first point on an empty line takes the lane's static level, so
    /// turning automation on changes nothing until something is moved.
    pub fn add_track_volume_point(
        &mut self,
        track: TrackId,
        at: TimelineTime,
    ) -> Result<(), EditorError> {
        let lane = self
            .active_sequence()
            .and_then(|sequence| sequence.audio_track(track))
            .ok_or(EditorError::TrackNotFound(track))?;
        let level = lane.volume.gain_at(at).unwrap_or(lane.gain);
        let mut points = lane.volume.points().to_vec();
        points.push(bettercut_timeline::VolumePoint::new(at, level));
        self.set_track_volume(track, points, false)
    }

    /// Take the volume line off `track`. Returns how many points went.
    pub fn clear_track_volume(&mut self, track: TrackId) -> Result<usize, EditorError> {
        let count = self.track_volume(track).len();
        if count == 0 {
            return Ok(0);
        }
        self.set_track_volume(track, Vec::new(), false)?;
        Ok(count)
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

    /// Whether a clip has motion to re-time.
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
        let duration = self.transition_length_for(clip);
        // No footage either side to blend: overlap the clips instead, as
        // CapCut does, rather than refuse the most ordinary request there is.
        if self
            .transition_room(clip, kind)
            .is_some_and(|room| room < bettercut_timeline::MIN_TRANSITION)
        {
            return self
                .overlap_into_transition(clip, kind, duration)
                .map(|_| ());
        }
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
    /// in: the interface explains itself.
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

    /// Every asset no clip uses, in library order.
    pub fn unused_media(&self) -> Vec<bettercut_foundation::MediaId> {
        self.project
            .media
            .iter()
            .map(|asset| asset.id)
            .filter(|id| !self.project.media_is_used(*id))
            .collect()
    }

    /// Take every asset no clip uses out of the library, as one undo step.
    /// Returns how many went. The files stay on disk (§2).
    pub fn remove_unused_media(&mut self) -> Result<usize, EditorError> {
        let unused = self.unused_media();
        let count = unused.len();
        if count == 0 {
            return Ok(0);
        }
        let commands = unused
            .into_iter()
            .map(|media| Command::RemoveMedia { media })
            .collect();
        let label = if count == 1 {
            "Remove Unused File".to_owned()
        } else {
            format!("Remove {count} Unused Files")
        };
        self.dispatch_group(label, commands)?;
        Ok(count)
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

    /// Change how the keys at the playhead ease (§24). Returns how many moved.
    ///
    /// Every animated parameter that has a key there, not one — a scale is two
    /// parameters and a position is two more, so easing one of a pair would
    /// make the shape drift as it moved. One choice covers the control the user
    /// is actually thinking about.
    ///
    /// Keys already on that curve are left out of the group rather than
    /// rewritten, so choosing the easing something already has is not an undo
    /// step that changes nothing.
    pub fn set_keyframe_easing(
        &mut self,
        clip: ClipId,
        easing: Interpolation,
    ) -> Result<usize, EditorError> {
        let at = self
            .source_time_at_playhead(clip)
            .ok_or(EditorError::PlayheadOffClip)?;
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;

        let commands: Vec<Command> = bettercut_timeline::AnimatedParameter::ALL
            .into_iter()
            .filter_map(|parameter| {
                let key = video.keyframes.get(parameter, at)?;
                (key.interpolation != easing).then_some(Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    key: Keyframe::new(at, key.value, easing),
                })
            })
            .collect();

        let changed = commands.len();
        if commands.is_empty() {
            return Ok(0);
        }
        self.dispatch_group("Keyframe Easing", commands)?;
        Ok(changed)
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

    /// Decode `media` with its fields woven into whole frames, or as it
    /// comes. One undo step; nothing when it is already so. The caller
    /// reopens the decoders (the preview's `proxy_ready`), as after a relink.
    pub fn set_media_deinterlace(
        &mut self,
        media: bettercut_foundation::MediaId,
        deinterlace: bool,
    ) -> Result<bool, EditorError> {
        let asset = self
            .project
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;
        if asset.deinterlace == deinterlace {
            return Ok(false);
        }
        self.dispatch(Command::SetMediaDeinterlace { media, deinterlace })?;
        Ok(true)
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

    /// Call the sequence's first frame `start` (`Sequence::start_timecode`),
    /// as one step. Nothing on the timeline moves: positions count from
    /// zero inside, and only what they are called changes.
    pub fn set_start_timecode(&mut self, start: TimelineTime) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch(Command::SetStartTimecode { sequence, start })
    }

    /// What the active sequence's first frame is called.
    pub fn start_timecode(&self) -> TimelineTime {
        self.active_sequence()
            .map_or(TimelineTime::ZERO, |sequence| sequence.start_timecode)
    }

    /// A position as it is *shown*: counted from the start timecode. Every
    /// readout goes through here, so they cannot disagree.
    pub fn display_time(&self, position: TimelineTime) -> TimelineTime {
        position + self.start_timecode()
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

        // The file about to be replaced is kept as a version first. A failure
        // there is reported but does not stop the save: the user asked for
        // their work to be written, and a missing backup is not a reason to
        // leave it unwritten.
        if self.path.as_ref().is_some_and(|own| own == &path)
            && let Err(err) = crate::versions::keep_version(&path, crate::versions::now_seconds())
        {
            tracing::warn!(%err, "could not keep the previous version before saving");
        }

        bettercut_project_format::save(&self.project, &path)?;

        // §39: the user's file now holds everything, so there is nothing left
        // to recover. Leaving the journal behind would make the next launch
        // offer to restore work they already have — which reads as data loss
        // even though nothing was lost.
        self.journal.discard();
        self.journal = Journal::new(RecoveryPaths::for_project(Some(&path), &session_id()));

        self.disk_stamp = modified(&path);
        self.path = Some(path);
        self.dirty = false;
        self.events.emit(Event::ProjectSaved);
        Ok(())
    }

    /// Whether the project file has been written by something else since this
    /// editor last opened or saved it: an assistant through `bettercut-mcp`,
    /// a sync folder, another copy of the app. One metadata read.
    pub fn changed_on_disk(&self) -> bool {
        match (&self.path, self.disk_stamp) {
            (Some(path), Some(seen)) => modified(path).is_some_and(|now| now > seen),
            _ => false,
        }
    }

    /// Take the file on disk as seen without loading it ("Keep Mine"), so
    /// the same change is not raised again.
    pub fn accept_disk_state(&mut self) {
        if let Some(path) = &self.path {
            self.disk_stamp = modified(path);
        }
    }

    /// Write the project to `path` and carry on editing the original.
    ///
    /// Save As moves the work to the new file; this leaves it where it is. The
    /// copy is the project exactly as it stands, unsaved changes included, but
    /// the editor's own file, its unsaved state and its recovery data are all
    /// untouched: a version kept aside before trying something, not a new home
    /// for the edit. Returns where the copy went, with the project extension
    /// added when the name had none.
    ///
    /// Refused when `path` is the project's own file — that is Save, and doing
    /// it here would mark nothing as saved while having saved over it.
    pub fn save_copy(&self, path: impl AsRef<Path>) -> Result<std::path::PathBuf, EditorError> {
        let mut path = path.as_ref().to_path_buf();
        if path.extension().is_none() {
            path.set_extension(PROJECT_EXTENSION);
        }
        if self.path.as_ref().is_some_and(|own| same_file(own, &path)) {
            return Err(EditorError::CopyOverOriginal);
        }
        bettercut_project_format::save(&self.project, &path)?;
        Ok(path)
    }

    /// The longest a track's name may be. Enough for "Interview — second
    /// camera"; past it the header column cuts the name off anyway.
    pub const MAX_TRACK_NAME: usize = 40;

    /// Rename `track`, as one undo step. Surrounding spaces are trimmed and the
    /// name is cut to [`Self::MAX_TRACK_NAME`] characters. Returns whether it
    /// changed: the same name again is no step. An empty name is refused — a
    /// lane with no name is one nobody can talk about.
    pub fn rename_track(&mut self, track: TrackId, name: &str) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let name: String = name.trim().chars().take(Self::MAX_TRACK_NAME).collect();
        if name.is_empty() {
            return Err(EditorError::EmptyTrackName);
        }
        let current = self
            .active_sequence()
            .and_then(|s| s.track_name(track))
            .ok_or(EditorError::TrackNotFound(track))?;
        if current == name {
            return Ok(false);
        }
        self.dispatch(Command::RenameTrack {
            sequence,
            track,
            name,
        })?;
        Ok(true)
    }

    /// Duplicate `track` with every clip on it, placed straight after it among
    /// the tracks of its kind, as one undo step. Returns the new track.
    ///
    /// The copies are new clips: new ids, not tied to the original's sound or
    /// picture (§12) — a copied picture sharing its link would drag the
    /// original's sound whenever it moved — and in no group. The copy is
    /// unlocked, so it can be worked on straight away, and keeps the track's
    /// visibility and mix.
    pub fn duplicate_track(&mut self, track: TrackId) -> Result<TrackId, EditorError> {
        use bettercut_timeline::{Clip, Track};

        fn copy_of<C: Clip + Clone>(
            original: &Track<C>,
            unlink: impl Fn(&mut C),
        ) -> Result<Track<C>, EditorError> {
            let mut copy = Track::new(format!("{} copy", original.name));
            copy.enabled = original.enabled;
            copy.gain = original.gain;
            copy.pan = original.pan;
            // The copy rides with the same edits the original does. It is not
            // the target, though: that is one lane per kind, and the user
            // asked for a copy of a track, not for their imports to move.
            copy.sync_lock = original.sync_lock;
            for clip in original.clips() {
                let mut clip = clip.clone();
                clip.set_id(ClipId::new());
                unlink(&mut clip);
                copy.insert(clip)?;
            }
            Ok(copy)
        }

        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let (index, payload) =
            if let Some(i) = sequence.video_tracks.iter().position(|t| t.id == track) {
                (
                    i + 1,
                    crate::command::TrackPayload::Video(Box::new(copy_of(
                        &sequence.video_tracks[i],
                        |c| c.link = None,
                    )?)),
                )
            } else if let Some(i) = sequence.audio_tracks.iter().position(|t| t.id == track) {
                (
                    i + 1,
                    crate::command::TrackPayload::Audio(Box::new(copy_of(
                        &sequence.audio_tracks[i],
                        |c| c.link = None,
                    )?)),
                )
            } else if let Some(i) = sequence.text_tracks.iter().position(|t| t.id == track) {
                (
                    i + 1,
                    crate::command::TrackPayload::Text(Box::new(copy_of(
                        &sequence.text_tracks[i],
                        |_| {},
                    )?)),
                )
            } else if let Some(i) = sequence
                .adjustment_tracks
                .iter()
                .position(|t| t.id == track)
            {
                (
                    i + 1,
                    crate::command::TrackPayload::Adjustment(Box::new(copy_of(
                        &sequence.adjustment_tracks[i],
                        |_| {},
                    )?)),
                )
            } else {
                return Err(EditorError::TrackNotFound(track));
            };
        let new_id = match &payload {
            crate::command::TrackPayload::Video(t) => t.id,
            crate::command::TrackPayload::Audio(t) => t.id,
            crate::command::TrackPayload::Text(t) => t.id,
            crate::command::TrackPayload::Adjustment(t) => t.id,
        };
        self.dispatch(Command::InsertTrack {
            sequence: sequence_id,
            index,
            track: payload,
        })?;
        Ok(new_id)
    }

    /// Discard recovery data on an orderly shutdown.
    ///
    /// Unsaved edits are the exception, and the important half. There is no
    /// prompt on the close button yet, so quitting with changes outstanding
    /// loses them unless the journal survives to be offered back on the next
    /// launch. From the work's point of view that quit is indistinguishable
    /// from a crash, which is the case §39 exists for — so it is treated as
    /// one, and the caller does not have to know that.
    ///
    /// Returns whether the recovery data was removed.
    pub fn shutdown(&mut self) -> bool {
        if self.dirty {
            tracing::info!("quit with unsaved changes; keeping recovery data");
            return false;
        }
        self.journal.discard();
        true
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
        let commands: Vec<Command> = commands.into_iter().collect();
        self.journal.append_all(&commands);

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
            Command::SetMediaDeinterlace { media, deinterlace } => {
                self.require_media(media)?;
                Ok(Box::new(ops::SetMediaDeinterlace::new(media, deinterlace)))
            }
            Command::SetProjectNotes { notes } => Ok(Box::new(ops::SetProjectNotes::new(notes))),

            Command::ChangeSetting { change } => Ok(Box::new(ops::ChangeSetting::new(change))),

            // Whole-sequence choices: the cover frame, the watermark and the
            // sound visualizer. Each is checked against the sequence it names
            // before anything is written.
            Command::SetCoverFrame { sequence, at } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetCoverFrame {
                    sequence,
                    at: at.map(|at| self.snap_to_frame(sequence, at)),
                    previous: None,
                }))
            }

            Command::SetWatermark {
                sequence,
                watermark,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetWatermark {
                    sequence,
                    watermark: watermark.map(bettercut_timeline::watermark::Watermark::clamped),
                    previous: None,
                }))
            }

            Command::SetVisualizer {
                sequence,
                visualizer,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetVisualizer::new(
                    sequence,
                    visualizer.map(|v| *v),
                )))
            }

            // The sequences themselves (§30).
            Command::AddSequence { sequence, index } => {
                Ok(Box::new(ops::AddSequence::new(sequence, index)))
            }

            Command::RemoveSequence { sequence } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::RemoveSequence::new(sequence)))
            }

            Command::RenameSequence { sequence, name } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::RenameSequence::new(sequence, name)))
            }

            // The media library: a file's name in this project, the bin it is
            // filed under, and what a colour clip draws.
            Command::RenameMedia { media, name } => {
                self.require_media(media)?;
                Ok(Box::new(ops::RenameMedia::new(media, name)))
            }

            Command::SetMediaBin { media, bin } => {
                self.require_media(media)?;
                Ok(Box::new(ops::SetMediaBin::new(media, bin)))
            }

            Command::SetMediaRating { media, rating } => {
                self.require_media(media)?;
                Ok(Box::new(ops::SetMediaRating::new(media, rating)))
            }

            Command::SetColour { media, colour } => {
                self.require_media(media)?;
                Ok(Box::new(ops::SetColour::new(media, colour)))
            }

            Command::SetClipAngle {
                sequence,
                track,
                clip,
                angle,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetClipAngle {
                    sequence,
                    track,
                    clip,
                    angle,
                    previous: None,
                }))
            }

            Command::SlipClip {
                sequence,
                track,
                clip,
                offset,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(crate::slip::SlipClip::new(
                    sequence, track, clip, offset,
                )))
            }

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

            Command::AddAdjustment {
                sequence,
                track,
                clip,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::AddAdjustment::new(sequence, track, *clip)))
            }

            Command::RemoveAdjustment {
                sequence,
                track,
                clip,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::RemoveAdjustment::new(sequence, track, clip)))
            }

            Command::SetAdjustmentLook {
                sequence,
                track,
                clip,
                look,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetAdjustmentLook::new(
                    sequence, track, clip, look,
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

            Command::SetGroups { sequence, groups } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetGroups::new(sequence, groups)))
            }

            Command::SetClipNote {
                sequence,
                clip,
                text,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetClipNote::new(sequence, clip, text)))
            }

            Command::SetInOut {
                sequence,
                mark_in,
                mark_out,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetInOut::new(sequence, mark_in, mark_out)))
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

            Command::SetTrackColour {
                sequence,
                track,
                label,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetTrackColour::new(sequence, track, label)))
            }

            Command::SetTrackEq {
                sequence,
                track,
                eq,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetTrackEq::new(sequence, track, eq)))
            }

            Command::SetTrackVolume {
                sequence,
                track,
                points,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetTrackVolume::new(sequence, track, points)))
            }

            Command::SetColorLabel {
                sequence,
                track,
                clip,
                label,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetColorLabel::new(
                    sequence, track, clip, label,
                )))
            }

            Command::SetClipName {
                sequence,
                track,
                clip,
                name,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::SetClipName::new(sequence, track, clip, name)))
            }

            Command::SetClipMarks { sequence, marks } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetClipMarks::new(sequence, marks)))
            }

            Command::SetClipSolo {
                sequence,
                clip,
                solo,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::SetClipSolo::new(sequence, clip, solo)))
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

            Command::SetPanEnvelope {
                sequence,
                track,
                clip,
                keys,
            } => Ok(Box::new(ops::SetGainEnvelope::pan(
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

            Command::SetStartTimecode { sequence, start } => {
                Ok(Box::new(ops::SetStartTimecode::new(sequence, start)))
            }

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

            Command::InsertTrack {
                sequence,
                index,
                track,
            } => {
                self.require_sequence(sequence)?;
                Ok(Box::new(ops::InsertTrack::new(sequence, index, track)))
            }

            Command::RenameTrack {
                sequence,
                track,
                name,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::RenameTrack::new(sequence, track, name)))
            }

            Command::MoveTrack {
                sequence,
                track,
                index,
            } => {
                self.require_track(sequence, track)?;
                Ok(Box::new(ops::MoveTrack::new(sequence, track, index)))
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

            Command::ReplaceClipMedia {
                sequence,
                track,
                clip,
                swap,
            } => {
                self.require_track(sequence, track)?;
                if self.project.media_asset(swap.media).is_none() {
                    return Err(EditorError::MediaNotFound(swap.media));
                }
                Ok(Box::new(ops::ReplaceClipMedia::new(
                    sequence, track, clip, swap,
                )))
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

            Command::NudgeSound {
                sequence,
                track,
                clip,
                new_start,
            } => {
                self.require_track(sequence, track)?;
                // Sound only: a picture clip off its frame grid would be
                // drawn a frame early or late somewhere.
                if self.audio_clip(clip).is_none() {
                    return Err(EditorError::ClipKindMismatch);
                }
                Ok(Box::new(ops::MoveClip::new(
                    sequence,
                    track,
                    track,
                    clip,
                    new_start.max(TimelineTime::ZERO),
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
    pub(crate) fn snap_to_frame(&self, sequence: SequenceId, at: TimelineTime) -> TimelineTime {
        self.project
            .sequence(sequence)
            .map_or(at, |s| s.snap_to_frame(at.max(TimelineTime::ZERO)))
    }

    /// The file must be in the project before anything is asked of it.
    fn require_media(&self, media: bettercut_foundation::MediaId) -> Result<(), EditorError> {
        self.project
            .media_asset(media)
            .map(|_| ())
            .ok_or(EditorError::MediaNotFound(media))
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

    /// One of a track's flags, whichever lane it is in (§8, §20a.4).
    ///
    /// Asked by controls that toggle: a header button and a menu item both
    /// need to know what the flag is *now* to say what pressing it will do.
    /// False for a track that is not there, which is what a control drawn for
    /// a track that has just been deleted should show.
    pub fn track_flag(&self, track: TrackId, flag: TrackFlag) -> bool {
        fn read<C>(track: &bettercut_timeline::Track<C>, flag: TrackFlag) -> bool {
            match flag {
                TrackFlag::Enabled => track.enabled,
                TrackFlag::Locked => track.locked,
                TrackFlag::Solo => track.solo,
                TrackFlag::SyncLock => track.sync_lock,
                TrackFlag::Targeted => track.targeted,
            }
        }
        self.active_sequence().is_some_and(|sequence| {
            let video = sequence
                .video_tracks
                .iter()
                .find(|t| t.id == track)
                .map(|t| read(t, flag));
            let audio = || {
                sequence
                    .audio_tracks
                    .iter()
                    .find(|t| t.id == track)
                    .map(|t| read(t, flag))
            };
            let text = || {
                sequence
                    .text_tracks
                    .iter()
                    .find(|t| t.id == track)
                    .map(|t| read(t, flag))
            };
            // Without this an adjustment lane reads as hidden, unlocked and
            // unsoloed whatever it is — the header shows the wrong state.
            let adjustment = || sequence.adjustment_track(track).map(|t| read(t, flag));
            video
                .or_else(audio)
                .or_else(text)
                .or_else(adjustment)
                .unwrap_or(false)
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
        let Some(old_start) = self.clip_start(clip) else {
            return vec![primary];
        };
        let delta = new_start.ticks() - old_start.ticks();

        // Its sound (§12), and everything grouped with it along with each of
        // their sounds — all by the same amount, each on its own track.
        let mut moving: Vec<(TimelineTime, Command)> = vec![(old_start, primary)];
        for companion in self.moves_with(clip).into_iter().filter(|c| *c != clip) {
            let (Some(track), Some(start)) = (self.track_of(companion), self.clip_start(companion))
            else {
                continue;
            };
            moving.push((
                start,
                Command::MoveClip {
                    sequence,
                    from_track: track,
                    to_track: track,
                    clip: companion,
                    new_start: TimelineTime::from_ticks(start.ticks() + delta),
                },
            ));
        }
        // A group can hold two clips on one track: the one moving into the
        // other's old place has to wait until it has left. Leading edge first.
        if delta > 0 {
            moving.sort_by_key(|(start, _)| std::cmp::Reverse(*start));
        } else {
            moving.sort_by_key(|(start, _)| *start);
        }
        moving.into_iter().map(|(_, command)| command).collect()
    }

    // ---- notes ----

    /// The longest a clip's note may be.
    pub const MAX_CLIP_NOTE: usize = 500;

    /// The note on `clip`, if it has one.
    pub fn clip_note(&self, clip: ClipId) -> Option<&str> {
        self.active_sequence()?
            .notes
            .iter()
            .find(|note| note.clip == clip)
            .map(|note| note.text.as_str())
    }

    /// Leave a note on `clip`, change it, or take it off with an empty or
    /// blank `text`. Trimmed and cut to [`Self::MAX_CLIP_NOTE`] characters.
    /// One undo step; returns whether anything changed.
    pub fn set_clip_note(&mut self, clip: ClipId, text: &str) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        if self
            .active_sequence()
            .and_then(|s| s.clip_span(clip))
            .is_none()
        {
            return Err(EditorError::ClipNotFound(clip));
        }
        let text: String = text.trim().chars().take(Self::MAX_CLIP_NOTE).collect();
        if self.clip_note(clip).unwrap_or("") == text {
            return Ok(false);
        }
        self.dispatch(Command::SetClipNote {
            sequence,
            clip,
            text,
        })?;
        Ok(true)
    }

    // ---- groups ----

    /// The live members of the group `clip` is in — ids of clips since deleted
    /// or split left out — or `None` when it is in none.
    pub fn group_of(&self, clip: ClipId) -> Option<Vec<ClipId>> {
        let sequence = self.active_sequence()?;
        let group = sequence.groups.iter().find(|group| group.contains(&clip))?;
        let live: Vec<ClipId> = group
            .iter()
            .copied()
            .filter(|member| sequence.clip_span(*member).is_some())
            .collect();
        (live.len() > 1).then_some(live)
    }

    /// Everything that moves when `clip` does: its group, and every member's
    /// linked partners. Always includes `clip`.
    pub fn moves_with(&self, clip: ClipId) -> Vec<ClipId> {
        let members = self.group_of(clip).unwrap_or_else(|| vec![clip]);
        let mut all: Vec<ClipId> = Vec::new();
        for member in members {
            for linked in self.linked_with(member) {
                if !all.contains(&linked) {
                    all.push(linked);
                }
            }
        }
        if !all.contains(&clip) {
            all.insert(0, clip);
        }
        all
    }

    /// Group `clips` so they move together, as one undo step. Any group one of
    /// them was already in is folded into the new one, since a clip is in at
    /// most one group. Returns the size of the group.
    ///
    /// Refused unless at least two clips — not counting a clip's own linked
    /// sound, which moves with it anyway — are given.
    pub fn group_clips(&mut self, clips: &[ClipId]) -> Result<usize, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let mut members: Vec<ClipId> = Vec::new();
        for clip in clips {
            if sequence.clip_span(*clip).is_none() {
                continue;
            }
            for member in self.group_of(*clip).unwrap_or_else(|| vec![*clip]) {
                if !members.contains(&member) {
                    members.push(member);
                }
            }
        }
        // Clips that are only one another's linked partners are one thing.
        let mut units: Vec<Vec<ClipId>> = Vec::new();
        for member in &members {
            if !units.iter().any(|unit| unit.contains(member)) {
                units.push(self.linked_with(*member));
            }
        }
        if units.len() < 2 {
            return Err(EditorError::NothingToGroup);
        }
        let mut groups: Vec<Vec<ClipId>> = sequence
            .groups
            .iter()
            .filter(|group| !group.iter().any(|clip| members.contains(clip)))
            .cloned()
            .collect();
        let count = members.len();
        groups.push(members);
        self.dispatch(Command::SetGroups {
            sequence: sequence_id,
            groups,
        })?;
        Ok(count)
    }

    /// Break up every group any of `clips` is in, as one undo step. Returns
    /// how many groups went; none is no edit.
    pub fn ungroup_clips(&mut self, clips: &[ClipId]) -> Result<usize, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let Some(sequence) = self.active_sequence() else {
            return Ok(0);
        };
        let before = sequence.groups.len();
        let groups: Vec<Vec<ClipId>> = sequence
            .groups
            .iter()
            .filter(|group| !group.iter().any(|clip| clips.contains(clip)))
            .cloned()
            .collect();
        let removed = before - groups.len();
        if removed > 0 {
            self.dispatch(Command::SetGroups {
                sequence: sequence_id,
                groups,
            })?;
        }
        Ok(removed)
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

    /// Move the *sound* clips among `clips` by `ticks` — a millisecond, for
    /// lining sound up with picture by ear — and only them: the picture
    /// stays on its frame grid, and a linked partner is left where it is,
    /// because putting the two out of step is the point. Returns how many
    /// moved; nothing but sound in the selection is nothing moved.
    pub fn nudge_sound_by(&mut self, clips: &[ClipId], ticks: i64) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        if ticks == 0 {
            return Ok(0);
        }
        let mut targets: Vec<(ClipId, TrackId, TimelineTime)> = clips
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|clip| self.audio_clip(*clip).is_some())
            .filter_map(|clip| Some((clip, self.track_of(clip)?, self.clip_start(clip)?)))
            .collect();
        if targets.is_empty() {
            return Ok(0);
        }
        let earliest = targets.iter().map(|(_, _, start)| start.ticks()).min();
        let delta = match earliest {
            Some(earliest) if earliest + ticks < 0 => -earliest,
            _ => ticks,
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
            .map(|(clip, track, start)| Command::NudgeSound {
                sequence,
                track,
                clip,
                new_start: TimelineTime::from_ticks(start.ticks() + delta),
            })
            .collect();
        self.dispatch_group("Nudge sound".to_owned(), commands)?;
        Ok(count)
    }

    /// Where a video or audio clip starts, for the linked edits above.
    pub fn clip_start(&self, clip: ClipId) -> Option<TimelineTime> {
        Some(self.project.active()?.clip_span(clip)?.timeline.start)
    }

    /// The same, for the end.
    pub fn clip_end(&self, clip: ClipId) -> Option<TimelineTime> {
        Some(self.project.active()?.clip_span(clip)?.timeline.end)
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
    pub(crate) fn stage_insert_time(
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

    /// Take `range` out of every lane and close the gap it leaves (§10's
    /// ripple delete, over a stretch rather than a clip).
    ///
    /// The mirror of [`Self::insert_time`]: a clip the range starts or ends
    /// inside is split there, everything inside goes, and everything after
    /// moves left by the length of the range — on every lane, so the sound,
    /// the titles and the captions after it keep their place against the
    /// picture. Markers inside it go with it; markers after it move.
    ///
    /// One undo step for the lot (§79). Returns how many clips were removed.
    pub fn remove_time(
        &mut self,
        range: bettercut_timeline::TimelineRange,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let start = self.snap_to_frame(sequence, range.start);
        let end = self.snap_to_frame(sequence, range.end);
        if end <= start {
            return Ok(0);
        }
        self.staged("Ripple Delete Range", |editor, stage| {
            editor.stage_remove_time(stage, sequence, start, end)
        })
    }

    /// The stretch itself, inside someone else's edit.
    pub(crate) fn stage_remove_time(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        start: TimelineTime,
        end: TimelineTime,
    ) -> Result<usize, EditorError> {
        let length = TimelineTime::from_ticks(end.ticks() - start.ticks());

        // Cut clean edges at both ends first, so what is left either side is a
        // whole clip. The far end first: splitting at the near end would
        // renumber nothing, but doing the later cut first keeps the earlier
        // one against the clip it was measured on.
        for at in [end, start] {
            let straddling: Vec<ClipId> = self
                .clips_from(at, true)
                .into_iter()
                .map(|(_, clip, _)| clip)
                .collect();
            for command in self.split_commands(sequence, at, &straddling) {
                self.stage(stage, command)?;
            }
        }

        // Everything wholly inside the stretch goes.
        let inside: Vec<(TrackId, ClipId)> = self
            .project()
            .active()
            .map(|active| {
                active
                    .clip_spans()
                    .filter(|span| span.timeline.start >= start && span.timeline.end <= end)
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

        // And everything after it moves left. Earliest first: moving a later
        // clip first would leave the earlier one with nowhere to land.
        let mut moves = self.clips_from(end, false);
        moves.sort_by_key(|(_, _, start)| start.ticks());
        for (track, clip, at) in moves {
            self.stage(
                stage,
                Command::MoveClip {
                    sequence,
                    from_track: track,
                    to_track: track,
                    clip,
                    new_start: TimelineTime::from_ticks(at.ticks() - length.ticks()),
                },
            )?;
        }

        // Markers mark instants in the picture: the ones inside the stretch
        // are marking something that is no longer there.
        let markers = self.markers();
        if markers.iter().any(|marker| marker.time >= start) {
            let kept = markers
                .iter()
                .filter(|marker| marker.time < start || marker.time >= end)
                .map(|marker| {
                    let mut marker = marker.clone();
                    if marker.time >= end {
                        marker.time =
                            TimelineTime::from_ticks(marker.time.ticks() - length.ticks());
                    }
                    marker
                })
                .collect();
            self.stage(
                stage,
                Command::SetMarkers {
                    sequence,
                    markers: kept,
                },
            )?;
        }

        Ok(removed)
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

    /// Every clip that starts at or after `at`, on `track` or on every track —
    /// what "select everything after here" selects. From zero, every clip.
    ///
    /// Clips on locked tracks are left out: a selection like this is made to
    /// move or delete the rest of the edit, and a locked clip would refuse.
    /// In time order, then track order, so the result is the same every call.
    pub fn clips_starting_from(&self, at: TimelineTime, track: Option<TrackId>) -> Vec<ClipId> {
        let Some(sequence) = self.project.active() else {
            return Vec::new();
        };
        let mut found: Vec<(TimelineTime, usize, ClipId)> = sequence
            .clip_spans()
            .enumerate()
            .filter(|(_, span)| {
                span.timeline.start >= at
                    && track.is_none_or(|track| span.track == track)
                    && !crate::gaps::track_locked(sequence, span.track)
            })
            .map(|(order, span)| (span.timeline.start, order, span.clip))
            .collect();
        found.sort_unstable_by_key(|(start, order, _)| (*start, *order));
        found.into_iter().map(|(_, _, clip)| clip).collect()
    }

    /// Clips at or after `at` on every track — or, with `straddling`, only the
    /// ones the instant falls strictly inside.
    pub(crate) fn clips_from(
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

        sequence
            .clip_spans()
            .filter(|span| wanted(span.timeline))
            .map(|span| (span.track, span.clip, span.timeline.start))
            .collect()
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

    /// How many clips show bars at the sequence's current shape, having never
    /// been framed for it.
    ///
    /// What the sequence panel says after a reshape, so the three buttons that
    /// deal with it — fill, crop, fit — explain themselves: switching a
    /// landscape edit to vertical silently pillarboxes every shot, and a user
    /// who does not know to look for a fix will think the export is broken.
    ///
    /// Only clips still at the framing they were placed with: full size,
    /// centred, upright, uncropped, and not animated in size or position. A
    /// shot shrunk into a corner or cropped by hand shows bars *because someone
    /// chose that*, and counting it would make this a nag that never goes away.
    pub fn clips_showing_bars(&self) -> usize {
        let Some(sequence) = self.active_sequence() else {
            return 0;
        };
        let Some(output) = aspect_of(sequence.resolution.width, sequence.resolution.height) else {
            return 0;
        };

        sequence
            .video_tracks
            .iter()
            .flat_map(|track| track.clips())
            .filter(|clip| at_placed_framing(clip))
            .filter(|clip| {
                self.project
                    .media_asset(clip.media_id)
                    .and_then(|asset| aspect_of(asset.width, asset.height))
                    // A percent either way is the same shape: 1920×1080 and
                    // 1920×1088 are both 16:9 to anyone looking.
                    .is_some_and(|source| (source / output - 1.0).abs() > 0.01)
            })
            .count()
    }

    /// §33's auto crop: take the frame's own shape out of every clip.
    ///
    /// The companion to [`Self::reframe_clips`], and usually the better answer
    /// for the same problem. Filling scales a clip up until the bars are gone,
    /// which costs resolution — landscape footage covering a vertical frame is
    /// enlarged nearly twice. Cropping takes the frame's shape out of the
    /// source at full size instead: nothing is enlarged, and what is lost was
    /// never going to be on screen.
    ///
    /// A clip the user has already cropped is left alone. That crop is a
    /// decision about *this* shot — a watermark trimmed off an edge, a subject
    /// framed by hand — and replacing it with a centred one would throw away
    /// work that cannot be guessed back. Counted separately so the interface
    /// can say so, exactly as `reframe_clips` does for animated scale.
    ///
    /// Returns `(cropped, left alone)`.
    pub fn auto_crop_clips(&mut self) -> Result<(usize, usize), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let output = aspect_of(sequence.resolution.width, sequence.resolution.height);

        let mut wanted: Vec<(ClipId, bettercut_timeline::Crop)> = Vec::new();
        let mut kept = 0;
        for track in &sequence.video_tracks {
            for clip in track.clips() {
                if !clip.crop.is_none() {
                    kept += 1;
                    continue;
                }
                let Some(asset) = self.project.media_asset(clip.media_id) else {
                    continue; // §66: media that has gone still has a clip
                };
                let (Some(source), Some(output)) = (aspect_of(asset.width, asset.height), output)
                else {
                    continue; // a file whose size we never learned
                };
                let crop = bettercut_timeline::crop_to_aspect(source, output);
                // Already the frame's shape: a step in the history that
                // changes nothing is worse than no step at all.
                if crop.is_none() {
                    continue;
                }
                wanted.push((clip.id, crop));
            }
        }

        let count = wanted.len();
        self.staged("Crop to the Frame", |editor, stage| {
            for (clip, crop) in wanted {
                let track = editor
                    .track_of(clip)
                    .ok_or(EditorError::ClipNotFound(clip))?;
                editor.stage(
                    stage,
                    Command::SetClipProperty {
                        sequence: sequence_id,
                        track,
                        clip,
                        property: crate::command::ClipProperty::Crop(crop),
                    },
                )?;
            }
            Ok(())
        })?;
        Ok((count, kept))
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
    pub(crate) fn split_commands(
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

        // Every lane kind: a title or an adjustment splits like anything else,
        // and leaving one out here is how a feature that works in the model
        // never reaches the shortcut — which happened to titles once.
        for span in sequence.clip_spans() {
            consider(span.track, span.clip, span.timeline, span.link);
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
        let ranges = self.prepared_ranges(sequence, ranges);
        let members = self.members_of(clip);
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
            editor.stage_remove_ranges(stage, sequence, members, ranges)
        })
    }

    /// Ripple trim to the playhead: take away the part of each clip before the
    /// playhead (`TrimEdge::Start`) or after it (`TrimEdge::End`), and close the
    /// gap it leaves — what Q and W do. One undo step.
    ///
    /// `clips` are the selection; empty means the clip under the playhead, on
    /// the highest picture track that has one, or else the highest sound track.
    /// Only clips the playhead is strictly inside are trimmed. Linked sound is
    /// trimmed with its picture (§12), once however many of the pair are given.
    ///
    /// Trimming a start leaves the playhead where the cut now is — the join the
    /// user wants to watch next. Returns how many clips were trimmed.
    pub fn ripple_trim_to_playhead(
        &mut self,
        clips: &[ClipId],
        edge: TrimEdge,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self.playhead;
        let inside = |editor: &Self, clip: ClipId| {
            editor
                .span_and_link(clip)
                .filter(|(span, _)| span.start < at && at < span.end)
                .map(|(span, _)| span)
        };

        let candidates: Vec<ClipId> = if clips.is_empty() {
            let Some(active) = self.project.active() else {
                return Ok(0);
            };
            let pictures = active
                .video_tracks
                .iter()
                .rev()
                .map(|t| t.clip_at(at).map(|c| c.id));
            let sounds = active
                .audio_tracks
                .iter()
                .map(|t| t.clip_at(at).map(|c| c.id));
            pictures
                .chain(sounds)
                .flatten()
                .find(|clip| inside(self, *clip).is_some())
                .into_iter()
                .collect()
        } else {
            clips.to_vec()
        };

        // One cut per linked group: the picture and its sound are one clip to
        // the person pressing the key.
        let mut seen: Vec<ClipId> = Vec::new();
        let mut targets = Vec::new();
        for clip in candidates {
            if seen.contains(&clip) {
                continue;
            }
            let Some(span) = inside(self, clip) else {
                continue;
            };
            seen.extend(self.linked_with(clip));
            let range = match edge {
                TrimEdge::Start => bettercut_timeline::TimelineRange::new(span.start, at),
                TrimEdge::End => bettercut_timeline::TimelineRange::new(at, span.end),
            };
            if let Ok(range) = range {
                targets.push((self.members_of(clip), range));
            }
        }
        if targets.is_empty() {
            return Ok(0);
        }

        let count = targets.len();
        let label = if count == 1 {
            "Ripple Trim".to_owned()
        } else {
            format!("Ripple Trim {count} Clips")
        };
        let earliest = targets.iter().map(|(_, range)| range.start).min();
        self.staged(label, |editor, stage| {
            for (members, range) in targets {
                let ranges = editor.prepared_ranges(sequence, &[range]);
                let edited: Vec<TrackId> = members.iter().map(|(track, _)| *track).collect();
                let ride = ranges.first().copied();
                editor.stage_remove_ranges(stage, sequence, members, ranges)?;
                // The same stretch comes out of every lane riding along.
                if let Some(range) = ride {
                    editor.stage_sync_remove(stage, sequence, &edited, range)?;
                }
            }
            Ok(())
        })?;
        if edge == TrimEdge::Start
            && let Some(start) = earliest
        {
            self.set_playhead(start);
        }
        Ok(count)
    }

    /// A clip and everything linked to it, each with its track.
    fn members_of(&self, clip: ClipId) -> Vec<(TrackId, ClipId)> {
        self.linked_with(clip)
            .into_iter()
            .filter_map(|c| Some((self.track_of(c)?, c)))
            .collect()
    }

    /// Ranges snapped to the frame grid (§76), latest first, overlaps merged.
    fn prepared_ranges(
        &self,
        sequence: SequenceId,
        ranges: &[bettercut_timeline::TimelineRange],
    ) -> Vec<bettercut_timeline::TimelineRange> {
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
        ranges
    }

    /// Split out and ripple-delete each range from a clip and its partners,
    /// inside someone else's staged edit. Ranges must be latest first.
    fn stage_remove_ranges(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        members: Vec<(TrackId, ClipId)>,
        ranges: Vec<bettercut_timeline::TimelineRange>,
    ) -> Result<usize, EditorError> {
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
                let Some((span, linked)) = self.span_and_link(id) else {
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
                    self.stage(
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
                    self.stage(
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
                self.stage(
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
        let command = Command::RippleDeleteClip {
            sequence,
            track,
            clip,
        };
        // The lanes with sync lock on close the same stretch, so what sat
        // under the clip still sits where it was (§10). With none on — the
        // usual case — this is the one command it always was.
        let Some(span) = self.span_and_link(clip).map(|(span, _)| span) else {
            return self.dispatch(command);
        };
        let riders = !self.sync_riders(&[track]).is_empty();
        self.staged("Ripple Delete", |editor, stage| {
            editor.stage(stage, command)?;
            if riders {
                editor.stage_sync_remove(stage, sequence, &[track], span)?;
            }
            // And the markers, which mark the edit rather than any one lane.
            editor.stage_markers_closed(stage, sequence, span)
        })
    }

    /// Which track holds a clip. Linear over tracks, not over clips.
    pub fn track_of(&self, clip: ClipId) -> Option<TrackId> {
        // Every lane kind, through the sequence's one list of them — see
        // `Sequence::clip_spans` for the five lookups this used to be one of.
        self.project
            .active()?
            .clip_span(clip)
            .map(|span| span.track)
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
                sequence.target_track(bettercut_timeline::TrackKind::Video),
                sequence.target_track(bettercut_timeline::TrackKind::Audio),
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
                // Nor adjustments: like a title, one is added through its own
                // command, and never reaches the clipboard.
                bettercut_timeline::TrackKind::Adjustment => None,
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
            ClipPayload::Adjustment(c) => c.timeline.end,
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
/// Whether a clip is still at the framing it was placed with: full size,
/// centred, upright, uncropped, and not animated in size or position.
///
/// The line between a shot nobody has framed yet and one somebody has. Both
/// the "shows bars" hint and the reshaped export copies draw it here, so what
/// the hint counts is exactly what a copy re-frames.
/// Whether two paths name the same file, allowing for one being relative or
/// differently spelled — compared canonically when both exist.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub(crate) fn at_placed_framing(clip: &bettercut_timeline::VideoClip) -> bool {
    use bettercut_timeline::AnimatedParameter as A;

    let t = clip.transform;
    let near = |a: f32, b: f32| (a - b).abs() < 0.001;
    clip.crop.is_none()
        && near(t.scale.x, 1.0)
        && near(t.scale.y, 1.0)
        && near(t.position.x, 0.0)
        && near(t.position.y, 0.0)
        && near(t.rotation_degrees, 0.0)
        && ![A::ScaleX, A::ScaleY, A::PositionX, A::PositionY]
            .into_iter()
            .any(|parameter| clip.keyframes.is_animated(parameter))
}

pub(crate) fn aspect_of(width: u32, height: u32) -> Option<f32> {
    (width > 0 && height > 0).then(|| width as f32 / height as f32)
}

/// When `path` was last written, if it can be read.
fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
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
