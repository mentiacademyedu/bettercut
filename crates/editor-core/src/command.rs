//! Commands: the only way project state changes (§10, §11, §79).
//!
//! ## Two representations, deliberately
//!
//! * [`Command`] is the **request** — serializable, what the UI sends and what
//!   §38.2's journal appends to disk.
//! * [`EditorCommand`] is the **executed operation** — it carries the undo
//!   payload (the removed clip, the previous name), which is derived at execute
//!   time and never written to the journal.
//!
//! Keeping them apart is what makes §11's "commands must be serializable" and
//! "do not snapshot the entire project" hold at the same time: the journal
//! records intent, and replaying intent reproduces the state.

use bettercut_foundation::{ClipId, FrameRate, SequenceId, TimelineTime, TrackId};
use bettercut_project_format::Project;
use bettercut_timeline::{AnimatedParameter, AudioClip, Keyframe, TrackKind, VideoClip};
use serde::{Deserialize, Serialize};

use crate::error::EditorError;
pub use crate::ops::TrimEdge;

/// A clip of either kind, so commands can be written once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipPayload {
    Video(Box<VideoClip>),
    Audio(Box<AudioClip>),
    /// §26's titles. Here because the operations that hand a whole clip
    /// around — split's undo, ripple delete's undo — apply to them too, and a
    /// separate payload type would mean writing those commands twice.
    Text(Box<bettercut_timeline::TextClip>),
}

impl ClipPayload {
    pub fn id(&self) -> ClipId {
        match self {
            Self::Video(c) => c.id,
            Self::Audio(c) => c.id,
            Self::Text(c) => c.id,
        }
    }

    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Text(_) => TrackKind::Text,
        }
    }

    pub fn start(&self) -> TimelineTime {
        match self {
            Self::Video(c) => c.timeline.start,
            Self::Audio(c) => c.timeline.start,
            Self::Text(c) => c.timeline.start,
        }
    }
}

// So the operations that are generic over a track — split, ripple delete — can
// wrap whatever kind of clip came out without matching on it. Written out
// rather than derived: three lines each, and a macro would be harder to read
// than the thing it replaced.
impl From<VideoClip> for ClipPayload {
    fn from(clip: VideoClip) -> Self {
        Self::Video(Box::new(clip))
    }
}

impl From<AudioClip> for ClipPayload {
    fn from(clip: AudioClip) -> Self {
        Self::Audio(Box::new(clip))
    }
}

impl From<bettercut_timeline::TextClip> for ClipPayload {
    fn from(clip: bettercut_timeline::TextClip) -> Self {
        Self::Text(Box::new(clip))
    }
}

/// A whole track, held by `RemoveTrack` so undo can restore it intact.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackPayload {
    Video(Box<bettercut_timeline::VideoTrack>),
    Audio(Box<bettercut_timeline::AudioTrack>),
    /// §26's text lanes, including §27's captions. A lane that can be created
    /// and not removed is a trap, and the caption import creates one.
    Text(Box<bettercut_timeline::TextTrack>),
}

/// Boolean track flags, so one command covers hide/mute/lock (§10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackFlag {
    /// Visible for video tracks, unmuted for audio tracks.
    Enabled,
    Locked,
}

/// A project setting the user can change (§13, §67).
///
/// A closed enum rather than a path-and-value pair: the journal replays these
/// after a crash, so every variant that can ever be written has to be one the
/// current build knows how to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "setting", content = "value", rename_all = "snake_case")]
pub enum SettingChange {
    /// §13: "Allow the user to disable automatic proxies."
    AutoGenerateProxies(bool),
    /// §13/§17: which resolution proxies are generated at.
    PerformanceMode(bettercut_project_format::PerformanceMode),
    /// §67's ceiling on the media cache.
    CacheLimitBytes(u64),
}

impl SettingChange {
    /// Shown in the undo menu.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::AutoGenerateProxies(_) => "Automatic Proxies",
            Self::PerformanceMode(_) => "Performance Mode",
            Self::CacheLimitBytes(_) => "Cache Limit",
        }
    }
}

