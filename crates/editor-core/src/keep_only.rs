//! Keeping the moments worth keeping: everything *but* the stretches given
//! comes out, which is the other way round from removing silences and is
//! what a highlight reel actually is.

use bettercut_foundation::ClipId;
use bettercut_timeline::TimelineRange;

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Take out everything in `clip` that is not inside one of `keep`, and
    /// close the gaps. One undo step (it is one `remove_ranges` underneath).
    /// Returns how many stretches came out.
    ///
    /// Anything linked to the clip goes with it, as it does for any other
    /// ripple edit (§12). An empty `keep` is refused rather than silently
    /// deleting the clip: "keep nothing" is never what anybody meant.
    pub fn keep_only(
        &mut self,
        clip: ClipId,
        keep: &[TimelineRange],
    ) -> Result<usize, EditorError> {
        if keep.is_empty() {
            return Ok(0);
        }
        let span = self
            .active_sequence()
            .and_then(|s| s.clip_span(clip))
            .ok_or(EditorError::ClipNotFound(clip))?
            .timeline;
        let drop = outside(span, keep);
        if drop.is_empty() {
            return Ok(0);
        }
        self.remove_ranges(clip, &drop)
    }
}

/// The parts of `span` that no range in `keep` covers, in order. `keep` may
/// be in any order and may overlap.
pub fn outside(span: TimelineRange, keep: &[TimelineRange]) -> Vec<TimelineRange> {
    let mut kept: Vec<TimelineRange> = keep
        .iter()
        .filter_map(|range| {
            TimelineRange::new(range.start.max(span.start), range.end.min(span.end)).ok()
        })
        .collect();
    kept.sort_by_key(|range| range.start.ticks());

    let mut out = Vec::new();
    let mut at = span.start;
    for range in kept {
        if range.start > at
            && let Ok(gap) = TimelineRange::new(at, range.start)
        {
            out.push(gap);
        }
        at = at.max(range.end);
    }
    if at < span.end
        && let Ok(gap) = TimelineRange::new(at, span.end)
    {
        out.push(gap);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::TimelineTime;

    fn range(from: i64, to: i64) -> TimelineRange {
        TimelineRange::new(
            TimelineTime::from_seconds(from),
            TimelineTime::from_seconds(to),
        )
        .unwrap()
    }

    #[test]
    fn what_is_dropped_is_everything_between_and_around() {
        let span = range(0, 20);
        assert_eq!(
            outside(span, &[range(5, 8), range(12, 15)]),
            [range(0, 5), range(8, 12), range(15, 20)]
        );
        // Out of order and overlapping is the same answer.
        assert_eq!(
            outside(span, &[range(12, 15), range(5, 8), range(6, 9)]),
            [range(0, 5), range(9, 12), range(15, 20)]
        );
        // A keep covering the lot drops nothing.
        assert!(outside(span, &[range(0, 20)]).is_empty());
        // Keeps outside the span are clamped away.
        assert_eq!(outside(span, &[range(30, 40)]), [range(0, 20)]);
    }
}
