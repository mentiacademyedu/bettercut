//! The cut being trimmed: which one, what is either side of it, and how far
//! it can go.
//!
//! Trimming on the timeline is a drag, and a drag is a guess: the hand moves
//! in pixels while the edit is in frames, and the two frames that actually
//! matter — the last of the shot going out and the first of the shot coming
//! in — are a few pixels tall on the way past. Every editor made for cutting
//! has a second way to do it: pick the cut, look at both frames big, and move
//! it a frame at a time.
//!
//! What lives here is the *picking*: everything else the trim window needs
//! already exists — [`Editor::roll_room`](crate::editor::Editor) says how far
//! the cut can move, and [`Editor::roll_edit`](crate::editor::Editor) moves
//! it, one undo step, sound and all (§12).
//!
//! # Roll, not ripple
//!
//! Moving the cut leaves everything after it where it is, which is what a
//! trim window is for: a scene is timed, and the question is where within it
//! to change shot. A ripple — pulling the whole rest of the cut along — is the
//! timeline's Q and W, where the consequence is visible.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::command::TrimEdge;
use crate::editor::Editor;

/// A cut between two shots that touch, as the trim window works on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// The shot going out, and the shot coming in.
    pub outgoing: ClipId,
    pub incoming: ClipId,
    /// Where the cut is now.
    pub at: TimelineTime,
    /// The earliest and latest it could be moved to, both files and both
    /// neighbours allowed for.
    pub earliest: TimelineTime,
    pub latest: TimelineTime,
}

impl Cut {
    /// Whether the cut can move at all — a shot trimmed to the end of its file
    /// on one side and a frame long on the other has nowhere to go.
    pub fn can_move(self) -> bool {
        self.earliest < self.latest
    }
}

impl Editor {
    /// The cut nearest `at` on the spine of the edit: the first picture lane,
    /// where a shot going out meets a shot coming in.
    ///
    /// `None` when that lane has no such cut — one clip, or clips with gaps
    /// between them, which is a start and an end rather than a cut.
    pub fn cut_near(&self, at: TimelineTime) -> Option<Cut> {
        let sequence = self.active_sequence()?;
        let clips = sequence.video_tracks.first()?.clips();

        // Every place a shot ends and the next begins at the same instant.
        let nearest = clips
            .windows(2)
            .filter(|pair| pair[0].timeline.end == pair[1].timeline.start)
            .min_by_key(|pair| (pair[0].timeline.end.ticks() - at.ticks()).abs())?;
        self.cut_after(nearest[0].id)
    }

    /// The cut at the end of `outgoing`, if a shot begins exactly where it
    /// ends.
    pub fn cut_after(&self, outgoing: ClipId) -> Option<Cut> {
        let incoming = self.roll_partner(outgoing, TrimEdge::End)?;
        let at = self.active_sequence()?.clip_span(outgoing)?.timeline.end;
        let (earliest, latest) = self.roll_room(outgoing)?;
        Some(Cut {
            outgoing,
            incoming,
            at,
            earliest,
            latest,
        })
    }

    /// Move the cut at the end of `outgoing` by `frames`, negative for
    /// earlier. Returns where it ended up.
    ///
    /// Held inside [`Cut::earliest`] and [`Cut::latest`], so trimming past
    /// what the footage allows stops there rather than being refused: a key
    /// held down should run out of room quietly.
    pub fn trim_cut_by(
        &mut self,
        outgoing: ClipId,
        frames: i64,
    ) -> Result<TimelineTime, crate::error::EditorError> {
        let cut = self
            .cut_after(outgoing)
            .ok_or(crate::error::EditorError::NoNeighbour)?;
        let sequence = self.active_sequence_id()?;
        let per_frame = self
            .project()
            .sequence(sequence)
            .and_then(|active| bettercut_foundation::ticks_per_frame(active.frame_rate))
            .unwrap_or(1)
            .max(1);

        let wanted = TimelineTime::from_ticks(cut.at.ticks() + frames * per_frame);
        let to = wanted.clamp(cut.earliest, cut.latest);
        if to == cut.at {
            return Ok(cut.at);
        }
        self.roll_edit(outgoing, to)?;
        // Where it actually landed: the roll snaps to the frame grid, and the
        // window should say what happened rather than what was asked for.
        Ok(self
            .active_sequence()
            .and_then(|active| active.clip_span(outgoing))
            .map_or(to, |span| span.timeline.end))
    }
}
