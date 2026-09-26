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

use bettercut_foundation::{ClipId, FrameRate, MediaId, SequenceId, TimelineTime, TrackId};
use bettercut_project_format::Project;
use bettercut_timeline::{AnimatedParameter, AudioClip, Keyframe, TrackKind, VideoClip};
use serde::{Deserialize, Serialize};

use crate::error::EditorError;
pub use crate::ops::TrimEdge;

/// What a clip reads, and how: the part of a clip a media replacement changes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MediaSwap {
    pub media: bettercut_foundation::MediaId,
    pub source: bettercut_timeline::SourceRange,
    pub speed: bettercut_foundation::Rational,
    pub reversed: bool,
    pub link: Option<bettercut_foundation::LinkId>,
}

/// A clip of either kind, so commands can be written once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipPayload {
    Video(Box<VideoClip>),
    Audio(Box<AudioClip>),
    /// §26's titles. Here because the operations that hand a whole clip
    /// around — split's undo, ripple delete's undo — apply to them too, and a
    /// separate payload type would mean writing those commands twice.
    Text(Box<bettercut_timeline::TextClip>),
    /// An adjustment (`bettercut_timeline::adjustment`), for the same reason a
    /// title is here: split's and ripple delete's undo hand the whole clip back.
    Adjustment(Box<bettercut_timeline::AdjustmentClip>),
}

impl ClipPayload {
    pub fn id(&self) -> ClipId {
        match self {
            Self::Video(c) => c.id,
            Self::Audio(c) => c.id,
            Self::Text(c) => c.id,
            Self::Adjustment(c) => c.id,
        }
    }

    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Text(_) => TrackKind::Text,
            Self::Adjustment(_) => TrackKind::Adjustment,
        }
    }

    pub fn start(&self) -> TimelineTime {
        match self {
            Self::Video(c) => c.timeline.start,
            Self::Audio(c) => c.timeline.start,
            Self::Text(c) => c.timeline.start,
            Self::Adjustment(c) => c.timeline.start,
        }
    }

    /// Where it ends — with [`Self::start`], how much room a paste needs.
    pub fn end(&self) -> TimelineTime {
        match self {
            Self::Video(c) => c.timeline.end,
            Self::Audio(c) => c.timeline.end,
            Self::Text(c) => c.timeline.end,
            Self::Adjustment(c) => c.timeline.end,
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

impl From<bettercut_timeline::AdjustmentClip> for ClipPayload {
    fn from(clip: bettercut_timeline::AdjustmentClip) -> Self {
        Self::Adjustment(Box::new(clip))
    }
}

/// A whole track: held by `RemoveTrack` so undo can restore it intact, and
/// carried by `InsertTrack` to put a prepared one in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrackPayload {
    Video(Box<bettercut_timeline::VideoTrack>),
    Audio(Box<bettercut_timeline::AudioTrack>),
    /// §26's text lanes, including §27's captions. A lane that can be created
    /// and not removed is a trap, and the caption import creates one.
    Text(Box<bettercut_timeline::TextTrack>),
    /// An adjustment lane, for the same reason: one that could be added and
    /// not removed would be a trap.
    Adjustment(Box<bettercut_timeline::AdjustmentTrack>),
}

/// Boolean track flags, so one command covers hide/mute/lock (§10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackFlag {
    /// Visible for video tracks, unmuted for audio tracks.
    Enabled,
    Locked,
    /// §20a.4: while any track in a lane is soloed, only those play.
    Solo,
    /// §10's sync lock: a ripple edit on another track moves this one's later
    /// clips too.
    SyncLock,
    /// §10's track targeting: new clips land here.
    Targeted,
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
    /// How long a photo runs when placed.
    PhotoLength(bettercut_foundation::MediaTime),
    /// Whether the main picture lane closes up by itself.
    MagneticTimeline(bool),
    /// How long a transition is when one is added.
    TransitionLength(bettercut_foundation::TimelineTime),
}