/// One editable thing about a text overlay (§26).
///
/// Separate from [`ClipProperty`] rather than folded into it: the two overlap
/// in the transform controls and nowhere else, and a single enum would mean
/// every match on a clip property having to say "not for text" for the words
/// and the font.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "property", rename_all = "snake_case")]
pub enum TextProperty {
    /// The words themselves.
    Content(String),
    /// Everything about how they are drawn — font, colour, outline, the lot.
    ///
    /// One property rather than a dozen because the interface edits a style as
    /// a whole and the rasterizer consumes it as a whole; splitting it would
    /// buy a finer undo history for a control nobody adjusts one field at a
    /// time. Boxed: a `TextStyle` is much larger than the other variants, and
    /// the enum would be that size everywhere.
    Style(Box<bettercut_text::TextStyle>),
    Position {
        x: f32,
        y: f32,
    },
    Scale {
        x: f32,
        y: f32,
    },
    Rotation(f32),
    Opacity(f32),
}

impl TextProperty {
    /// The text-clip equivalent of a clip property, where there is one (§26).
    ///
    /// The preview's drag handles produce a [`ClipProperty`] — they do not know
    /// or care what kind of layer they are moving, and should not have to.
    /// `None` for the properties a title does not have: colour grading, blur
    /// and gain are about footage.
    pub fn from_clip_property(property: ClipProperty) -> Option<Self> {
        match property {
            ClipProperty::Position { x, y } => Some(Self::Position { x, y }),
            ClipProperty::Scale { x, y } => Some(Self::Scale { x, y }),
            ClipProperty::Rotation(degrees) => Some(Self::Rotation(degrees)),
            ClipProperty::Opacity(value) => Some(Self::Opacity(value)),
            ClipProperty::Brightness(_)
            | ClipProperty::Contrast(_)
            | ClipProperty::Saturation(_)
            | ClipProperty::Blur(_)
            | ClipProperty::Gain(_) => None,
        }
    }

    /// What this property is called in the undo history.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Content(_) => "text",
            Self::Style(_) => "text style",
            Self::Position { .. } => "text position",
            Self::Scale { .. } => "text scale",
            Self::Rotation(_) => "text rotation",
            Self::Opacity(_) => "text opacity",
        }
    }
}

