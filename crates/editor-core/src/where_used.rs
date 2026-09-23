//! Where things are: the clips that use a file, the clips under the
//! playhead.

use bettercut_foundation::{ClipId, MediaId, TimelineTime};

use crate::editor::Editor;

impl Editor {
    /// Every clip on the active sequence that plays `media`, earliest first.
    pub fn clips_using(&self, media: MediaId) -> Vec<(ClipId, TimelineTime)> {
        let Some(sequence) = self.active_sequence() else {
            return Vec::new();
        };
        let mut found: Vec<(ClipId, TimelineTime)> = sequence
            .video_tracks
            .iter()
            .flat_map(|t| t.clips().iter())
            .filter(|c| c.media_id == media)
            .map(|c| (c.id, c.timeline.start))
            .chain(
                sequence
                    .audio_tracks
                    .iter()
                    .flat_map(|t| t.clips().iter())
                    .filter(|c| c.media_id == media)
                    .map(|c| (c.id, c.timeline.start)),
            )
            .collect();
        found.sort_by_key(|(_, start)| start.ticks());
        found
    }

    /// Every clip, on any lane, that the playhead is over.
    pub fn clips_under_playhead(&self) -> Vec<ClipId> {
        let at = self.playhead();
        self.active_sequence()
            .map(|s| {
                s.clip_spans()
                    .filter(|span| span.timeline.contains(at))
                    .map(|span| span.clip)
                    .collect()
            })
            .unwrap_or_default()
    }
}
