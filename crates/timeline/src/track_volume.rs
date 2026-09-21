//! A volume line along a whole track, rather than along one clip (§20a.4,
//! §24).
//!
//! A clip's envelope belongs to the clip: trim it, move it, change its speed
//! and the envelope goes with it, because it is *that shot's* level. A track's
//! belongs to the timeline. Bringing the music down for a scene, riding it up
//! under a montage and pulling it back for the last line is a shape about the
//! edit, and it has to survive the clips underneath being re-cut — which is
//! exactly what a clip envelope cannot do.
//!
//! Both exist, and they multiply: the clip says how loud that piece is, the
//! track says how loud the lane is under it, as a desk works.
//!
//! # Anchored in timeline time
//!
//! Clip keyframes are anchored in *source* time (§24) so that trimming a clip
//! does not slide its envelope. A track has no source, and the whole point of
//! the line is that it stays where it was drawn while the clips move under it,
//! so its points are timeline instants.

use bettercut_foundation::TimelineTime;
use serde::{Deserialize, Serialize};

/// The loudest a track's line may be taken, matching [`crate::MAX_TRACK_GAIN`].
pub const MAX_VOLUME: f32 = 4.0;

/// One point on a track's volume line: an instant, and the level there.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VolumePoint {
    pub at: TimelineTime,
    pub gain: f32,
}

impl VolumePoint {
    pub fn new(at: TimelineTime, gain: f32) -> Self {
        Self {
            at: TimelineTime::from_ticks(at.ticks().max(0)),
            gain: gain.clamp(0.0, MAX_VOLUME),
        }
    }
}

/// A track's volume line: points in time order, the level between them being
/// the straight line from one to the next.
///
/// Empty is not "silence", it is "no line": the track's own
/// [`crate::Track::gain`] is the level, exactly as it was before any of this
/// existed. One point is a flat level, which is the useful thing to draw first
/// and then take hold of.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<VolumePoint>", into = "Vec<VolumePoint>")]
pub struct TrackVolume {
    points: Vec<VolumePoint>,
}

impl From<Vec<VolumePoint>> for TrackVolume {
    fn from(points: Vec<VolumePoint>) -> Self {
        let mut volume = Self::default();
        volume.set(points);
        volume
    }
}

impl From<TrackVolume> for Vec<VolumePoint> {
    fn from(volume: TrackVolume) -> Self {
        volume.points
    }
}

impl TrackVolume {
    pub fn points(&self) -> &[VolumePoint] {
        &self.points
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Replace the whole line.
    ///
    /// Sorted, clamped and thinned here, in one place, for the reason
    /// `Keyframes::replace` gives: the line is written as a whole — a drag
    /// moves one point in a copy and writes the copy — and a second place that
    /// sorted would be a second place that could disagree about how.
    ///
    /// Two points at the same instant are one point: the later one wins, since
    /// it is the one a drag just put there.
    pub fn set(&mut self, points: impl IntoIterator<Item = VolumePoint>) {
        let mut points: Vec<VolumePoint> = points
            .into_iter()
            .map(|point| VolumePoint::new(point.at, point.gain))
            .collect();
        points.sort_by_key(|point| point.at.ticks());
        let mut deduped: Vec<VolumePoint> = Vec::with_capacity(points.len());
        for point in points {
            match deduped.last_mut() {
                Some(last) if last.at == point.at => *last = point,
                _ => deduped.push(point),
            }
        }
        self.points = deduped;
    }

    /// The level at `at`, or `None` when there is no line — in which case the
    /// track's static gain is the answer.
    ///
    /// Flat before the first point and after the last: a line drawn across the
    /// middle of an edit says nothing about either end, and reading it as a
    /// ramp from silence would quietly fade the start of the track.
    pub fn gain_at(&self, at: TimelineTime) -> Option<f32> {
        let first = self.points.first()?;
        if at <= first.at {
            return Some(first.gain);
        }
        let last = self.points.last()?;
        if at >= last.at {
            return Some(last.gain);
        }
        // The first point after `at`; there is one, since `at` is before the
        // last, and one before it, since `at` is after the first.
        let index = self.points.partition_point(|point| point.at <= at);
        let (before, after) = (self.points.get(index - 1)?, self.points.get(index)?);
        let span = (after.at.ticks() - before.at.ticks()).max(1);
        let along = (at.ticks() - before.at.ticks()) as f32 / span as f32;
        Some(before.gain + (after.gain - before.gain) * along)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> TimelineTime {
        TimelineTime::from_seconds(seconds)
    }

    fn line(points: &[(i64, f32)]) -> TrackVolume {
        TrackVolume::from(
            points
                .iter()
                .map(|(second, gain)| VolumePoint::new(at(*second), *gain))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn no_line_is_no_answer() {
        assert_eq!(TrackVolume::default().gain_at(at(3)), None);
    }

    #[test]
    fn one_point_is_a_flat_level() {
        let volume = line(&[(5, 0.5)]);
        assert_eq!(volume.gain_at(at(0)), Some(0.5));
        assert_eq!(volume.gain_at(at(5)), Some(0.5));
        assert_eq!(volume.gain_at(at(50)), Some(0.5));
    }

    #[test]
    fn between_two_points_it_is_the_line_between_them() {
        let volume = line(&[(0, 1.0), (10, 0.0)]);
        assert_eq!(volume.gain_at(at(0)), Some(1.0));
        assert_eq!(volume.gain_at(at(10)), Some(0.0));
        let half = volume.gain_at(at(5)).expect("a level");
        assert!((half - 0.5).abs() < 1e-5, "{half}");
    }

    #[test]
    fn it_is_flat_outside_the_points() {
        let volume = line(&[(10, 0.2), (20, 0.8)]);
        assert_eq!(volume.gain_at(at(0)), Some(0.2));
        assert_eq!(volume.gain_at(at(30)), Some(0.8));
    }

    #[test]
    fn points_are_sorted_clamped_and_deduped() {
        let volume = line(&[(20, 9.0), (10, -1.0), (10, 0.5)]);
        let points = volume.points();
        assert_eq!(points.len(), 2, "the two at ten are one point");
        assert_eq!(points[0].at, at(10));
        assert_eq!(points[0].gain, 0.5, "the later one wins");
        assert_eq!(points[1].gain, MAX_VOLUME, "held to the ceiling");
    }
}