/// The serializable request form. This is what the UI sends (§55) and what the
/// autosave journal records (§38.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    RenameProject {
        name: String,
    },
    ChangeSetting {
        change: SettingChange,
    },
    /// Adjust one property of one clip (§59).
    SetClipProperty {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        property: ClipProperty,
    },
    /// Add or replace one keyframe (§10's `SetKeyframeCommand`, §24).
    ///
    /// The key carries its own time, in the source media — see
    /// [`bettercut_timeline::keyframe`] for why keys are anchored there rather
    /// than to the timeline.
    SetKeyframe {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        parameter: AnimatedParameter,
        key: Keyframe,
    },
    RemoveKeyframe {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        parameter: AnimatedParameter,
        time: bettercut_foundation::MediaTime,
    },
    /// Add a text overlay (§26).
    ///
    /// Carries the whole clip rather than just the words, for the same reason
    /// [`Self::PasteClip`] does: §38.2 replays commands after a crash, and a
    /// command that invented a fresh id on replay would produce a different
    /// project than the one that was lost.
    AddText {
        sequence: SequenceId,
        track: TrackId,
        clip: Box<bettercut_timeline::TextClip>,
    },
    RemoveText {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
    },
    /// Change one thing about a text overlay (§26).
    SetTextProperty {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        property: TextProperty,
    },
    /// Detach a video's sound from its picture (§12), so the two can be moved,
    /// trimmed and re-timed independently.
    ///
    /// Clears the link on every clip carrying it rather than on one: a link
    /// held by one side only would be a clip tied to nothing, and every edit
    /// afterwards would have to keep checking for that.
    Unlink {
        sequence: SequenceId,
        link: bettercut_foundation::LinkId,
    },
    /// Change how fast a clip plays (§51).
    ///
    /// A separate command from [`Self::SetClipProperty`] because it is not a
    /// property of the picture — it changes how long the clip *is*, which the
    /// track has to make room for. The others cannot fail for want of space.
    SetClipSpeed {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        speed: bettercut_foundation::Rational,
    },
    /// Put a transition on the end of a clip, or take it off (§25).
    ///
    /// `None` removes. One command for both, for the same reason
    /// [`Self::RemoveKeyframe`] is separate but symmetrical: the undo logic is
    /// identical and splitting it would duplicate the only subtle part.
    ///
    /// The duration is a request. What is actually stored is clamped to what
    /// the two clips can support, because the alternative is a transition that
    /// looks right in the interface and flashes black on export.
    SetTransition {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        transition: Option<bettercut_timeline::Transition>,
    },
    /// Adjust the finished picture rather than one clip (§22).
    ///
    /// Reuses [`ClipProperty`] because the controls are the same ones — the
    /// difference is what they apply to, and that is this command's identity,
    /// not the property's. A second parallel enum would double every match in
    /// the interface for no gain.
    SetSequenceProperty {
        sequence: SequenceId,
        property: ClipProperty,
    },
    /// Take an asset out of the library (§12).
    ///
    /// Refused by the model while any clip still references it — dropping it
    /// would leave cuts pointing at nothing. Removing the clips first is the
    /// user's decision, not something this should do on their behalf.
    RemoveMedia {
        media: bettercut_foundation::MediaId,
    },
    /// Point an asset at a file that has moved (§66).
    ///
    /// Carries the new size rather than re-reading it, because §38.2 replays
    /// commands after a crash: a command that stats the filesystem would
    /// produce a different result on replay than it did when the user ran it.
    /// Validation belongs to whoever builds the command, not to replaying it.
    RelinkMedia {
        media: bettercut_foundation::MediaId,
        path: std::path::PathBuf,
        file_size: u64,
    },
    /// Change a sequence's output format (§8, §36).
    ///
    /// Both together, because they are one decision from the user's side —
    /// "make this a 1080p50 project" — and applying them as two commands would
    /// put two entries in the undo history for one choice.
    SetSequenceFormat {
        sequence: SequenceId,
        resolution: ResolutionRepr,
        frame_rate: FrameRate,
    },
    AddTrack {
        sequence: SequenceId,
        kind: TrackKindRepr,
        name: String,
        /// Minted by the caller, not by the command.
        ///
        /// Every ID a command creates has to be part of the request, or the
        /// same command executed twice produces two different projects.
        /// §38.2's journal replays commands to rebuild state after a crash,
        /// and redo re-executes them — both need the second run to be
        /// identical to the first.
        id: TrackId,
    },
    RemoveTrack {
        sequence: SequenceId,
        track: TrackId,
    },
    SetTrackFlag {
        sequence: SequenceId,
        track: TrackId,
        flag: TrackFlag,
        value: bool,
    },
    AddClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipPayload,
    },
    RemoveClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
    },
    MoveClip {
        sequence: SequenceId,
        from_track: TrackId,
        to_track: TrackId,
        clip: ClipId,
        new_start: TimelineTime,
    },
    TrimClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        edge: TrimEdge,
        to: TimelineTime,
    },
    SplitClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        at: TimelineTime,
        /// The halves' identities, supplied for the same reason as
        /// `AddTrack::id`.
        left: ClipId,
        right: ClipId,
        /// Fresh links for the two halves, when the clip was linked (§12).
        ///
        /// A split clones the clip, so without this both halves of the
        /// picture and both halves of the sound would share the original
        /// link — four clips re-timing together where two pairs were meant.
        /// The left halves share the first and the right halves the second.
        /// Supplied rather than generated so a replay (§38.2) reproduces the
        /// same pairing.
        #[serde(default)]
        relink: Option<(bettercut_foundation::LinkId, bettercut_foundation::LinkId)>,
    },
    RippleDeleteClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
    },
    PasteClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipPayload,
        at: TimelineTime,
        /// Identity of the pasted copy. Supplied so pasting the same clipboard
        /// entry twice yields two distinct, *reproducible* clips.
        new_id: ClipId,
    },
}

