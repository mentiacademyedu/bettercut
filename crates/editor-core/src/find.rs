//! Finding clips on the timeline by what they are called.
//!
//! A long edit has the interview somewhere and the drone shot somewhere else;
//! scrolling for them is slow. A clip is found by the name of its file (or the
//! name given to the file in the project), a title by its words, and any clip
//! by its note. Every word typed must appear, in any case and any order.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::editor::Editor;

/// One clip that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub clip: ClipId,
    pub start: TimelineTime,
    /// What it matched on, for a list.
    pub label: String,
}

/// Whether every word of `query` appears in `text`, ignoring case. An empty
/// query matches nothing: a search box with nothing in it is not a search.
pub fn matches_query(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    let mut words = query.split_whitespace().peekable();
    words.peek().is_some() && words.all(|word| text.contains(&word.to_lowercase()))
}

impl Editor {
    /// Every clip in the sequence on screen that `query` finds, in time order.
    pub fn find_clips(&self, query: &str) -> Vec<Found> {
        // Asked every frame by the find box: nothing typed is no work.
        if query.trim().is_empty() {
            return Vec::new();
        }
        let Some(sequence) = self.active_sequence() else {
            return Vec::new();
        };
        let mut found: Vec<Found> = Vec::new();
        for span in sequence.clip_spans() {
            let name = self
                .video_clip(span.clip)
                .map(|c| c.media_id)
                .or_else(|| self.audio_clip(span.clip).map(|c| c.media_id))
                .and_then(|media| self.project().media_asset(media))
                .map(|asset| format!("{} {}", asset.display_name(), asset.file_name));
            let text = self.text_clip(span.clip).map(|c| c.text.clone());
            let note = self.clip_note(span.clip).map(str::to_owned);
            let haystack = [name.as_deref(), text.as_deref(), note.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            if matches_query(&haystack, query) {
                let label = text
                    .filter(|t| !t.trim().is_empty())
                    .or_else(|| {
                        self.video_clip(span.clip)
                            .map(|c| c.media_id)
                            .or_else(|| self.audio_clip(span.clip).map(|c| c.media_id))
                            .and_then(|media| self.project().media_asset(media))
                            .map(|asset| asset.display_name().to_owned())
                    })
                    .unwrap_or_else(|| "clip".to_owned());
                found.push(Found {
                    clip: span.clip,
                    start: span.timeline.start,
                    label,
                });
            }
        }
        found.sort_by_key(|f| f.start);
        found
    }
}
