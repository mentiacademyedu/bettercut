//! One sound lane alone, for a stem: the export copy with every other
//! sound lane muted and the kept one heard whatever its own switches say.

use bettercut_foundation::{SequenceId, TrackId};
use bettercut_project_format::Project;

use crate::error::EditorError;

/// Make `sequence` in `project` play only the sound lane `keep`: every
/// other sound lane off, `keep` on and unsoloed, so a stem is that lane as
/// it sits in the mix — its volume line, its EQ — and nothing else.
pub fn isolate_sound_lane(
    project: &mut Project,
    sequence: SequenceId,
    keep: TrackId,
) -> Result<(), EditorError> {
    let sequence = project
        .sequence_mut(sequence)
        .ok_or(EditorError::SequenceNotFound(sequence))?;
    if !sequence.audio_tracks.iter().any(|t| t.id == keep) {
        return Err(EditorError::TrackNotFound(keep));
    }
    for track in &mut sequence.audio_tracks {
        let this = track.id == keep;
        track.enabled = this;
        track.solo = false;
    }
    // A soloed sound clip elsewhere would silence the lane being kept.
    sequence.soloed_clips.clear();
    Ok(())
}
