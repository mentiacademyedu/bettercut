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

use bettercut_foundation::{ClipId, SequenceId, TimelineTime, TrackId};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, TrackKind, VideoClip};
use serde::{Deserialize, Serialize};

use crate::error::EditorError;
pub use crate::ops::TrimEdge;

/// A clip of either kind, so commands can be written once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipPayload {
    Video(Box<VideoClip>),
    Audio(Box<AudioClip>),
}

impl ClipPayload {
    pub fn id(&self) -> ClipId {
        match self {
            Self::Video(c) => c.id,
            Self::Audio(c) => c.id,
        }
    }

    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
        }
    }

    pub fn start(&self) -> TimelineTime {
        match self {
            Self::Video(c) => c.timeline.start,
            Self::Audio(c) => c.timeline.start,
        }
    }
}

/// A whole track, held by `RemoveTrack` so undo can restore it intact.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackPayload {
    Video(Box<bettercut_timeline::VideoTrack>),
    Audio(Box<bettercut_timeline::AudioTrack>),
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

/// `TrackKind` lives in the timeline crate and is not serializable there
/// (it is a runtime discriminator, not project data). This is its wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKindRepr {
    Video,
    Audio,
}

impl From<TrackKindRepr> for TrackKind {
    fn from(value: TrackKindRepr) -> Self {
        match value {
            TrackKindRepr::Video => Self::Video,
            TrackKindRepr::Audio => Self::Audio,
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
