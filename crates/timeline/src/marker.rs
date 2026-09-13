//! Markers: named instants on the timeline (§10).
//!
//! A marker is a place worth coming back to — a beat, a line of dialogue, the
//! frame a cut should land on. Clips snap to them, and the keys that jump
//! between cuts stop at them too, which is what makes cutting to music a
//! matter of dropping clips onto marks rather than scrubbing for each beat.
//!
//! On the sequence rather than on a clip, because a beat belongs to the song's
//! place in the edit, not to whichever clip happens to be under it.

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
}

impl Marker {
    pub fn at(time: TimelineTime) -> Self {
        Self {
            time,
            label: String::new(),
            color: crate::clip::ColorLabel::None,
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

        let many = (0..MAX_MARKERS as i64 + 50)
            .map(|i| Marker::at(t(i)))
            .collect();
        assert_eq!(normalized(many).len(), MAX_MARKERS);
    }
}
