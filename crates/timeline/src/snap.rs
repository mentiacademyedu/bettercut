//! Snapping (§10).
//!
//! Snapping is what makes a mouse-dragged edit land exactly on a cut instead of
//! three ticks away from it. Without it, "butt these two clips together" is
//! impossible with a mouse, and the one-frame gaps it leaves show up as black
//! flashes in the export.
//!
//! The tolerance is supplied by the caller **in ticks**, converted from a pixel
//! distance at the current zoom. That keeps the feel constant — snapping should
//! grab from roughly the same *visual* distance whether you are zoomed to
//! frames or to minutes — while keeping every value in this crate an integer
//! (§74).

use bettercut_foundation::{ClipId, TimelineTime};

use crate::clip::Clip;
use crate::sequence::Sequence;

/// Why a particular instant is worth snapping to. Drives the guide line's
/// appearance and the "what did it snap to?" readout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    /// The very start of the timeline.
    SequenceStart,
    Playhead,
    ClipStart(ClipId),
    ClipEnd(ClipId),
    /// A marker (`crate::marker`), placed there on purpose.
    Marker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapTarget {
    pub time: TimelineTime,
    pub kind: SnapKind,
}

impl SnapKind {
    /// Ranking for ties. When two targets are the same distance away, the more
    /// deliberate one wins: the user put the playhead there on purpose, so it
    /// beats an incidental clip edge.
    fn priority(self) -> u8 {
        match self {
            Self::Playhead => 0,
            // Put there on purpose, like the playhead, and unlike a clip edge
            // that is wherever the last trim left it.
            Self::Marker => 1,
            Self::SequenceStart => 2,
            Self::ClipStart(_) | Self::ClipEnd(_) => 3,
        }
    }
}

/// Collect everything worth snapping to across the whole sequence.
///
/// `exclude` is the clip (or clips) being dragged — a clip must never snap to
/// its own edges, or it would refuse to move at all.
///
/// Targets come from **all** tracks, not just the one being edited: aligning a
/// cut on V1 with a beat marked on A1 is the single most common reason to snap.
pub fn collect_targets(
    sequence: &Sequence,
    playhead: TimelineTime,
    exclude: &[ClipId],
) -> Vec<SnapTarget> {
    let mut targets = Vec::new();

    targets.push(SnapTarget {
        time: TimelineTime::ZERO,
        kind: SnapKind::SequenceStart,
    });
    targets.push(SnapTarget {
        time: playhead,
        kind: SnapKind::Playhead,
    });

    let mut push_clip = |id: ClipId, range: crate::clip::TimelineRange| {
        if exclude.contains(&id) {
            return;
        }
        targets.push(SnapTarget {
            time: range.start,
            kind: SnapKind::ClipStart(id),
        });
        targets.push(SnapTarget {
            time: range.end,
            kind: SnapKind::ClipEnd(id),
        });
    };

    for track in &sequence.video_tracks {
        for clip in track.clips() {
            push_clip(clip.id(), clip.timeline());
        }
    }
    for track in &sequence.audio_tracks {
        for clip in track.clips() {
            push_clip(clip.id(), clip.timeline());
        }
    }
    // Titles too: lining a caption up with the cut it describes is as common
    // as lining up two shots.
    for track in &sequence.text_tracks {
        for clip in track.clips() {
            push_clip(clip.id(), clip.timeline());
        }
    }

    targets.extend(sequence.markers.iter().map(|marker| SnapTarget {
        time: marker.time,
        kind: SnapKind::Marker,
    }));

    targets
}

/// The nearest target within `tolerance`, or `None` to leave the value alone.
///
/// Ties break toward the higher-priority target, then toward the earlier time,
/// so the result does not depend on the order clips happen to be stored in.
pub fn nearest(
    value: TimelineTime,
    targets: &[SnapTarget],
    tolerance: TimelineTime,
) -> Option<SnapTarget> {
    if tolerance.ticks() <= 0 {
        return None;
    }

    targets
        .iter()
        .filter_map(|target| {
            let distance = (target.time.ticks() - value.ticks()).abs();
            (distance <= tolerance.ticks()).then_some((distance, target))
        })
        .min_by_key(|(distance, target)| (*distance, target.kind.priority(), target.time.ticks()))
        .map(|(_, target)| *target)
}

