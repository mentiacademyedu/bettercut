//! Quarter turns: a sideways phone shot put upright in one click.

use bettercut_foundation::ClipId;

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Turn every picture clip in `clips` a quarter turn — clockwise for a
    /// positive `quarters`, anticlockwise for a negative one — as one undo
    /// step. Returns how many clips turned.
    ///
    /// Added to whatever rotation each clip already has, and brought back into
    /// the −180°–180° the rotation control shows, so four turns right is where
    /// the clip started. On a clip whose rotation is keyframed the turn lands
    /// as a key at the playhead, as any rotation change does. Titles and
    /// sounds in the selection are left alone.
    pub fn rotate_quarter(
        &mut self,
        clips: &[ClipId],
        quarters: i32,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let turn = (quarters.rem_euclid(4) * 90) as f32;
        let mut commands = Vec::new();
        for clip in clips {
            let Some(video) = self.video_clip(*clip) else {
                continue;
            };
            let track = self
                .track_of(*clip)
                .ok_or(EditorError::ClipNotFound(*clip))?;
            let degrees = normalised(video.transform.rotation_degrees + turn);
            commands.push(Command::SetClipProperty {
                sequence,
                track,
                clip: *clip,
                property: ClipProperty::Rotation(degrees),
            });
        }
        let count = commands.len();
        if count > 0 {
            self.dispatch_group(
                if quarters < 0 {
                    "Rotate Left"
                } else {
                    "Rotate Right"
                },
                commands,
            )?;
        }
        Ok(count)
    }
}

/// `degrees` in (−180, 180], snapped to the nearest hundredth so repeated
/// turns do not collect float drift.
fn normalised(degrees: f32) -> f32 {
    let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    let wrapped = if wrapped <= -180.0 { 180.0 } else { wrapped };
    (wrapped * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::normalised;

    #[test]
    fn angles_wrap_into_the_controls_range() {
        assert_eq!(normalised(90.0), 90.0);
        assert_eq!(normalised(270.0), -90.0);
        assert_eq!(normalised(180.0), 180.0);
        assert_eq!(normalised(-180.0), 180.0);
        assert_eq!(normalised(360.0), 0.0);
        assert_eq!(normalised(135.0 + 90.0), -135.0);
    }
}
