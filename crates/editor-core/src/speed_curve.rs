//! A speed curve drawn by hand: how fast the clip plays, across the clip.
//!
//! The five presets (`crate::speed_ramp::SpeedRamp`) are the shapes people
//! reach for most; this is for the sixth shape, the one that is not on the
//! list — hold for a beat, ease into slow motion on the punch, run out fast.
//!
//! The curve is points, read straight across between them, and it becomes
//! pieces the same way a preset does: everything downstream of a clip's speed
//! assumes one speed per clip (see the ramp's own note), so the curve is
//! *sampled* into equal pieces rather than made continuous.

use bettercut_foundation::Rational;

/// The slowest and fastest a point may be set to.
pub const MIN_SPEED: f32 = 0.1;
pub const MAX_SPEED: f32 = 10.0;

/// How many pieces a curve may be cut into. Fewer than three is a ramp with no
/// middle; more than sixteen is more cuts than a short clip has frames.
pub const MIN_PIECES: usize = 3;
pub const MAX_PIECES: usize = 16;

/// Speed against position through the clip, as points to read between.
///
/// Positions are 0–1 across the clip and speeds are multipliers of the clip's
/// own speed — the same "relative" rule the presets follow, so a curve drawn
/// on a 2× clip keeps its shape rather than snapping back to normal.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedCurve {
    points: Vec<(f32, f32)>,
}

impl Default for SpeedCurve {
    /// A flat curve at normal speed: the thing to start dragging.
    fn default() -> Self {
        Self {
            points: vec![(0.0, 1.0), (0.5, 1.0), (1.0, 1.0)],
        }
    }
}

impl SpeedCurve {
    /// A curve from points of any order or sanity: sorted, held to the limits,
    /// and always with a point at each end.
    pub fn new(points: impl IntoIterator<Item = (f32, f32)>) -> Self {
        let mut points: Vec<(f32, f32)> = points
            .into_iter()
            .filter(|(at, speed)| at.is_finite() && speed.is_finite())
            .map(|(at, speed)| (at.clamp(0.0, 1.0), speed.clamp(MIN_SPEED, MAX_SPEED)))
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        if points.first().is_none_or(|(at, _)| *at > 0.0) {
            let speed = points.first().map_or(1.0, |(_, speed)| *speed);
            points.insert(0, (0.0, speed));
        }
        if points.last().is_none_or(|(at, _)| *at < 1.0) {
            let speed = points.last().map_or(1.0, |(_, speed)| *speed);
            points.push((1.0, speed));
        }
        Self { points }
    }

    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// Move the point at `index` to `(at, speed)`, held between its
    /// neighbours so the curve never doubles back on itself.
    ///
    /// The two ends keep their positions: a curve that started a third of the
    /// way in would leave the first third of the clip undescribed.
    pub fn move_point(&mut self, index: usize, at: f32, speed: f32) {
        let last = self.points.len().saturating_sub(1);
        let Some(point) = self.points.get_mut(index) else {
            return;
        };
        let speed = speed.clamp(MIN_SPEED, MAX_SPEED);
        if index == 0 || index == last {
            point.1 = speed;
            return;
        }
        *point = (at, speed);
        let low = self.points[index - 1].0 + 0.01;
        let high = self.points[index + 1].0 - 0.01;
        self.points[index].0 = at.clamp(low.min(high), high.max(low));
    }

    /// Put a point at `at`, on the curve as it is now. Returns its index.
    pub fn add_point(&mut self, at: f32) -> usize {
        let at = at.clamp(0.01, 0.99);
        let speed = self.speed_at(at);
        let index = self
            .points
            .iter()
            .position(|(existing, _)| *existing > at)
            .unwrap_or(self.points.len());
        self.points.insert(index, (at, speed));
        index
    }

    /// Take a point out. The two ends stay: see [`Self::move_point`].
    pub fn remove_point(&mut self, index: usize) {
        if index == 0 || index + 1 >= self.points.len() {
            return;
        }
        self.points.remove(index);
    }

    /// The speed at `at`, read straight between the points either side.
    pub fn speed_at(&self, at: f32) -> f32 {
        let at = at.clamp(0.0, 1.0);
        match self.points.first() {
            None => 1.0,
            Some((first, speed)) if at <= *first => *speed,
            _ => {
                let last = self.points.last().copied().unwrap_or((1.0, 1.0));
                if at >= last.0 {
                    return last.1;
                }
                self.points
                    .windows(2)
                    .find(|pair| at >= pair[0].0 && at <= pair[1].0)
                    .map_or(last.1, |pair| {
                        let (a, b) = (pair[0], pair[1]);
                        let span = (b.0 - a.0).max(1e-6);
                        a.1 + (b.1 - a.1) * ((at - a.0) / span)
                    })
            }
        }
    }