/// Snap a dragged value, returning both the result and what it caught on.
///
/// The caller draws a guide line at `SnapTarget::time` when this returns
/// `Some` — a snap the user cannot see reads as the drag being buggy.
pub fn snap(
    value: TimelineTime,
    targets: &[SnapTarget],
    tolerance: TimelineTime,
) -> (TimelineTime, Option<SnapTarget>) {
    match nearest(value, targets, tolerance) {
        Some(target) => (target.time, Some(target)),
        None => (value, None),
    }
}

/// Snap a scrubbed playhead to the cuts and marks near it.
///
/// The same targets a dragged clip snaps to, less the playhead itself: the
/// playhead is what is moving, and a target where it just was would hold it
/// in place. Parking the playhead exactly on a cut is how a split, a mark or
/// a trim-to-playhead lands where the eye put it rather than a frame off.
pub fn snap_playhead(
    value: TimelineTime,
    sequence: &Sequence,
    tolerance: TimelineTime,
) -> (TimelineTime, Option<SnapTarget>) {
    let targets: Vec<SnapTarget> = collect_targets(sequence, value, &[])
        .into_iter()
        .filter(|target| target.kind != SnapKind::Playhead)
        .collect();
    snap(value, &targets, tolerance)
}

/// Snap the *span* of a clip being moved: its leading edge and its trailing
/// edge are both candidates, and whichever snaps closer wins.
///
/// Dragging a clip so its **end** meets the next clip's start is just as common
/// as aligning its start, and only considering the start makes half of all
/// intended snaps impossible.
pub fn snap_move(
    new_start: TimelineTime,
    duration: TimelineTime,
    targets: &[SnapTarget],
    tolerance: TimelineTime,
) -> (TimelineTime, Option<SnapTarget>) {
    let new_end = new_start + duration;

    let by_start = nearest(new_start, targets, tolerance).map(|t| {
        let distance = (t.time.ticks() - new_start.ticks()).abs();
        (distance, t.time, t)
    });
    let by_end = nearest(new_end, targets, tolerance).map(|t| {
        let distance = (t.time.ticks() - new_end.ticks()).abs();
        // Snapping the end means placing the start a duration earlier.
        (distance, t.time - duration, t)
    });

    match (by_start, by_end) {
        (Some(start), Some(end)) => {
            if end.0 < start.0 {
                (end.1.max(TimelineTime::ZERO), Some(end.2))
            } else {
                (start.1.max(TimelineTime::ZERO), Some(start.2))
            }
        }
        (Some(start), None) => (start.1.max(TimelineTime::ZERO), Some(start.2)),
        (None, Some(end)) => (end.1.max(TimelineTime::ZERO), Some(end.2)),
        (None, None) => (new_start, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scrubbed playhead lands on a cut within reach and never on where it
    /// already was.
    #[test]
    fn a_scrubbed_playhead_snaps_to_a_cut_but_not_to_itself() {
        use crate::clip::{SourceRange, VideoClip};
        use bettercut_foundation::{MediaId, MediaTime};

        let mut sequence = Sequence::default_hd();
        let clip = VideoClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(4),
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap(),
        )
        .unwrap();
        sequence.video_tracks[0].insert(clip).unwrap();
        let tolerance = TimelineTime::from_millis(100);

        // Near the clip's start: onto it.
        let near = TimelineTime::from_seconds(4) - TimelineTime::from_millis(40);
        let (at, hit) = snap_playhead(near, &sequence, tolerance);
        assert_eq!(at, TimelineTime::from_seconds(4));
        assert!(matches!(hit.map(|t| t.kind), Some(SnapKind::ClipStart(_))));

        // Far from anything: left alone, not held where it was.
        let far = TimelineTime::from_seconds(10);
        assert_eq!(snap_playhead(far, &sequence, tolerance).0, far);
    }
    use crate::clip::{SourceRange, VideoClip};
    use crate::sequence::Sequence;
    use bettercut_foundation::{MediaId, MediaTime};

    fn sequence_with_clips(spans: &[(i64, i64)]) -> Sequence {
        let mut sequence = Sequence::default_hd();
        for (start, len) in spans {
            let source =
                SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(*len)).expect("len > 0");
            let clip = VideoClip::new(MediaId::new(), TimelineTime::from_ticks(*start), source)
                .expect("valid");
            sequence.video_tracks[0].insert(clip).expect("no overlap");
        }
        sequence
    }

    fn t(ticks: i64) -> TimelineTime {
        TimelineTime::from_ticks(ticks)
    }

    #[test]
    fn targets_include_every_clip_edge_and_the_playhead() {
        let sequence = sequence_with_clips(&[(0, 1000), (2000, 1000)]);
        let targets = collect_targets(&sequence, t(5000), &[]);

        let times: Vec<i64> = targets.iter().map(|s| s.time.ticks()).collect();
        assert!(times.contains(&0));
        assert!(times.contains(&1000));
        assert!(times.contains(&2000));
        assert!(times.contains(&3000));
        assert!(times.contains(&5000), "playhead missing");
    }

    /// A clip must not snap to itself, or it cannot be dragged at all.
    #[test]
    fn the_dragged_clip_is_excluded() {
        let sequence = sequence_with_clips(&[(0, 1000), (2000, 1000)]);
        let dragged = sequence.video_tracks[0].clips()[0].id;

        let targets = collect_targets(&sequence, t(9_999_999), &[dragged]);
        assert!(
            !targets
                .iter()
                .any(|s| matches!(s.kind, SnapKind::ClipStart(id) | SnapKind::ClipEnd(id) if id == dragged)),
            "dragged clip snapped to itself"
        );
    }

    #[test]
    fn snapping_catches_the_nearest_target_within_tolerance() {
        let sequence = sequence_with_clips(&[(0, 1000), (2000, 1000)]);
        let targets = collect_targets(&sequence, t(50_000), &[]);

        // 1040 is 40 ticks from the clip edge at 1000.
        let (snapped, hit) = snap(t(1040), &targets, t(100));
        assert_eq!(snapped.ticks(), 1000);
        assert!(hit.is_some());
    }

    #[test]
    fn snapping_leaves_the_value_alone_outside_tolerance() {
        let sequence = sequence_with_clips(&[(0, 1000)]);
        let targets = collect_targets(&sequence, t(50_000), &[]);

        let (snapped, hit) = snap(t(1500), &targets, t(100));
        assert_eq!(snapped.ticks(), 1500);
        assert!(hit.is_none());
    }

    #[test]
    fn a_zero_tolerance_disables_snapping() {
        let sequence = sequence_with_clips(&[(0, 1000)]);
        let targets = collect_targets(&sequence, t(0), &[]);
        assert!(nearest(t(1000), &targets, t(0)).is_none());
    }

    /// The playhead is a deliberate placement, so it wins an exact tie.
    #[test]
    fn the_playhead_wins_ties_against_a_clip_edge() {
        let sequence = sequence_with_clips(&[(0, 1000)]);
        let targets = collect_targets(&sequence, t(1000), &[]);

        let hit = nearest(t(1010), &targets, t(100)).expect("should snap");
        assert_eq!(hit.kind, SnapKind::Playhead);
    }

    /// Dragging a clip so its trailing edge meets the next clip is half of all
    /// real snapping, and only considering the leading edge would miss it.
    #[test]
    fn a_moved_clip_can_snap_by_its_trailing_edge() {
        let sequence = sequence_with_clips(&[(5000, 1000)]);
        let targets = collect_targets(&sequence, t(999_999), &[]);

        // Dragging a 1000-tick clip to start at 3960 puts its end at 4960 —
        // 40 ticks short of the clip starting at 5000.
        let (start, hit) = snap_move(t(3960), t(1000), &targets, t(100));
        assert_eq!(start.ticks(), 4000, "end did not snap to 5000");
        assert!(matches!(hit.expect("snapped").kind, SnapKind::ClipStart(_)));
    }

    #[test]
    fn the_closer_edge_wins_when_both_could_snap() {
        let sequence = sequence_with_clips(&[(1000, 500), (5000, 500)]);
        let targets = collect_targets(&sequence, t(999_999), &[]);

        // Start is 10 from 1500 (previous clip's end); end would be 40 from 5000.
        let (start, _) = snap_move(t(1490), t(3470), &targets, t(100));
        assert_eq!(start.ticks(), 1500, "the nearer edge should have won");
    }

    #[test]
    fn snapping_never_produces_a_negative_start() {
        let sequence = sequence_with_clips(&[(0, 1000)]);
        let targets = collect_targets(&sequence, t(0), &[]);

        // Dragging a long clip so its end snaps to 0 would put its start below 0.
        let (start, _) = snap_move(t(-40), t(5000), &targets, t(100));
        assert!(!start.is_negative(), "produced a negative start");
    }
}