impl SettingChange {
    /// Shown in the undo menu.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::AutoGenerateProxies(_) => "Automatic Proxies",
            Self::PerformanceMode(_) => "Performance Mode",
            Self::CacheLimitBytes(_) => "Cache Limit",
            Self::PhotoLength(_) => "Photo Length",
            Self::TransitionLength(_) => "Transition Length",
            Self::MagneticTimeline(_) => "Magnetic Timeline",
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
    /// How the title arrives and leaves (§26). Both ends in one property: the
    /// panel edits them side by side, and each change is one undo step.
    Animation(bettercut_timeline::TextAnimation),
    /// Smear the title along the way it is moving, as a picture clip can be.
    MotionBlur(bool),
    /// The shape drawn in place of the text, or none for text again.
    Shape(Option<bettercut_text::Shape>),
    /// A counting number drawn in place of the text, or none for text again.
    Counter(Option<bettercut_timeline::Counter>),
    /// Each word lit up in this colour as it comes, or none for plain text.
    Highlight(Option<bettercut_text::Rgba>),
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
            | ClipProperty::Temperature(_)
            | ClipProperty::Tint(_)
            | ClipProperty::Vibrance(_)
            | ClipProperty::Anchor { .. }
            | ClipProperty::Wheels(_)
            | ClipProperty::Secondary(_)
            | ClipProperty::Blur(_)
            | ClipProperty::Backdrop(_)
            | ClipProperty::ChromaKey(_)
            | ClipProperty::LumaKey(_)
            | ClipProperty::Mask(_)
            | ClipProperty::Blend(_)
            | ClipProperty::MotionBlur(_)
            | ClipProperty::SmoothMotion(_)
            | ClipProperty::Border(_)
            | ClipProperty::Shadow(_)
            | ClipProperty::CornerPin(_)
            | ClipProperty::KeepPitch(_)
            | ClipProperty::Motion(_)
            | ClipProperty::Background(_)
            | ClipProperty::Flip { .. }
            | ClipProperty::Crop(_)
            | ClipProperty::Vignette(_)
            | ClipProperty::Grain(_)
            | ClipProperty::Bars(_)
            | ClipProperty::ProgressBar(_)
            | ClipProperty::BurnIn(_)
            | ClipProperty::Sharpen(_)
            | ClipProperty::Lut(_)
            | ClipProperty::Reverse(_)
            | ClipProperty::RgbSplit(_)
            | ClipProperty::Glitch(_)
            | ClipProperty::Pixelate(_)
            | ClipProperty::ZoomBlur(_)
            | ClipProperty::Lens(_)
            | ClipProperty::Posterise(_)
            | ClipProperty::SmoothSkin(_)
            | ClipProperty::TiltShift { .. }
            | ClipProperty::Glow(_)
            | ClipProperty::OldFilm(_)
            | ClipProperty::Tone(_)
            | ClipProperty::Curves(_)
            | ClipProperty::LightLeak(_)
            | ClipProperty::LensFlare(_)
            | ClipProperty::BeatPulse(_)
            | ClipProperty::Reflection(_)
            | ClipProperty::Denoise(_)
            | ClipProperty::Gate(_)
            | ClipProperty::Eq(_)
            | ClipProperty::Space(_)
            | ClipProperty::Channels(_)
            | ClipProperty::Pitch(_)
            | ClipProperty::Leveller(_)
            | ClipProperty::DeEss(_)
            | ClipProperty::Robot(_)
            | ClipProperty::StereoWidth(_)
            | ClipProperty::FadeShape(_)
            | ClipProperty::Mute(_)
            | ClipProperty::Crossfade(_)
            | ClipProperty::Pan(_)
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
            Self::Animation(_) => "text animation",
            Self::MotionBlur(_) => "text motion blur",
            Self::Shape(_) => "shape",
            Self::Counter(_) => "timer",
            Self::Highlight(_) => "word highlight",
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
    /// Replace the project's notes (`Project::notes`).
    SetProjectNotes {
        notes: String,
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
    /// Replace a sound clip's whole volume envelope (§24 on §20a.4's
    /// clip-gain stage).
    ///
    /// One command rather than a key at a time, because an envelope is one
    /// shape: ducking writes it, dragging a point moves one within it, and a
    /// drag has to collapse into a single undo step the way every other
    /// gesture does (§11).
    SetGainEnvelope {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        keys: Vec<Keyframe>,
    },
    /// Replace a sound clip's whole pan envelope: the volume's twin, one
    /// stage over — where the sound sits between the speakers rather than
    /// how loud it is. One command for the whole shape, for the same reason.
    SetPanEnvelope {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        keys: Vec<Keyframe>,
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
    /// Add an adjustment clip. Carries the whole clip, id included, for the
    /// reason `AddText` does: a replay must produce the same project.
    AddAdjustment {
        sequence: SequenceId,
        track: TrackId,
        clip: Box<bettercut_timeline::AdjustmentClip>,
    },
    RemoveAdjustment {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
    },
    /// Set how an adjustment grades what is beneath it.
    ///
    /// The whole look at once rather than a property each, as a title's style
    /// is: the panel edits it as one thing and the renderer takes it as one.
    /// A slider drag still collapses into one undo step, because drags collapse
    /// on the history label and this one's never changes.
    SetAdjustmentLook {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        look: bettercut_timeline::AdjustmentLook,
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
    /// Change how fast a clip plays.
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
    /// Replace a sequence's markers.
    ///
    /// The whole list rather than one at a time: beat detection adds hundreds
    /// in one go, and one command is one undo step. Normalized on the way in
    /// (sorted, one per instant), so the journal cannot replay a bad list.
    SetMarkers {
        sequence: SequenceId,
        markers: Vec<bettercut_timeline::Marker>,
    },
    /// Replace a sequence's clip marks — the whole list, so adding one and
    /// clearing a clip's are each one undo step (`ClipMark`).
    SetClipMarks {
        sequence: SequenceId,
        marks: Vec<bettercut_timeline::ClipMark>,
    },
    /// Leave a note on a clip, or take it off with an empty `text`.
    SetClipNote {
        sequence: SequenceId,
        clip: ClipId,
        text: String,
    },
    /// Replace a sequence's clip groups — the whole list, so grouping and
    /// ungrouping are each one undo step however many groups they touch.
    SetGroups {
        sequence: SequenceId,
        groups: Vec<Vec<ClipId>>,
    },
    /// Set both marks at once, either possibly clear.
    SetInOut {
        sequence: SequenceId,
        mark_in: Option<bettercut_foundation::TimelineTime>,
        mark_out: Option<bettercut_foundation::TimelineTime>,
    },
    /// An audio track's volume line: the lane's level along the timeline
    /// (`bettercut_timeline::track_volume`).
    ///
    /// The whole line at once, as a clip's envelope is written: a drag moves
    /// one point in a copy and writes the copy, and a command per point would
    /// make undo replay a drag one point at a time.
    SetTrackVolume {
        sequence: SequenceId,
        track: TrackId,
        points: Vec<bettercut_timeline::VolumePoint>,
    },
    /// An audio track's volume and pan (§20a.4's track stage).
    SetTrackMix {
        sequence: SequenceId,
        track: TrackId,
        gain: f32,
        pan: f32,
    },
    /// An audio track's equaliser: the clip's three controls and hum filter,
    /// over the whole lane.
    SetTrackEq {
        sequence: SequenceId,
        track: TrackId,
        eq: bettercut_timeline::ClipEq,
    },
    /// Set how a sound clip fades in and out. Audio clips only.
    ///
    /// Both ends in one command, because the panel shows them side by side and
    /// a change to either is one thing the user did.
    /// Give a clip a name of its own, or `None` for the file's. Any lane.
    SetClipName {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        name: Option<String>,
    },
    /// Tag a clip with a colour (`ColorLabel`). Any lane.
    SetColorLabel {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        label: bettercut_timeline::ColorLabel,
    },
    /// Solo one clip: while any clip in its kind of lane is soloed, only the
    /// soloed ones are seen or heard (`Sequence::soloed_clips`).
    SetClipSolo {
        sequence: SequenceId,
        clip: ClipId,
        solo: bool,
    },
    SetClipFades {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        fade_in: bettercut_foundation::TimelineTime,
        fade_out: bettercut_foundation::TimelineTime,
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
    /// Decode a file's fields woven into whole frames, or as it comes
    /// (`MediaAsset::deinterlace`).
    SetMediaDeinterlace {
        media: MediaId,
        deinterlace: bool,
    },
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
    /// Choose the sequence's cover frame, or clear it with `None`.
    SetCoverFrame {
        sequence: SequenceId,
        at: Option<TimelineTime>,
    },
    /// Put a watermark on the sequence, or take it off with `None`.
    SetWatermark {
        sequence: SequenceId,
        watermark: Option<bettercut_timeline::watermark::Watermark>,
    },
    /// Put an audio visualizer over the sequence, or take it off with `None`.
    SetVisualizer {
        sequence: SequenceId,
        visualizer: Option<Box<bettercut_timeline::visualizer::Visualizer>>,
    },
    SetSequenceFormat {
        sequence: SequenceId,
        resolution: ResolutionRepr,
        frame_rate: FrameRate,
    },
    /// What a sequence's first frame is called (`Sequence::start_timecode`).
    SetStartTimecode {
        sequence: SequenceId,
        start: TimelineTime,
    },
    /// Put a whole prepared sequence into the project at `index`. What adding
    /// and duplicating a sequence dispatch; the ids inside are fixed in the
    /// request, so a replay makes the same sequence.
    AddSequence {
        sequence: Box<bettercut_timeline::Sequence>,
        index: usize,
    },
    /// Take a sequence out of the project. Refused for the last one.
    RemoveSequence {
        sequence: SequenceId,
    },
    RenameSequence {
        sequence: SequenceId,
        name: String,
    },
    /// Give a file in the project a name of its own, or `None` to show its
    /// file name again.
    RenameMedia {
        media: MediaId,
        name: Option<String>,
    },
    /// Change what a colour clip's generated media draws.
    SetColour {
        media: MediaId,
        colour: bettercut_media::Generated,
    },
    /// File a file in a bin, or take it out of one with `None`.
    /// Rate a file 0–5 in the media browser (`Editor::set_media_rating`).
    SetMediaRating {
        media: bettercut_foundation::MediaId,
        rating: u8,
    },
    SetMediaBin {
        media: MediaId,
        bin: Option<String>,
    },
    /// Give a track a new name.
    RenameTrack {
        sequence: SequenceId,
        track: TrackId,
        name: String,
    },
    /// Move a track to `index` among the tracks of its kind, everything on
    /// it and about it coming along. For picture tracks the order is what is
    /// drawn over what.
    MoveTrack {
        sequence: SequenceId,
        track: TrackId,
        index: usize,
    },
    /// Put a whole prepared track — clips and all — in at `index` among the
    /// tracks of its kind. What duplicating a track dispatches; the ids inside
    /// are fixed in the request, so a replay makes the same track.
    InsertTrack {
        sequence: SequenceId,
        index: usize,
        track: TrackPayload,
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
    /// A lane's colour label (`Track::color_label`), any kind of lane.
    SetTrackColour {
        sequence: SequenceId,
        track: TrackId,
        label: bettercut_timeline::ColorLabel,
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
    /// Point a picture or sound clip at other media, keeping everything else
    /// about it (`crate::replace`). Its span on the timeline does not change.
    ReplaceClipMedia {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        swap: MediaSwap,
    },
    MoveClip {
        sequence: SequenceId,
        from_track: TrackId,
        to_track: TrackId,
        clip: ClipId,
        new_start: TimelineTime,
    },
    /// Move a *sound* clip to an exact tick, off the frame grid: a
    /// millisecond of sync. `MoveClip` snaps to frames, which is right for
    /// picture and wrong for this.
    NudgeSound {
        sequence: SequenceId,
        track: TrackId,
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
    /// Choose which angle of a multicam clip is on screen, or `None` for all
    /// of them at once (`crate::multicam`).
    SetClipAngle {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        angle: Option<usize>,
    },
    /// Play a different stretch of the clip's file without moving the clip:
    /// its in and out points both move `offset` ticks of source (later when
    /// positive), held inside the file (`crate::slip`).
    SlipClip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
        offset: i64,
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
    /// White balance, -1..1, zero for no change (§45).
    Temperature(f32),
    Tint(f32),
    /// The pivot: where in its own frame a picture turns and scales about,
    /// 0..1 each way, the middle by default. Video only, not animated: a
    /// pivot that moved would make the same rotation land somewhere else
    /// every frame.
    Anchor {
        x: f32,
        y: f32,
    },
    /// Vibrance, -1..1, zero for no change: saturation weighted towards the
    /// colours that have least of it (§45).
    Vibrance(f32),
    /// The colour wheels — lift, gamma and gain — as one setting, because a
    /// grade is one shape: a wheel dragged is the whole set changing.
    Wheels(bettercut_timeline::ColorWheels),
    /// One range of hue moved on its own, as one setting: the pick and its
    /// shifts are one thought.
    Secondary(bettercut_timeline::HslSecondary),
    /// §45's blur, 0–100. Video only. A fraction of frame height rather than a
    /// pixel radius, so preview and export agree (§46).
    Blur(f32),
    /// What fills the frame around a clip that does not cover it (§36). Video
    /// only, and not animatable: it is a choice, not a dial.
    Backdrop(bettercut_timeline::Backdrop),
    /// The chroma key, or `None` to take it off. Video only, and not
    /// animatable — it is a fact about the footage.
    ChromaKey(Option<bettercut_timeline::ChromaKey>),
    /// A key by brightness, or none. Video only.
    LumaKey(Option<bettercut_timeline::LumaKey>),
    /// The mask, or `None` to take it off. Video only.
    Mask(Option<bettercut_timeline::Mask>),
    /// §22: how a clip combines with what is beneath it. Video only.
    Blend(bettercut_timeline::BlendMode),
    /// How the shot arrives and leaves. Video only, and not animatable —
    /// it *is* an animation, and keyframing one would be two movements over
    /// the same instants arguing with each other.
    Motion(bettercut_timeline::ClipMotion),
    /// Smear a moving shot along its path. Video only, and not animatable —
    /// it is a property of the shot, not a dial to ride.
    MotionBlur(bool),
    /// Smooth slow motion on a picture clip.
    SmoothMotion(bool),
    /// Rounded corners and a border on a picture clip.
    Border(bettercut_timeline::Border),
    /// Hold a sound clip's pitch where it is when its speed changes.
    KeepPitch(bool),
    /// A drop shadow under a picture clip.
    Shadow(bettercut_timeline::Shadow),
    /// §45's corner pin: the four corners moved, to set a picture into a
    /// screen or a wall in the shot beneath it.
    CornerPin(bettercut_timeline::CornerPin),
    /// §22's crop: how much of the source is thrown away before anything else.
    /// Video only, and not animatable — a crop that moved through a shot is a
    /// pan, and a pan is the transform's job.
    Crop(bettercut_timeline::Crop),
    /// Mirror the picture. Video only, and not animatable: there is no halfway
    /// between a shot and its reflection to interpolate through.
    Flip {
        axis: bettercut_timeline::FlipAxis,
        on: bool,
    },
    /// §22: what shows where no picture does. The mirror image of the video
    /// properties above — this one belongs to the sequence and a clip has no
    /// use for it, because a clip *is* a picture.
    Background([f32; 3]),
    /// How much the edges of the whole frame are darkened, 0–1. The sequence's
    /// alone, for the reason `MasterLook::vignette` gives: a vignette frames the
    /// frame, and on a clip in a corner it would frame the wrong thing.
    Vignette(f32),
    /// Film grain over the whole frame, 0–1. The sequence's alone, like the
    /// vignette: grain is the film the whole picture is on.
    Grain(f32),
    /// Cinematic bars over the whole frame, as the shape they cut to (zero
    /// for none). The sequence's alone.
    Bars(f32),
    /// A progress bar over the whole video. The sequence's alone.
    ProgressBar(bettercut_timeline::ProgressBar),
    /// The timecode and file name over the picture, for a copy sent out for
    /// notes (`timeline::burn_in`). Whole video only.
    BurnIn(bettercut_timeline::BurnIn),
    /// Sharpening, 0–100 (`bettercut_timeline::MAX_SHARPEN`). Video only, and
    /// not animatable: a property of the shot rather than a dial to ride.
    Sharpen(f32),
    /// A colour lookup table and its strength (`bettercut_timeline::lut`), or
    /// none. Video only: a LUT grades footage.
    Lut(Option<bettercut_timeline::ClipLut>),
    /// Play backwards. Picture and sound both take it, and
    /// `Editor::set_reversed` applies it to a linked pair together (§12).
    Reverse(bool),
    /// RGB split, 0–100. Video only, not animated.
    RgbSplit(f32),
    /// Glitch, 0–100. Video only, not animated.
    Glitch(f32),
    /// Pixelate, 0–100. Picture only.
    Pixelate(f32),
    /// Zoom blur, 0–100. Picture only.
    ZoomBlur(f32),
    /// Lens correction, -1..1: negative bows the edges in, positive pushes
    /// them out; zero is the lens as it was. Video only.
    Lens(f32),
    /// Posterise levels, 2–16; below 2 is off. Video only.
    Posterise(f32),
    /// Smooth skin, 0-1. Video only.
    SmoothSkin(f32),
    /// Tilt-shift: the sharp band's height (0–1, 0 for none) and centre
    /// (0–1), with the blur growing away from it. Video only.
    TiltShift {
        band: f32,
        centre: f32,
    },
    /// Glow, 0–100. Video only, not animated.
    Glow(f32),
    /// Old film, 0–100. Video only, not animated.
    OldFilm(f32),
    /// A colour tone — sepia or a duotone — on a picture clip. Video only.
    Tone(bettercut_timeline::curves::Tone),
    /// Colour curves. Picture only, not animated.
    Curves(bettercut_timeline::curves::ColourCurves),
    /// Light leak, 0–100. Picture only.
    LightLeak(f32),
    /// Lens flare, 0–100. Picture only.
    LensFlare(f32),
    /// Beat pulse, 0–100. Picture only.
    BeatPulse(f32),
    /// Mirrored halves, four-way or a kaleidoscope. Video only, not animated.
    Reflection(bettercut_timeline::Reflection),
    /// Voice clean-up on a sound clip, 0–100. Sound only, not animated.
    Denoise(f32),
    /// The noise gate on a sound clip, 0–100. Sound only, not animated.
    Gate(f32),
    /// A sound clip's low cut, high cut and presence. Sound only.
    Eq(bettercut_timeline::ClipEq),
    /// A sound clip's echo or reverb. Sound only.
    Space(bettercut_timeline::ClipSpace),
    /// Which of a sound clip's channels play where. Sound only.
    Channels(bettercut_timeline::ChannelMode),
    /// A sound clip's pitch, in semitones, without a change of speed. Sound
    /// only.
    Pitch(f32),
    /// A sound clip's leveller, 0–100. Sound only.
    Leveller(f32),
    /// A sound clip's de-esser, 0–100. Sound only.
    DeEss(f32),
    /// A sound clip's robot voice, 0–100. Sound only.
    Robot(f32),
    /// How wide a sound clip's stereo image is, 0–2, one for as recorded.
    /// Sound only.
    StereoWidth(f32),
    /// Where a sound clip sits between the speakers, -1 to +1, on top of its
    /// lane's pan. Sound only. Its keyed form is the pan envelope.
    Pan(f32),
    /// The shape of a sound clip's fades. Sound only.
    FadeShape(bettercut_timeline::FadeShape),
    /// A sound clip silenced in place. Sound only.
    Mute(bool),
    /// How long a sound clip crossfades into the next. Sound only; set through
    /// `Editor::set_audio_crossfade`, which holds it to the material there is.
    Crossfade(TimelineTime),
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
            Self::Temperature(v) => [Some((P::Temperature, v)), None],
            Self::Tint(v) => [Some((P::Tint, v)), None],
            // No animated parameter behind it: vibrance is set, not keyed.
            Self::Vibrance(_)
            | Self::Wheels(_)
            | Self::Anchor { .. }
            | Self::Secondary(_)
            | Self::CornerPin(_)
            | Self::KeepPitch(_) => [None, None],
            Self::Blur(v) => [Some((P::Blur, v)), None],
            Self::Backdrop(_)
            | Self::ChromaKey(_)
            | Self::LumaKey(_)
            | Self::Mask(_)
            | Self::Blend(_)
            | Self::Motion(_)
            | Self::Background(_)
            | Self::MotionBlur(_)
            | Self::SmoothMotion(_)
            | Self::Border(_)
            | Self::Shadow(_)
            | Self::Flip { .. }
            | Self::Crop(_)
            | Self::Vignette(_)
            | Self::Grain(_)
            | Self::Bars(_)
            | Self::ProgressBar(_)
            | Self::BurnIn(_)
            | Self::Sharpen(_)
            | Self::Lut(_)
            | Self::Reverse(_)
            | Self::RgbSplit(_)
            | Self::Glitch(_)
            | Self::Pixelate(_)
            | Self::ZoomBlur(_)
            | Self::Lens(_)
            | Self::Posterise(_)
            | Self::SmoothSkin(_)
            | Self::TiltShift { .. }
            | Self::Glow(_)
            | Self::OldFilm(_)
            | Self::Tone(_)
            | Self::Curves(_)
            | Self::LightLeak(_)
            | Self::LensFlare(_)
            | Self::BeatPulse(_)
            | Self::Reflection(_)
            | Self::Denoise(_)
            | Self::Gate(_)
            | Self::Eq(_)
            | Self::Space(_)
            | Self::Channels(_)
            | Self::Pitch(_)
            | Self::Leveller(_)
            | Self::DeEss(_)
            | Self::Robot(_)
            | Self::StereoWidth(_)
            | Self::FadeShape(_)
            | Self::Mute(_)
            | Self::Crossfade(_)
            | Self::Pan(_)
            | Self::Gain(_) => [None, None],
        }
    }

    /// Whether every parameter this property writes is at its default — that
    /// is, whether the control has been touched at all.
    ///
    /// Used to decide whether a reset button is worth offering. The values come
    /// from `AnimatedParameter::default_value`, so there is no second list to
    /// disagree with the one the model keeps.
    pub fn is_default(self) -> bool {
        // Gain is not an animated parameter (§24 animates picture only), so it
        // has no entry to compare against — and `all` over nothing is true.
        // Asked here directly, or every volume reset button is permanently
        // greyed out whatever the volume is.
        if let Self::Pan(pan) = self {
            return pan == 0.0;
        }
        if let Self::Gain(value) = self {
            return (value - 1.0).abs() < 1e-6;
        }
        // And the same for the background, for the same reason: it has no
        // animated parameter to compare against, so falling through would
        // report every colour as the default one and leave its reset button
        // greyed out however far from black it was.
        if let Self::Background(colour) = self {
            return colour.iter().all(|channel| *channel == 0.0);
        }
        // And again: no animated parameter behind it, so without this every
        // vignette reads as untouched and its reset stays greyed out.
        if let Self::Vignette(amount) = self {
            return amount == 0.0;
        }
        if let Self::Border(border) = self {
            return border.is_none();
        }
        if let Self::TiltShift { band, .. } = self {
            return band <= 0.0;
        }
        if let Self::LumaKey(key) = self {
            return key.is_none();
        }
        if let Self::Shadow(shadow) = self {
            return !shadow.is_visible();
        }
        if let Self::CornerPin(pin) = self {
            return pin.is_none();
        }
        if let Self::KeepPitch(on) = self {
            return !on;
        }
        // Nor behind grain.
        if let Self::Grain(amount) = self {
            return amount == 0.0;
        }
        if let Self::Bars(shape) = self {
            return shape == 0.0;
        }
        if let Self::BurnIn(burn) = self {
            return !burn.is_visible();
        }
        if let Self::ProgressBar(bar) = self {
            return !bar.is_visible();
        }
        // The same trap once more: no animated parameter behind it.
        if let Self::Sharpen(amount) = self {
            return amount == 0.0;
        }
        if let Self::Lut(lut) = self {
            return lut.is_none();
        }
        // Nor behind vibrance, so without this it always reads as untouched.
        if let Self::Vibrance(amount) = self {
            return amount == 0.0;
        }
        if let Self::Wheels(wheels) = self {
            return wheels.is_identity();
        }
        if let Self::Secondary(secondary) = self {
            return secondary.is_identity();
        }
        if let Self::Anchor { x, y } = self {
            return (x - 0.5).abs() < 1e-6 && (y - 0.5).abs() < 1e-6;
        }
        if let Self::Curves(curves) = self {
            return curves.is_identity();
        }
        if let Self::Reverse(on) = self {
            return !on;
        }
        if let Self::Channels(mode) = self {
            return mode == bettercut_timeline::ChannelMode::Stereo;
        }
        if let Self::Pitch(semitones) = self {
            return semitones == 0.0;
        }
        if let Self::Leveller(amount) = self {
            return amount == 0.0;
        }
        if let Self::DeEss(amount) = self {
            return amount == 0.0;
        }
        if let Self::Robot(amount) = self {
            return amount == 0.0;
        }
        if let Self::StereoWidth(width) = self {
            return width == 1.0;
        }
        if let Self::FadeShape(shape) = self {
            return shape == bettercut_timeline::FadeShape::Smooth;
        }
        if let Self::Mute(muted) = self {
            return !muted;
        }
        if let Self::Tone(tone) = self {
            return tone.is_none();
        }
        if let Self::RgbSplit(amount)
        | Self::Glitch(amount)
        | Self::Pixelate(amount)
        | Self::ZoomBlur(amount)
        | Self::Lens(amount)
        | Self::Posterise(amount)
        | Self::SmoothSkin(amount)
        | Self::Glow(amount)
        | Self::OldFilm(amount)
        | Self::LightLeak(amount)
        | Self::LensFlare(amount)
        | Self::BeatPulse(amount)
        | Self::Denoise(amount)
        | Self::Gate(amount) = self
        {
            return amount == 0.0;
        }
        if let Self::Eq(eq) = self {
            return eq.is_flat();
        }
        if let Self::Space(space) = self {
            return space.is_dry();
        }
        if let Self::Crossfade(length) = self {
            return length <= TimelineTime::ZERO;
        }
        if let Self::Reflection(kind) = self {
            return matches!(kind, bettercut_timeline::Reflection::None);
        }
        if let Self::MotionBlur(on) | Self::SmoothMotion(on) = self {
            return !on;
        }
        // And the same again: a mirror is its own control with no animated
        // parameter behind it, so falling through would call every flipped clip
        // untouched and leave its reset greyed out.
        if let Self::Flip { on, .. } = self {
            return !on;
        }
        // And once more for the crop, which has no animated parameter either:
        // without this every cropped clip reports itself untouched and its
        // reset button stays greyed out however much was taken off.
        if let Self::Crop(crop) = self {
            return crop.is_none();
        }
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
            Self::Pan(_) => "Pan",
            Self::Gain(_) => "Volume",
            Self::Position { .. } => "Position",
            Self::Scale { .. } => "Scale",
            Self::Rotation(_) => "Rotation",
            Self::Brightness(_) => "Brightness",
            Self::Contrast(_) => "Contrast",
            Self::Saturation(_) => "Saturation",
            Self::Temperature(_) => "Temperature",
            Self::Tint(_) => "Tint",
            Self::Vibrance(_) => "Vibrance",
            Self::Wheels(_) => "Colour Wheels",
            Self::Anchor { .. } => "Pivot",
            Self::Secondary(_) => "Secondary Colour",
            Self::Blur(_) => "Blur",
            Self::Backdrop(_) => "Backdrop",
            Self::ChromaKey(_) => "Chroma Key",
            Self::LumaKey(_) => "Luma Key",
            Self::Mask(_) => "Mask",
            Self::Blend(_) => "Blend",
            Self::Motion(_) => "Animation",
            Self::Background(_) => "Background",
            Self::MotionBlur(_) => "Motion Blur",
            Self::SmoothMotion(_) => "Smooth Slow Motion",
            Self::Border(_) => "Corners and Border",
            Self::Shadow(_) => "Shadow",
            Self::CornerPin(_) => "Corner Pin",
            Self::KeepPitch(_) => "Keep Pitch",
            Self::Crop(_) => "Crop",
            Self::Vignette(_) => "Vignette",
            Self::Grain(_) => "Grain",
            Self::Bars(_) => "Cinematic Bars",
            Self::ProgressBar(_) => "Progress Bar",
            Self::BurnIn(_) => "Burn-in",
            Self::Sharpen(_) => "Sharpen",
            Self::Lut(_) => "LUT",
            Self::Reverse(_) => "Reverse",
            Self::RgbSplit(_) => "RGB split",
            Self::Glitch(_) => "Glitch",
            Self::Pixelate(_) => "Pixelate",
            Self::ZoomBlur(_) => "Zoom blur",
            Self::Lens(_) => "Lens",
            Self::Posterise(_) => "Posterise",
            Self::SmoothSkin(_) => "Smooth Skin",
            Self::TiltShift { .. } => "Tilt-shift",
            Self::Glow(_) => "Glow",
            Self::OldFilm(_) => "Old film",
            Self::Tone(_) => "Tone",
            Self::Curves(_) => "Curves",
            Self::LightLeak(_) => "Light leak",
            Self::LensFlare(_) => "Lens flare",
            Self::BeatPulse(_) => "Beat pulse",
            Self::Reflection(_) => "Mirror",
            Self::Denoise(_) => "Voice clean-up",
            Self::Gate(_) => "Noise gate",
            Self::Eq(_) => "EQ",
            Self::Space(_) => "Echo & reverb",
            Self::Channels(_) => "Channels",
            Self::Pitch(_) => "Voice pitch",
            Self::Leveller(_) => "Leveller",
            Self::DeEss(_) => "De-esser",
            Self::Robot(_) => "Robot voice",
            Self::StereoWidth(_) => "Stereo Width",
            Self::FadeShape(_) => "Fade shape",
            Self::Mute(_) => "Mute",
            Self::Crossfade(_) => "Crossfade",
            // The axis is in the name so that mirroring one way and then the
            // other is two undo steps rather than one collapsed gesture.
            Self::Flip { axis, .. } => axis.label(),
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
    /// Adjustment lanes: grades over a stretch of the edit.
    Adjustment,
}

impl From<TrackKindRepr> for TrackKind {
    fn from(value: TrackKindRepr) -> Self {
        match value {
            TrackKindRepr::Video => Self::Video,
            TrackKindRepr::Audio => Self::Audio,
            TrackKindRepr::Text => Self::Text,
            TrackKindRepr::Adjustment => Self::Adjustment,
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

    /// A group of commands that have all run already, so undoing it undoes
    /// them — how a step is extended after the fact.
    pub(crate) fn already_run(
        label: impl Into<String>,
        commands: Vec<Box<dyn EditorCommand>>,
    ) -> Self {
        let executed = commands.len();
        Self {
            label: label.into(),
            commands,
            executed,
        }
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
            Command::SetProjectNotes {
                notes: "fix the colour on shot 3".to_owned(),
            },
            Command::SetPanEnvelope {
                sequence: seq,
                track,
                clip: ClipId::new(),
                keys: vec![],
            },
            Command::SetStartTimecode {
                sequence: seq,
                start: TimelineTime::from_seconds(3_600),
            },
            Command::SetClipSolo {
                sequence: seq,
                clip: ClipId::new(),
                solo: true,
            },
            Command::NudgeSound {
                sequence: seq,
                track,
                clip: ClipId::new(),
                new_start: TimelineTime::from_ticks(960),
            },
            Command::SetMediaDeinterlace {
                media: MediaId::new(),
                deinterlace: true,
            },
            Command::SetClipName {
                sequence: seq,
                track,
                clip: ClipId::new(),
                name: Some("interview wide".to_owned()),
            },
            Command::SetClipMarks {
                sequence: seq,
                marks: vec![bettercut_timeline::ClipMark {
                    clip: ClipId::new(),
                    source: bettercut_foundation::MediaTime::from_seconds(2),
                    label: "door opens".to_owned(),
                    color: bettercut_timeline::ColorLabel::None,
                }],
            },
            Command::AddTrack {
                sequence: seq,
                kind: TrackKindRepr::Audio,
                name: "A2".to_owned(),
                id: TrackId::new(),
            },
            Command::InsertTrack {
                sequence: seq,
                index: 1,
                track: TrackPayload::Audio(Box::new(bettercut_timeline::AudioTrack::new(
                    "A1 copy",
                ))),
            },
            Command::RemoveTrack {
                sequence: seq,
                track,
            },
            Command::RenameTrack {
                sequence: seq,
                track,
                name: "Voice".to_owned(),
            },
            Command::MoveTrack {
                sequence: seq,
                track,
                index: 0,
            },
            Command::SetCoverFrame {
                sequence: seq,
                at: Some(TimelineTime::from_seconds(3)),
            },
            Command::SetWatermark {
                sequence: seq,
                watermark: Some(bettercut_timeline::watermark::Watermark::new(MediaId::new())),
            },
            Command::SetVisualizer {
                sequence: seq,
                visualizer: Some(Box::new(bettercut_timeline::visualizer::Visualizer::new(
                    vec![0.0, 0.5, 1.0],
                    TimelineTime::from_seconds(1),
                ))),
            },
            Command::RenameMedia {
                media: MediaId::new(),
                name: Some("Interview, wide".to_owned()),
            },
            Command::SetMediaBin {
                media: MediaId::new(),
                bin: Some("B-roll".to_owned()),
            },
            Command::SetColour {
                media: MediaId::new(),
                colour: bettercut_media::Generated::Colour {
                    top: [255, 0, 0],
                    bottom: [0, 0, 255],
                },
            },
            Command::AddSequence {
                sequence: Box::new(bettercut_timeline::Sequence::default_hd()),
                index: 1,
            },
            Command::RemoveSequence { sequence: seq },
            Command::RenameSequence {
                sequence: seq,
                name: "Vertical cut".to_owned(),
            },
            Command::SetTrackFlag {
                sequence: seq,
                track,
                flag: TrackFlag::Locked,
                value: true,
            },
            Command::SetTrackVolume {
                sequence: seq,
                track,
                points: vec![bettercut_timeline::VolumePoint::new(
                    bettercut_foundation::TimelineTime::from_seconds(2),
                    0.5,
                )],
            },
            Command::RemoveClip {
                sequence: seq,
                track,
                clip: ClipId::new(),
            },
            Command::SetGroups {
                sequence: seq,
                groups: vec![vec![ClipId::new(), ClipId::new()]],
            },
            Command::SetClipNote {
                sequence: seq,
                clip: ClipId::new(),
                text: "swap for take 3".to_owned(),
            },
            Command::ReplaceClipMedia {
                sequence: seq,
                track,
                clip: ClipId::new(),
                swap: MediaSwap {
                    media: bettercut_foundation::MediaId::new(),
                    source: bettercut_timeline::SourceRange::new(
                        bettercut_foundation::MediaTime::ZERO,
                        bettercut_foundation::MediaTime::from_seconds(2),
                    )
                    .expect("valid"),
                    speed: bettercut_foundation::Rational::ONE,
                    reversed: true,
                    link: Some(bettercut_foundation::LinkId::new()),
                },
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
            Command::SlipClip {
                sequence: seq,
                track,
                clip: ClipId::new(),
                offset: -48_000,
            },
            Command::SetClipAngle {
                sequence: seq,
                track,
                clip: ClipId::new(),
                angle: Some(2),
            },
            Command::SetMediaRating {
                media: bettercut_foundation::MediaId::new(),
                rating: 4,
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