/// One adjustable property of a clip (§59 "Basic transform", §20a gain).
///
/// A closed enum rather than a path-and-value pair, for the same reason as
/// [`SettingChange`]: the journal replays these after a crash, so every variant
/// that can be written has to be one the current build knows how to apply.
///
/// Values are plain numbers rather than the timeline's `Vec2`, keeping the wire
/// format independent of a type that exists for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "property", content = "value", rename_all = "snake_case")]
pub enum ClipProperty {
    /// 0.0–1.0. Video only.
    Opacity(f32),
    /// Linear gain, 0.0–4.0. Audio only.
    Gain(f32),
    /// Offset from centre in normalized output units. Video only.
    Position {
        x: f32,
        y: f32,
    },
    Scale {
        x: f32,
        y: f32,
    },
    Rotation(f32),
    /// §45's cheap colour adjustment. 1.0 leaves the picture alone.
    Brightness(f32),
    Contrast(f32),
    Saturation(f32),
    /// §45's blur, 0–100. Video only. A fraction of frame height rather than a
    /// pixel radius, so preview and export agree (§46).
    Blur(f32),
}

impl ClipProperty {
    /// The animatable parameters this property writes, with the values it is
    /// writing to them.
    ///
    /// Two entries for the pairs, one for the scalars, none for gain — audio is
    /// not animated in Milestone 8. This is what lets the same slider write a
    /// static value or a keyframe without the interface having two of every
    /// control.
    ///
    /// A fixed array rather than a `Vec`: this runs on every frame a slider is
    /// dragged, and there is nothing here worth an allocation.
    pub fn animated(self) -> [Option<(AnimatedParameter, f32)>; 2] {
        use AnimatedParameter as P;
        match self {
            Self::Opacity(v) => [Some((P::Opacity, v)), None],
            Self::Position { x, y } => [Some((P::PositionX, x)), Some((P::PositionY, y))],
            Self::Scale { x, y } => [Some((P::ScaleX, x)), Some((P::ScaleY, y))],
            Self::Rotation(v) => [Some((P::Rotation, v)), None],
            Self::Brightness(v) => [Some((P::Brightness, v)), None],
            Self::Contrast(v) => [Some((P::Contrast, v)), None],
            Self::Saturation(v) => [Some((P::Saturation, v)), None],
            Self::Blur(v) => [Some((P::Blur, v)), None],
            Self::Gain(_) => [None, None],
        }
    }

    /// Whether every parameter this property writes is at its default — that
    /// is, whether the control has been touched at all.
    ///
    /// Used to decide whether a reset button is worth offering. The values come
    /// from `AnimatedParameter::default_value`, so there is no second list to
    /// disagree with the one the model keeps.
    pub fn is_default(self) -> bool {
        self.animated()
            .into_iter()
            .flatten()
            .all(|(parameter, value)| parameter.is_default(value))
    }

    /// Shown in the undo menu, and used to decide whether two edits are the
    /// same gesture and should collapse into one history entry.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Opacity(_) => "Opacity",
            Self::Gain(_) => "Volume",
            Self::Position { .. } => "Position",
            Self::Scale { .. } => "Scale",
            Self::Rotation(_) => "Rotation",
            Self::Brightness(_) => "Brightness",
            Self::Contrast(_) => "Contrast",
            Self::Saturation(_) => "Saturation",
            Self::Blur(_) => "Blur",
        }
    }
}

/// A sequence's pixel dimensions, as the wire form of `Resolution`.
///
/// `Resolution` itself is serializable, but commands are a stable format that
/// the journal replays after a crash — going through a type declared here keeps
/// the wire shape from changing whenever the timeline crate's type does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionRepr {
    pub width: u32,
    pub height: u32,
}

impl From<bettercut_timeline::Resolution> for ResolutionRepr {
    fn from(value: bettercut_timeline::Resolution) -> Self {
        Self {
            width: value.width,
            height: value.height,
        }
    }
}

impl From<ResolutionRepr> for bettercut_timeline::Resolution {
    fn from(value: ResolutionRepr) -> Self {
        Self::new(value.width, value.height)
    }
}

/// `TrackKind` lives in the timeline crate and is not serializable there
/// (it is a runtime discriminator, not project data). This is its wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKindRepr {
    Video,
    Audio,
    /// §26's text overlays and §27's captions, which share a lane kind because
    /// a caption *is* a text clip — one with its timing read from a file.
    Text,
}

impl From<TrackKindRepr> for TrackKind {
    fn from(value: TrackKindRepr) -> Self {
        match value {
            TrackKindRepr::Video => Self::Video,
            TrackKindRepr::Audio => Self::Audio,
            TrackKindRepr::Text => Self::Text,
        }
    }
}

