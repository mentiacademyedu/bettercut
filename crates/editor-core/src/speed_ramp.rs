//! Speed ramps: a clip that speeds up and slows down along a curve.
//!
//! # Why pieces, not a curve
//!
//! Everything that reads a clip's speed — the source position under the
//! playhead, trimming, splitting, audio resampling — assumes one speed per
//! clip. A continuous curve would have to rewrite every one of those, and each
//! rewrite is a place for picture and sound to disagree.
//!
//! So a ramp is built from what already works: the clip is split into equal
//! pieces of material, and each piece gets its own exact speed from the curve
//! (§74 — a `Rational`, never a float). Picture and sound are split and
//! re-timed together, so they stay in sync (§12), and the whole thing is one
//! undo step (§79).
//!
//! The trade-off is visible and deliberate: a ramp shows on the timeline as a
//! run of short clips, and the sound steps from one speed to the next rather
//! than gliding.

use bettercut_foundation::{ClipId, Rational, TimelineTime};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// A named speed curve, as the familiar presets name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeedRamp {
    /// Slow, a fast rush through the middle, slow again.
    Montage,
    /// Quick, a sudden slow-motion moment, quick again.
    Hero,
    /// Eases down into deep slow motion and back out.
    Bullet,
    /// Starts fast and settles to the clip's own speed.
    FlashIn,
    /// Holds the clip's own speed, then races away.
    FlashOut,
}

impl SpeedRamp {
    /// Every preset, in the order a menu shows them.
    pub const ALL: [Self; 5] = [
        Self::Montage,
        Self::Hero,
        Self::Bullet,
        Self::FlashIn,
        Self::FlashOut,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Montage => "Montage",
            Self::Hero => "Hero",
            Self::Bullet => "Bullet",
            Self::FlashIn => "Flash In",
            Self::FlashOut => "Flash Out",
        }
    }

    /// One line on what it does, for the menu's hover text.
    pub fn description(self) -> &'static str {
        match self {
            Self::Montage => "Slow, a fast rush through the middle, then slow again",
            Self::Hero => "Quick, a sudden slow-motion moment, then quick again",
            Self::Bullet => "Eases into deep slow motion and back out",
            Self::FlashIn => "Starts fast and settles to normal speed",
            Self::FlashOut => "Normal speed, then races away",
        }
    }

    /// How fast each piece plays, relative to the clip's speed now, from the
    /// first piece to the last. Every piece holds the same amount of material.
    ///
    /// Relative rather than absolute so a ramp on a clip already at 2× keeps
    /// its character instead of snapping back to normal speed around it.
    pub fn factors(self) -> &'static [Rational] {
        const HALF: Rational = Rational::from_parts(1, 2);
        const ONE: Rational = Rational::from_parts(1, 1);
        const TWO: Rational = Rational::from_parts(2, 1);
        const THREE: Rational = Rational::from_parts(3, 1);
        const FOUR: Rational = Rational::from_parts(4, 1);
        const QUARTER: Rational = Rational::from_parts(1, 4);
        const FIFTH: Rational = Rational::from_parts(1, 5);
        match self {
            Self::Montage => &[HALF, ONE, THREE, ONE, HALF],
            Self::Hero => &[TWO, TWO, QUARTER, TWO, TWO],
            Self::Bullet => &[ONE, HALF, FIFTH, HALF, ONE],
            Self::FlashIn => &[FOUR, TWO, ONE, ONE, ONE],
            Self::FlashOut => &[ONE, ONE, ONE, TWO, FOUR],
        }
    }
}

/// `a × b`, exactly, or `None` if it would not fit.
fn times(a: Rational, b: Rational) -> Option<Rational> {
    Rational::new(a.num().checked_mul(b.num())?, a.den().checked_mul(b.den())?)
}

impl Editor {
    /// Ramp `clip`'s speed along `ramp`, as one undo step.
    ///
    /// Splits the clip — and whatever is linked to it — into equal pieces of
    /// material and re-times each piece. Later clips on the same tracks move to
    /// make room, exactly as a speed change moves them.
    ///
    /// Returns the pieces, first to last. Fails without changing anything on a
    /// held frame or photo, or a clip too short to hold a frame per piece.
    pub fn apply_speed_ramp(
        &mut self,
        clip: ClipId,
        ramp: SpeedRamp,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        if !self.can_retime(clip) {
            return Err(EditorError::NoMotionToRetime);
        }
        let sequence = self
            .project()
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let span = sequence
            .clip_span(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let speed = self
            .video_clip(clip)
            .map(|video| video.speed)
            .or_else(|| self.audio_clip(clip).map(|audio| audio.speed))
            .ok_or(EditorError::ClipNotFound(clip))?;

        let factors = ramp.factors();
        let pieces = factors.len();
        let (start, end) = (span.timeline.start, span.timeline.end);

        // Equal lengths at one speed are equal amounts of material, so the
        // cuts are evenly spaced — then put on the frame grid (§76), where two
        // may land on the same frame if the clip is short.
        let length = end.ticks() - start.ticks();
        let mut cuts: Vec<TimelineTime> = (1..pieces as i64)
            .map(|k| {
                let at = start + TimelineTime::from_ticks(length * k / pieces as i64);
                sequence.snap_to_frame(at)
            })
            .filter(|at| *at > start && *at < end)
            .collect();
        cuts.dedup();
        if cuts.len() + 1 != pieces {
            return Err(EditorError::TooShortToRamp { pieces });
        }
        let track = span.track;

        self.staged(format!("{} Speed Ramp", ramp.label()), |editor, stage| {
            for at in cuts {
                // Whatever covers `at` now: after the first cut, the right half.
                let Some((_, target, _)) = editor
                    .clips_from(at, true)
                    .into_iter()
                    .find(|(on, _, _)| *on == track)
                else {
                    continue;
                };
                for command in editor.split_commands(sequence_id, at, &[target]) {
                    editor.stage(stage, command)?;
                }
            }

            // Nothing has moved yet, so the pieces are exactly the clips on this
            // track inside the original span.
            let ids: Vec<ClipId> = editor
                .project()
                .sequence(sequence_id)
                .ok_or(EditorError::SequenceNotFound(sequence_id))?
                .clip_spans()
                .filter(|piece| {
                    piece.track == track
                        && piece.timeline.start >= start
                        && piece.timeline.end <= end
                })
                .map(|piece| piece.clip)
                .collect();
            if ids.len() != pieces {
                return Err(EditorError::TooShortToRamp { pieces });
            }

            // Left to right: each re-time pushes the pieces after it, and they
            // are found by id, so the push never loses one.
            for (piece, factor) in ids.iter().zip(factors) {
                let target = times(speed, *factor).unwrap_or(speed);
                for linked in editor.linked_with(*piece) {
                    let Some(on) = editor.track_of(linked) else {
                        continue;
                    };
                    editor.stage(
                        stage,
                        Command::SetClipSpeed {
                            sequence: sequence_id,
                            track: on,
                            clip: linked,
                            speed: target,
                        },
                    )?;
                }
            }
            Ok(ids)
        })
    }
}
