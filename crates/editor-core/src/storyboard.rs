//! The storyboard: the cut as a row of cards rather than as a strip of time.
//!
//! A timeline answers "how long is this and where does it land". A storyboard
//! answers a different question — "what happens, and in what order" — and it
//! is the question a rough assembly is made of. Dragging card four in front of
//! card two is a thing people do on paper, on a wall, and in every editor made
//! for people who did not train on one.
//!
//! # The main picture lane, and nothing else
//!
//! Cards are the clips on the first picture lane: the spine of the edit. An
//! overlay on V2, a title, a music bed — none of them are "what happens next",
//! and putting them in the row would make it a second timeline rather than a
//! storyboard. What travels with a card is what §12 says travels with its
//! clip: its own sound, and anything grouped with it.
//!
//! # Reordering keeps the shape of the cut
//!
//! Moving a card does not move the cut about in time: the clips keep the
//! stretch of timeline they covered together and the gaps between them stay
//! where they were, exactly as a shuffle does — it is the same operation with
//! a chosen order instead of a random one
//! ([`Editor::lay_out_in_order`](crate::editor::Editor)).

use bettercut_foundation::{ClipId, MediaId, TimelineTime, TrackId};

use crate::editor::Editor;
use crate::error::EditorError;

/// One card: a clip on the spine of the edit, as the storyboard shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub clip: ClipId,
    /// The file it plays, for a thumbnail. `None` for a colour or a compound.
    pub media: Option<MediaId>,
    /// Where in the file to take the picture from: its in-point, which is the
    /// frame that says what the shot is.
    pub poster: bettercut_foundation::MediaTime,
    pub start: TimelineTime,
    pub duration: TimelineTime,
    /// What to write on it: the clip's own name, or its file's.
    pub name: String,
}

impl Editor {
    /// The lane the storyboard is of: the first picture lane, the spine of the
    /// edit.
    pub fn storyboard_track(&self) -> Option<TrackId> {
        self.active_sequence()?.video_tracks.first().map(|t| t.id)
    }

    /// Every clip on that lane, in order, as cards.
    pub fn storyboard(&self) -> Vec<Card> {
        let Some(sequence) = self.active_sequence() else {
            return Vec::new();
        };
        let Some(track) = sequence.video_tracks.first() else {
            return Vec::new();
        };
        track
            .clips()
            .iter()
            .map(|clip| {
                let asset = self.project().media_asset(clip.media_id);
                Card {
                    clip: clip.id,
                    // A generated picture has no file to read a frame from.
                    media: asset
                        .filter(|asset| asset.generated.is_none())
                        .map(|asset| asset.id),
                    poster: clip.source.start,
                    start: clip.timeline.start,
                    duration: clip.timeline.duration(),
                    name: asset.map_or_else(
                        || "clip".to_owned(),
                        |asset| asset.display_name().to_owned(),
                    ),
                }
            })
            .collect()
    }

    /// Move the card at `from` so that it sits at `to`, sliding the cards
    /// between them along. One undo step.
    ///
    /// Nothing happens, and no step is made, when the card is already there.
    /// Refused when the lane has a gap-filling clip from somewhere else in the
    /// way — the same refusal a shuffle makes, and for the same reason: the
    /// cut would come out in an order nobody asked for.
    pub fn reorder_storyboard(&mut self, from: usize, to: usize) -> Result<(), EditorError> {
        let cards = self.storyboard();
        if cards.len() < 2 {
            return Err(EditorError::NothingToShuffle);
        }
        if from >= cards.len() || to >= cards.len() || from == to {
            return Ok(());
        }

        let placed: Vec<(ClipId, TimelineTime, TimelineTime)> = cards
            .iter()
            .map(|card| {
                (
                    card.clip,
                    card.start,
                    TimelineTime::from_ticks(card.start.ticks() + card.duration.ticks()),
                )
            })
            .collect();

        // Which clip ends up in which slot: the moved one taken out and put
        // back in, which is what dragging a card does.
        let mut order: Vec<usize> = (0..placed.len()).collect();
        let moved = order.remove(from);
        order.insert(to, moved);

        self.lay_out_in_order(&placed, &order, "Reorder Clips")
    }
}