/// An executed, reversible operation (§10).
///
/// `undo` must restore exactly the state that existed before `execute`. A
/// command that cannot do that must fail in `execute` instead.
pub trait EditorCommand: std::fmt::Debug {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError>;
    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError>;

    /// Shown in the undo menu: "Undo Add Clip".
    fn label(&self) -> String;
}

/// Many commands, one undo step (§79).
///
/// Used by template application (§77), silence removal (§78), multi-delete, and
/// paste. Undo reverses the whole group, in reverse order.
#[derive(Debug, Default)]
pub struct CommandGroup {
    label: String,
    commands: Vec<Box<dyn EditorCommand>>,
    /// How many executed successfully, so a partial failure can be rolled back.
    executed: usize,
}

impl CommandGroup {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            commands: Vec::new(),
            executed: 0,
        }
    }

    pub fn push(&mut self, command: Box<dyn EditorCommand>) {
        self.commands.push(command);
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }
}

impl EditorCommand for CommandGroup {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        for (index, command) in self.commands.iter_mut().enumerate() {
            if let Err(err) = command.execute(project) {
                // Roll back what already succeeded, so a failed group leaves no
                // half-applied edit behind.
                for done in self.commands[..index].iter_mut().rev() {
                    if let Err(rollback) = done.undo(project) {
                        tracing::error!(?rollback, "rollback failed inside command group");
                    }
                }
                self.executed = 0;
                return Err(err);
            }
        }
        self.executed = self.commands.len();
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        for command in self.commands[..self.executed].iter_mut().rev() {
            command.undo(project)?;
        }
        self.executed = 0;
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaId, MediaTime};
    use bettercut_timeline::SourceRange;

    fn video_clip() -> VideoClip {
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(1)).expect("valid");
        VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).expect("valid")
    }

    /// §11/§38.2 — the journal depends on this.
    #[test]
    fn commands_round_trip_through_json() {
        let cmd = Command::AddClip {
            sequence: SequenceId::new(),
            track: TrackId::new(),
            clip: ClipPayload::Video(Box::new(video_clip())),
        };
        let json = serde_json::to_string(&cmd).expect("serialize");
        assert!(
            json.contains("\"op\":\"add_clip\""),
            "unexpected tag: {json}"
        );
        assert_eq!(
            serde_json::from_str::<Command>(&json).expect("deserialize"),
            cmd
        );
    }

    #[test]
    fn every_command_variant_serializes() {
        let seq = SequenceId::new();
        let track = TrackId::new();
        let variants = vec![
            Command::RenameProject {
                name: "x".to_owned(),
            },
            Command::AddTrack {
                sequence: seq,
                kind: TrackKindRepr::Audio,
                name: "A2".to_owned(),
                id: TrackId::new(),
            },
            Command::RemoveTrack {
                sequence: seq,
                track,
            },
            Command::SetTrackFlag {
                sequence: seq,
                track,
                flag: TrackFlag::Locked,
                value: true,
            },
            Command::RemoveClip {
                sequence: seq,
                track,
                clip: ClipId::new(),
            },
            // §38.2 replays these to rebuild an animation after a crash, so the
            // curve has to survive the wire as exactly as the value does.
            Command::SetKeyframe {
                sequence: seq,
                track,
                clip: ClipId::new(),
                parameter: AnimatedParameter::Opacity,
                key: Keyframe::new(
                    MediaTime::from_ticks(4800),
                    0.5,
                    bettercut_timeline::Interpolation::Bezier {
                        x1: 0.25,
                        y1: 0.1,
                        x2: 0.25,
                        y2: 1.0,
                    },
                ),
            },
            Command::RemoveKeyframe {
                sequence: seq,
                track,
                clip: ClipId::new(),
                parameter: AnimatedParameter::Blur,
                time: MediaTime::from_ticks(4800),
            },
        ];
        for cmd in variants {
            let json = serde_json::to_string(&cmd).expect("serialize");
            assert_eq!(
                serde_json::from_str::<Command>(&json).expect("deserialize"),
                cmd,
                "round trip failed for {cmd:?}"
            );
        }
    }

    #[test]
    fn clip_payload_reports_its_kind() {
        let payload = ClipPayload::Video(Box::new(video_clip()));
        assert_eq!(payload.kind(), TrackKind::Video);
        assert_eq!(payload.start(), TimelineTime::ZERO);
    }
}