    /// Whether this curve does nothing: normal speed all the way across.
    pub fn is_flat(&self) -> bool {
        self.points
            .iter()
            .all(|(_, speed)| (speed - 1.0).abs() < 0.005)
    }

    /// The curve as `pieces` exact factors, one per piece, read at the middle
    /// of each piece.
    ///
    /// Exact because §74 forbids floating point in timing: each speed is
    /// rounded to a hundredth and carried as a [`Rational`], which is what the
    /// re-timing wants anyway.
    pub fn factors(&self, pieces: usize) -> Vec<Rational> {
        let pieces = pieces.clamp(MIN_PIECES, MAX_PIECES);
        (0..pieces)
            .map(|piece| {
                let at = (piece as f32 + 0.5) / pieces as f32;
                let speed = self.speed_at(at).clamp(MIN_SPEED, MAX_SPEED);
                let hundredths = (f64::from(speed) * 100.0).round() as i64;
                Rational::new(hundredths.max(1), 100).unwrap_or(Rational::ONE)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_curve_is_normal_speed_everywhere() {
        let curve = SpeedCurve::default();
        assert!(curve.is_flat());
        for factor in curve.factors(5) {
            assert_eq!(factor, Rational::ONE);
        }
    }

    #[test]
    fn the_speed_between_two_points_is_read_straight_across() {
        let curve = SpeedCurve::new([(0.0, 1.0), (1.0, 3.0)]);
        assert!((curve.speed_at(0.5) - 2.0).abs() < 1e-5);
        assert!((curve.speed_at(0.25) - 1.5).abs() < 1e-5);
        assert!(!curve.is_flat());
    }

    #[test]
    fn points_are_sorted_and_the_ends_are_always_there() {
        let curve = SpeedCurve::new([(0.6, 2.0), (0.2, 0.5)]);
        let points = curve.points();
        assert_eq!(points.first().map(|p| p.0), Some(0.0));
        assert_eq!(points.last().map(|p| p.0), Some(1.0));
        assert!(points.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    }

    #[test]
    fn a_dragged_point_stays_between_its_neighbours() {
        let mut curve = SpeedCurve::new([(0.0, 1.0), (0.4, 1.0), (0.6, 1.0), (1.0, 1.0)]);
        // Dragged past the point after it.
        curve.move_point(1, 0.95, 2.0);
        assert!(curve.points()[1].0 < curve.points()[2].0);
        // The ends only ever change speed.
        curve.move_point(0, 0.5, 4.0);
        assert_eq!(curve.points()[0].0, 0.0);
        assert!((curve.points()[0].1 - 4.0).abs() < 1e-5);
    }

    #[test]
    fn a_point_added_on_the_curve_does_not_change_it() {
        let mut curve = SpeedCurve::new([(0.0, 1.0), (1.0, 3.0)]);
        let before: Vec<f32> = (0..10).map(|n| curve.speed_at(n as f32 / 9.0)).collect();
        let index = curve.add_point(0.5);
        assert_eq!(index, 1);
        let after: Vec<f32> = (0..10).map(|n| curve.speed_at(n as f32 / 9.0)).collect();
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() < 1e-5);
        }

        curve.remove_point(1);
        assert_eq!(curve.points().len(), 2);
        // The ends refuse to go.
        curve.remove_point(0);
        curve.remove_point(1);
        assert_eq!(curve.points().len(), 2);
    }

    #[test]
    fn factors_follow_the_curve_and_stay_inside_the_limits() {
        let curve = SpeedCurve::new([(0.0, 0.5), (0.5, 4.0), (1.0, 0.5)]);
        let factors = curve.factors(5);
        assert_eq!(factors.len(), 5);
        // Fastest in the middle, slowest at the ends.
        let middle = factors[2].as_f64();
        assert!(middle > factors[0].as_f64() && middle > factors[4].as_f64());
        for factor in &factors {
            let value = factor.as_f64();
            assert!(value >= f64::from(MIN_SPEED) - 1e-6 && value <= f64::from(MAX_SPEED));
        }
        // And a silly number of pieces is held to the limits.
        assert_eq!(curve.factors(999).len(), MAX_PIECES);
        assert_eq!(curve.factors(1).len(), MIN_PIECES);
    }

    #[test]
    fn a_speed_outside_the_limits_is_pulled_back_in() {
        let curve = SpeedCurve::new([(0.0, 50.0), (1.0, -3.0)]);
        assert!((curve.speed_at(0.0) - MAX_SPEED).abs() < 1e-5);
        assert!((curve.speed_at(1.0) - MIN_SPEED).abs() < 1e-5);
    }
}
