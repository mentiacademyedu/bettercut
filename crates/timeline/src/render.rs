//! Baked stretches of the edit: render in place.
//!
//! A minute of stacked titles, blurs and glitches over four layers is more
//! than a preview can composite at rate, and the answer every editor expects
//! is to bake that stretch once and play the file. The bake is written by the
//! ordinary export path, imported as media, and remembered here.
//!
//! # What makes a bake stale
//!
//! Everything the picture in that stretch is made of: the clips under it, the
//! master look, the shape and rate of the sequence, and the files the clips
//! read. `bettercut_playback::rendered` hashes exactly that, and a bake whose
//! hash no longer matches is ignored — the preview goes back to compositing,
//! which is slower and right, rather than showing an edit the user has already
//! changed.
//!
//! A stale bake is not deleted. The edit is often changed and changed back —
//! an effect turned off to look underneath it, a title nudged and nudged
//! home — and the file is still exactly the frames that edit makes.

use bettercut_foundation::{ClipId, MediaId};
use serde::{Deserialize, Serialize};

use crate::clip::TimelineRange;

/// One stretch of a sequence, already rendered to a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedRange {
    /// The stretch of the timeline the file covers, start inclusive.
    pub range: TimelineRange,

    /// The baked file, imported into the project so it decodes through the
    /// same path as any other media — proxies and thumbnails excepted, since
    /// it is not something the user imported.
    pub media: MediaId,

    /// The id the one baked layer is planned under.
    ///
    /// Kept rather than made per frame: the decode cache and the decode-ahead
    /// ring key on the clip id, and a fresh one each frame would open a fresh
    /// decoder each frame — which would be slower than compositing the edit
    /// the bake was meant to replace.
    pub clip: ClipId,

    /// What the edit under `range` hashed to when it was baked.
    pub fingerprint: u64,
}

impl RenderedRange {
    pub fn covers(&self, position: bettercut_foundation::TimelineTime) -> bool {
        self.range.contains(position)
    }
}
