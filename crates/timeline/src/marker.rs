//! Markers: named instants on the timeline (§10).
//!
//! A marker is a place worth coming back to — a beat, a line of dialogue, the
//! frame a cut should land on. Clips snap to them, and the keys that jump
//! between cuts stop at them too, which is what makes cutting to music a
//! matter of dropping clips onto marks rather than scrubbing for each beat.
//!
//! On the sequence rather than on a clip, because a beat belongs to the song's
//! place in the edit, not to whichever clip happens to be under it.
//!
//! # A mark can cover a stretch
//!
//! Most marks are an instant — a beat, a frame to cut on. Some are about a
//! *passage*: "this whole bit is too long", "music from here to here", "the
//! client hates this". Those carry a [`Marker::span`], drawn as a band rather
//! than a flag, and everything else about them is the same mark: it sorts by
//! where it starts, it snaps from where it starts, and it is still one entry
//! in the list.

use bettercut_foundation::TimelineTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub time: TimelineTime,
    /// What it marks. Empty for a plain mark, which is most of them.
    #[serde(default)]
    pub label: String,
    /// A colour, to tell kinds of mark apart at a glance — beats one colour,
    /// "fix this" another. The same set a clip's label uses. Defaulted: older
    /// projects' markers have none.
    #[serde(default)]
    pub color: crate::clip::ColorLabel,
    /// How long the mark covers, for a note about a passage rather than about
    /// an instant. Zero — the default, and what every older project has — is
    /// an instant.
    #[serde(default)]
    pub span: TimelineTime,
}

impl Marker {
    pub fn at(time: TimelineTime) -> Self {
        Self {
            time,
            label: String::new(),
            color: crate::clip::ColorLabel::None,
            span: TimelineTime::ZERO,
        }
    }

    /// A mark over a stretch: from `time` for `span`.
    pub fn over(time: TimelineTime, span: TimelineTime) -> Self {
        Self {
            span: TimelineTime::from_ticks(span.ticks().max(0)),
            ..Self::at(time)
        }
    }

    /// Whether this mark covers a passage rather than an instant.
    pub fn is_ranged(&self) -> bool {
        self.span.ticks() > 0
    }

    /// Where the mark ends: its own instant when it has no span.
    pub fn end(&self) -> TimelineTime {
        TimelineTime::from_ticks(self.time.ticks() + self.span.ticks().max(0))
    }

    /// Whether `at` falls inside the mark — true only at the instant itself
    /// for a plain one.
    pub fn covers(&self, at: TimelineTime) -> bool {
        if self.is_ranged() {
            at >= self.time && at < self.end()
        } else {
            at == self.time
        }
    }
}

/// The most markers a sequence holds. Beat detection on a long song is a few
/// hundred; far past this, the ruler is solid colour and snapping catches on
/// everything.
pub const MAX_MARKERS: usize = 2_000;

/// Put a list of markers in the one shape a sequence keeps them in: sorted by
/// time, one per instant, none before zero, at most [`MAX_MARKERS`].
///
/// Applied by whoever stores a list, so the invariant does not depend on every
/// caller building the list correctly — the journal replays whatever was
/// dispatched (§38.2).
pub fn normalized(mut markers: Vec<Marker>) -> Vec<Marker> {
    markers.retain(|m| m.time >= TimelineTime::ZERO);
    markers.sort_by_key(|m| m.time);
    // A labelled marker wins over a plain one at the same instant.
    markers.dedup_by(|later, earlier| {
        if later.time != earlier.time {
            return false;
        }
        if earlier.label.is_empty() {
            earlier.label = std::mem::take(&mut later.label);
        }
        if earlier.color == crate::clip::ColorLabel::None {
            earlier.color = later.color;
        }
        // The longer reach wins: two marks at one instant are one mark, and
        // the note about the passage is the one that would be lost.
        if later.span > earlier.span {
            earlier.span = later.span;
        }
        true
    });
    markers.truncate(MAX_MARKERS);
    markers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> TimelineTime {
        TimelineTime::from_seconds(s)
    }

    #[test]
    fn a_list_is_sorted_deduplicated_and_bounded() {
        let mut labelled = Marker::at(t(2));
        labelled.label = "drop".to_owned();
        let list = normalized(vec![
            Marker::at(t(5)),
            Marker::at(t(2)),
            labelled,
            Marker::at(TimelineTime::from_ticks(-1)),
        ]);
        let times: Vec<_> = list.iter().map(|m| m.time).collect();
        assert_eq!(times, [t(2), t(5)]);
        assert_eq!(list[0].label, "drop", "the labelled one survives");

        // A colour survives a merge the same way.
        let mut coloured = Marker::at(t(4));
        coloured.color = crate::clip::ColorLabel::Pink;
        let merged = normalized(vec![Marker::at(t(4)), coloured]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].color, crate::clip::ColorLabel::Pink);

        let many = (0..MAX_MARKERS as i64 + 50)
            .map(|i| Marker::at(t(i)))
            .collect();
        assert_eq!(normalized(many).len(), MAX_MARKERS);
    }
}
