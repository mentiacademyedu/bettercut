//! One change, many clips: the inspector acting on the whole selection.
//!
//! Ten shots from one camera want the same grade, the same crop, the same
//! volume. Doing it ten times is the work a selection exists to save — and
//! doing it as ten undo steps is worse than doing it once. So a property set
//! on several clips is one gesture: a drag across a slider coalesces into one
//! step however many clips it moved.
//!
//! A clip whose parameter is keyframed is left out: its value lives in its
//! keys, and a flat number written over it would be a number the renderer
//! never reads.

use bettercut_foundation::ClipId;

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Whether `clip` animates any parameter `property` sets.
    pub fn animates(&self, clip: ClipId, property: ClipProperty) -> bool {
        self.video_clip(clip).is_some_and(|video| {
            property
                .animated()
                .into_iter()
                .flatten()
                .any(|(parameter, _)| video.keyframes.is_animated(parameter))
        })
    }

    /// Set `property` on every clip in `clips` that can take it, as one
    /// gesture (coalescing while `continuing`). Keyframed clips, and clips
    /// the property does not belong to, are passed over. Returns how many
    /// took it.
    pub fn set_clips_property(
        &mut self,
        clips: &[ClipId],
        property: ClipProperty,
        continuing: bool,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let commands: Vec<Command> = clips
            .iter()
            .filter(|clip| !self.animates(**clip, property))
            .filter_map(|clip| {
                Some(Command::SetClipProperty {
                    sequence,
                    track: self.track_of(*clip)?,
                    clip: *clip,
                    property,
                })
            })
            .collect();
        if commands.is_empty() {
            return Ok(0);
        }
        let count = commands.len();
        self.dispatch_gesture(
            format!("Change {} on {count} Clips", property.kind()),
            commands,
            continuing,
        )?;
        Ok(count)
    }
}
