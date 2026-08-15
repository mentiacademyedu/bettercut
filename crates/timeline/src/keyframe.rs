//! Keyframes: clip parameters that change over time (§24).
//!
//! ## Times are anchored to the source, not to the timeline
//!
//! §24 writes keyframe times as `TimelineTime`. They are stored here as
//! [`MediaTime`] instead — the same 960 kHz integer tick (§9), never a float,
//! but measured against the *source media* rather than the sequence.
//!
//! The reason is that everything else about a clip already moves. Dragging it,
//! trimming either edge, splitting it, cutting and pasting it: each of those
//! changes where the clip sits on the timeline while leaving `source`
//! untouched. Timeline-anchored keys would have to be rewritten by every one of
//! those operations, and every one of them would need its undo to put the keys
//! back — a fade that drifts off its shot after an unrelated ripple delete is
//! exactly the kind of bug that is easy to write and hard to notice.
//!
//! Anchored to the source, none of those operations has to do anything at all,
//! and the animation stays on the picture it was drawn against.
//!
//! ## One curve per scalar
//!
//! Position and scale are animated as two independent curves each (`PositionX`
//! and `PositionY`) rather than as one curve over a pair. That keeps
//! interpolation to a single scalar implementation, and it is also what an
//! animator wants: easing X and Y separately is the difference between a linear
//! slide and an arc.

use bettercut_foundation::MediaTime;

use serde::{Deserialize, Serialize};

/// A clip parameter that can be animated (§24, §59).
///
/// A closed enum for the same reason as `ClipProperty`: §38.2's journal replays
/// these after a crash, so every variant that can be written to disk has to be
/// one the current build knows how to evaluate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimatedParameter {
    Opacity,
    PositionX,
    PositionY,
    ScaleX,
    ScaleY,
    Rotation,
    Brightness,
    Contrast,
    Saturation,
    Blur,
}

impl AnimatedParameter {
    /// Every parameter, in the order the inspector shows them.
    pub const ALL: [Self; 10] = [
        Self::Opacity,
        Self::ScaleX,
        Self::ScaleY,
        Self::PositionX,
        Self::PositionY,
        Self::Rotation,
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Blur,
    ];

    /// The limits this parameter is meaningful within.
    ///
    /// These live here, on the parameter, rather than at the point of editing,
    /// because there are now two ways to set a value — a slider and an
    /// interpolated key — and a curve that eased opacity to 1.4 while the
    /// slider stopped at 1.0 would be a difference the user cannot see the
    /// cause of. `None` means genuinely unbounded: rotation winds as far as it
    /// is dragged, and position may put a clip off-screen on purpose.
    pub fn limits(self) -> Option<(f32, f32)> {
        match self {
            Self::Opacity => Some((0.0, 1.0)),
            Self::PositionX | Self::PositionY | Self::Rotation => None,
            // Zero renders nothing and cannot be dragged back out of.
            Self::ScaleX | Self::ScaleY => Some((0.01, 10.0)),
            Self::Brightness | Self::Contrast | Self::Saturation => Some((0.0, 4.0)),
            Self::Blur => Some((0.0, crate::clip::MAX_BLUR)),
        }
    }

    /// The value this parameter has on an untouched clip.
    ///
    /// Here rather than at the point of resetting, because there are now three
    /// places that need to know it — the Inspector's per-control reset, the
    /// whole-clip reset, and the "has this been changed" check that decides
    /// whether either is worth offering. Three lists of numbers would eventually
    /// disagree about what untouched means.
    pub fn default_value(self) -> f32 {
        match self {
            // The identity for a multiply is one; for an offset it is zero.
            Self::Opacity
            | Self::ScaleX
            | Self::ScaleY
            | Self::Brightness
            | Self::Contrast
            | Self::Saturation => 1.0,
            Self::PositionX | Self::PositionY | Self::Rotation | Self::Blur => 0.0,
        }
    }

    /// Whether `value` is what an untouched clip would have.
    pub fn is_default(self, value: f32) -> bool {
        // Exact: these are set from the same constants, not accumulated, so a
        // tolerance would only hide a real difference.
        value == self.default_value()
    }

    /// Hold `value` inside [`Self::limits`].
    pub fn clamp(self, value: f32) -> f32 {
        match self.limits() {
            Some((low, high)) => value.clamp(low, high),
            None => value,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Opacity => "opacity",
            Self::PositionX => "position x",
            Self::PositionY => "position y",
            Self::ScaleX => "scale x",
            Self::ScaleY => "scale y",
            Self::Rotation => "rotation",
            Self::Brightness => "brightness",
            Self::Contrast => "contrast",
            Self::Saturation => "saturation",
            Self::Blur => "blur",
        }
    }
}

/// How the value moves from one keyframe to the next (§24).
///
/// The easing belongs to the **earlier** key of a pair: it describes the
/// segment leaving that key. That is what makes `Hold` mean what it reads as —
/// hold this value until the next key.
///
/// Every curve here is a cubic Bézier over the unit square, including the named
/// presets, which use the same control points CSS does. One evaluator, so a
/// hand-authored curve and a preset cannot disagree about what "ease out"
/// means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "curve", rename_all = "snake_case")]
pub enum Interpolation {
    /// Keep this key's value until the next one, then jump.
    Hold,
    /// The default: a straight ramp, which is what a key placed without a
    /// choice of curve should do.
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Explicit control points, for curves the presets do not cover.
    ///
    /// `x` is clamped into the unit interval when evaluated — a control point
    /// outside it makes the curve non-monotonic in time, which has no meaning
    /// here. `y` is deliberately *not* clamped: overshoot past the endpoint
    /// values is how an animator gets a bounce.
    Bezier {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
}

impl Interpolation {
    /// The presets an interface offers, in order of how often they are wanted.
    pub const PRESETS: [Self; 5] = [
        Self::Linear,
        Self::EaseInOut,
        Self::EaseIn,
        Self::EaseOut,
        Self::Hold,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Linear => "linear",
            Self::EaseIn => "ease in",
            Self::EaseOut => "ease out",
            Self::EaseInOut => "ease in-out",
            Self::Bezier { .. } => "custom",
        }
    }

    /// Map linear progress through a segment (0..=1) to eased progress.
    ///
    /// The result is not clamped: a Bézier with overshooting `y` handles is
    /// supposed to leave the 0..1 range. Callers clamp the *value* afterwards,
    /// which is where the meaningful limits are — an opacity of 1.4 is wrong,
    /// but an eased progress of 1.1 on the way there is not.
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Hold => 0.0,
            // Exactly `t`, and cheaper than solving for it.
            Self::Linear => t,
            Self::EaseIn => cubic_bezier(t, 0.42, 0.0, 1.0, 1.0),
            Self::EaseOut => cubic_bezier(t, 0.0, 0.0, 0.58, 1.0),
            Self::EaseInOut => cubic_bezier(t, 0.42, 0.0, 0.58, 1.0),
            Self::Bezier { x1, y1, x2, y2 } => {
                cubic_bezier(t, x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2)
            }
        }
    }
}

/// One component of a cubic Bézier anchored at 0 and 1.
fn bezier(t: f32, p1: f32, p2: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

fn bezier_slope(t: f32, p1: f32, p2: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Find the curve parameter whose x is `x`.
///
/// A Bézier is parameterized by its own `t`, which is *not* the horizontal
/// position; reading `y` at `t = x` would give a curve that is subtly wrong
/// everywhere except the ends. Newton–Raphson converges in a handful of steps
/// for the well-behaved curves the presets use, and bisection picks up the flat
/// ones where the derivative vanishes.
fn solve_for_x(x: f32, x1: f32, x2: f32) -> f32 {
    const EPSILON: f32 = 1e-5;

    let mut t = x;
    for _ in 0..8 {
        let error = bezier(t, x1, x2) - x;
        if error.abs() < EPSILON {
            return t;
        }
        let slope = bezier_slope(t, x1, x2);
        if slope.abs() < 1e-6 {
            break;
        }
        t -= error / slope;
    }

    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    let mut t = x.clamp(0.0, 1.0);
    for _ in 0..24 {
        let at = bezier(t, x1, x2);
        if (at - x).abs() < EPSILON {
            break;
        }
        if at < x {
            low = t;
        } else {
            high = t;
        }
        t = f32::midpoint(low, high);
    }
    t
}

fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    bezier(solve_for_x(t, x1, x2), y1, y2)
}

/// One point on a parameter's curve (§24).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Where in the **source media** this key sits. See the module docs.
    pub time: MediaTime,
    pub value: f32,
    /// How the value leaves this key on its way to the next one.
    #[serde(default)]
    pub interpolation: Interpolation,
}

impl Keyframe {
    pub fn new(time: MediaTime, value: f32, interpolation: Interpolation) -> Self {
        Self {
            time,
            value,
            interpolation,
        }
    }
}

/// Every key for one parameter, kept sorted by time (§8's `KeyframeTrack`).
///
/// The sort order is an invariant, not a convenience: evaluation binary-searches
/// it, and a track that arrived out of order from a hand-edited project file
/// would silently interpolate backwards. [`KeyframeTrack::sorted`] is the only
/// way to build one from arbitrary input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyframeTrack {
    pub parameter: AnimatedParameter,
    keys: Vec<Keyframe>,
}

impl KeyframeTrack {
    pub fn new(parameter: AnimatedParameter) -> Self {
        Self {
            parameter,
            keys: Vec::new(),
        }
    }

    /// Build from arbitrary keys, sorting them and dropping duplicate times.
    ///
    /// Later duplicates win, matching what [`Self::set`] does for a key written
    /// over an existing one.
    pub fn sorted(parameter: AnimatedParameter, keys: impl IntoIterator<Item = Keyframe>) -> Self {
        let mut track = Self::new(parameter);
        for key in keys {
            track.set(key);
        }
        track
    }

    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Add a key, or replace the one already at that instant.
    ///
    /// Returns what it replaced, which is what undo needs.
    pub fn set(&mut self, key: Keyframe) -> Option<Keyframe> {
        match self.index_of(key.time) {
            Ok(index) => Some(std::mem::replace(&mut self.keys[index], key)),
            Err(index) => {
                self.keys.insert(index, key);
                None
            }
        }
    }

    pub fn remove(&mut self, time: MediaTime) -> Option<Keyframe> {
        match self.index_of(time) {
            Ok(index) => Some(self.keys.remove(index)),
            Err(_) => None,
        }
    }

    pub fn get(&self, time: MediaTime) -> Option<Keyframe> {
        self.index_of(time).ok().map(|index| self.keys[index])
    }

    fn index_of(&self, time: MediaTime) -> Result<usize, usize> {
        self.keys
            .binary_search_by_key(&time.ticks(), |k| k.time.ticks())
    }

    /// The nearest key strictly before `time`.
    pub fn key_before(&self, time: MediaTime) -> Option<Keyframe> {
        self.keys.iter().rev().find(|key| key.time < time).copied()
    }

    /// The nearest key strictly after `time`.
    pub fn key_after(&self, time: MediaTime) -> Option<Keyframe> {
        self.keys.iter().find(|key| key.time > time).copied()
    }

    /// The value at `time`, or `None` when the track has no keys.
    ///
    /// Outside the outermost keys the value is held rather than extrapolated.
    /// Extrapolating would let a clip fade past invisible or scale past the
    /// slider's ceiling simply because the playhead moved somewhere the user
    /// never animated.
    pub fn value_at(&self, time: MediaTime) -> Option<f32> {
        let first = self.keys.first()?;
        if time <= first.time {
            return Some(first.value);
        }
        // Non-empty, so `last` cannot fail.
        let last = self.keys.last()?;
        if time >= last.time {
            return Some(last.value);
        }

        let index = match self.index_of(time) {
            Ok(index) => return Some(self.keys[index].value),
            // `time` is past the first key, so the insertion point is at least
            // 1 and the key before it exists.
            Err(index) => index - 1,
        };

        let from = self.keys[index];
        let to = self.keys[index + 1];
        let span = to.time.ticks() - from.time.ticks();
        if span <= 0 {
            return Some(to.value);
        }

        // §74 bans floating point in timeline *position* arithmetic; this is a
        // ratio between two positions that were subtracted as integers, which
        // is the one place a fraction is the answer being asked for.
        let progress = (time.ticks() - from.time.ticks()) as f64 / span as f64;
        let eased = from.interpolation.ease(progress as f32);
        Some(from.value + (to.value - from.value) * eased)
    }
}

/// Every animated parameter of one clip (§8's `keyframes: Vec<KeyframeTrack>`).
///
/// Serializes as a plain array, so a clip with no animation costs `[]` in the
/// project file and older projects load as empty without a migration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keyframes {
    tracks: Vec<KeyframeTrack>,
}

impl Keyframes {
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(KeyframeTrack::is_empty)
    }

    pub fn iter(&self) -> impl Iterator<Item = &KeyframeTrack> {
        self.tracks.iter()
    }

    pub fn track(&self, parameter: AnimatedParameter) -> Option<&KeyframeTrack> {
        self.tracks.iter().find(|t| t.parameter == parameter)
    }

    /// Whether this parameter is driven by keys rather than its static field.
    pub fn is_animated(&self, parameter: AnimatedParameter) -> bool {
        self.track(parameter).is_some_and(|t| !t.is_empty())
    }

    /// Total number of keys across every parameter.
    pub fn len(&self) -> usize {
        self.tracks.iter().map(KeyframeTrack::len).sum()
    }

    pub fn value_at(&self, parameter: AnimatedParameter, time: MediaTime) -> Option<f32> {
        self.track(parameter)?.value_at(time)
    }

    pub fn get(&self, parameter: AnimatedParameter, time: MediaTime) -> Option<Keyframe> {
        self.track(parameter)?.get(time)
    }

    /// Set a key, returning the one it replaced.
    pub fn set(&mut self, parameter: AnimatedParameter, key: Keyframe) -> Option<Keyframe> {
        match self.tracks.iter_mut().find(|t| t.parameter == parameter) {
            Some(track) => track.set(key),
            None => {
                let mut track = KeyframeTrack::new(parameter);
                track.set(key);
                self.tracks.push(track);
                None
            }
        }
    }

    /// Remove a key, returning it.
    ///
    /// A track left with no keys is dropped, so `is_animated` goes back to
    /// false and the project file does not accumulate empty tracks for every
    /// parameter the user has ever touched.
    pub fn remove(&mut self, parameter: AnimatedParameter, time: MediaTime) -> Option<Keyframe> {
        let index = self.tracks.iter().position(|t| t.parameter == parameter)?;
        let removed = self.tracks[index].remove(time);
        if self.tracks[index].is_empty() {
            self.tracks.remove(index);
        }
        removed
    }

    /// Every key time in the whole clip, sorted and deduplicated.
    ///
    /// What a keyframe row on the timeline draws, and what "jump to the next
    /// key" navigates: the user thinks in keys on a clip, not per parameter.
    pub fn times(&self) -> Vec<MediaTime> {
        let mut times: Vec<MediaTime> = self
            .tracks
            .iter()
            .flat_map(|t| t.keys().iter().map(|k| k.time))
            .collect();
        times.sort_unstable_by_key(|time| time.ticks());
        times.dedup();
        times
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ticks: i64) -> MediaTime {
        MediaTime::from_ticks(ticks)
    }

    fn track(keys: &[(i64, f32)], interpolation: Interpolation) -> KeyframeTrack {
        KeyframeTrack::sorted(
            AnimatedParameter::Opacity,
            keys.iter()
                .map(|&(t, v)| Keyframe::new(at(t), v, interpolation)),
        )
    }

    #[test]
    fn an_empty_track_has_no_value() {
        let track = KeyframeTrack::new(AnimatedParameter::Opacity);
        assert_eq!(track.value_at(at(0)), None);
    }

    /// A single key is a constant, not a ramp from nowhere.
    #[test]
    fn one_key_holds_everywhere() {
        let track = track(&[(1000, 0.5)], Interpolation::Linear);
        assert_eq!(track.value_at(at(0)), Some(0.5));
        assert_eq!(track.value_at(at(1000)), Some(0.5));
        assert_eq!(track.value_at(at(999_999)), Some(0.5));
    }

    #[test]
    fn linear_interpolation_hits_the_midpoint() {
        let track = track(&[(0, 0.0), (100, 1.0)], Interpolation::Linear);
        assert_eq!(track.value_at(at(0)), Some(0.0));
        assert_eq!(track.value_at(at(100)), Some(1.0));
        let mid = track.value_at(at(50)).expect("keyed");
        assert!((mid - 0.5).abs() < 1e-6, "midpoint was {mid}");
    }

    /// Outside the keys the value is held. Extrapolating would fade a clip past
    /// invisible wherever the user had not animated.
    #[test]
    fn values_are_held_outside_the_outermost_keys() {
        let track = track(&[(100, 0.2), (200, 0.8)], Interpolation::Linear);
        assert_eq!(track.value_at(at(0)), Some(0.2));
        assert_eq!(track.value_at(at(10_000)), Some(0.8));
    }

    #[test]
    fn hold_keeps_the_earlier_value_until_the_next_key() {
        let track = track(&[(0, 0.0), (100, 1.0)], Interpolation::Hold);
        assert_eq!(track.value_at(at(1)), Some(0.0));
        assert_eq!(track.value_at(at(99)), Some(0.0));
        assert_eq!(track.value_at(at(100)), Some(1.0));
    }

    /// The easing belongs to the key the segment leaves, so the *last* key's
    /// easing never affects anything before it.
    #[test]
    fn easing_comes_from_the_earlier_key() {
        let mut track = KeyframeTrack::new(AnimatedParameter::Opacity);
        track.set(Keyframe::new(at(0), 0.0, Interpolation::Hold));
        track.set(Keyframe::new(at(100), 1.0, Interpolation::Linear));
        assert_eq!(track.value_at(at(50)), Some(0.0), "hold should win here");
    }

    /// Every easing must start and end where the keys do, or a curve change
    /// would visibly jump the value at the keyframes themselves.
    #[test]
    fn every_easing_passes_through_both_keys() {
        for interpolation in Interpolation::PRESETS {
            let track = track(&[(0, 0.25), (960_000, 0.75)], interpolation);
            assert_eq!(
                track.value_at(at(0)),
                Some(0.25),
                "{} moved the first key",
                interpolation.label()
            );
            assert_eq!(
                track.value_at(at(960_000)),
                Some(0.75),
                "{} moved the last key",
                interpolation.label()
            );
        }
    }

    /// Ease-in starts slow: at the halfway point it must still be below the
    /// linear value, and ease-out above it. This is the assertion that catches
    /// reading `y` at `t = x` instead of solving for the curve parameter.
    #[test]
    fn ease_in_lags_and_ease_out_leads() {
        let midpoint = |curve| {
            track(&[(0, 0.0), (100, 1.0)], curve)
                .value_at(at(50))
                .expect("keyed")
        };
        let linear = midpoint(Interpolation::Linear);
        assert!(
            midpoint(Interpolation::EaseIn) < linear - 0.05,
            "ease in did not lag"
        );
        assert!(
            midpoint(Interpolation::EaseOut) > linear + 0.05,
            "ease out did not lead"
        );
        // Symmetric by construction.
        let in_out = midpoint(Interpolation::EaseInOut);
        assert!((in_out - 0.5).abs() < 1e-3, "ease in-out was {in_out}");
    }

    /// Monotonic easings must not overshoot; the presets are all monotonic.
    #[test]
    fn presets_stay_between_the_two_key_values() {
        for interpolation in Interpolation::PRESETS {
            for step in 0..=100 {
                let value = track(&[(0, 0.0), (100, 1.0)], interpolation)
                    .value_at(at(step))
                    .expect("keyed");
                assert!(
                    (-1e-4..=1.0 + 1e-4).contains(&value),
                    "{} left the range at {step}: {value}",
                    interpolation.label()
                );
            }
        }
    }

    #[test]
    fn setting_a_key_at_an_existing_time_replaces_it() {
        let mut track = KeyframeTrack::new(AnimatedParameter::Blur);
        assert!(
            track
                .set(Keyframe::new(at(10), 1.0, Interpolation::Linear))
                .is_none()
        );
        let replaced = track.set(Keyframe::new(at(10), 2.0, Interpolation::Hold));
        assert_eq!(replaced.map(|k| k.value), Some(1.0));
        assert_eq!(track.len(), 1);
        assert_eq!(track.value_at(at(10)), Some(2.0));
    }

    /// Keys arrive in whatever order the user places them; evaluation
    /// binary-searches, so the order has to be an invariant of the type.
    #[test]
    fn keys_are_kept_sorted_however_they_arrive() {
        let track = track(&[(300, 0.3), (100, 0.1), (200, 0.2)], Interpolation::Linear);
        let times: Vec<i64> = track.keys().iter().map(|k| k.time.ticks()).collect();
        assert_eq!(times, vec![100, 200, 300]);
    }

    #[test]
    fn removing_the_last_key_stops_the_parameter_being_animated() {
        let mut keyframes = Keyframes::default();
        keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(at(0), 0.0, Interpolation::Linear),
        );
        assert!(keyframes.is_animated(AnimatedParameter::Opacity));
        assert!(
            keyframes
                .remove(AnimatedParameter::Opacity, at(0))
                .is_some()
        );
        assert!(!keyframes.is_animated(AnimatedParameter::Opacity));
        assert!(keyframes.is_empty());
    }

    #[test]
    fn parameters_animate_independently() {
        let mut keyframes = Keyframes::default();
        keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(at(0), 0.25, Interpolation::Linear),
        );
        keyframes.set(
            AnimatedParameter::Blur,
            Keyframe::new(at(0), 40.0, Interpolation::Linear),
        );
        assert_eq!(
            keyframes.value_at(AnimatedParameter::Opacity, at(500)),
            Some(0.25)
        );
        assert_eq!(
            keyframes.value_at(AnimatedParameter::Blur, at(500)),
            Some(40.0)
        );
        assert_eq!(
            keyframes.value_at(AnimatedParameter::Rotation, at(500)),
            None
        );
        assert_eq!(keyframes.len(), 2);
    }

    #[test]
    fn key_times_are_merged_across_parameters() {
        let mut keyframes = Keyframes::default();
        for (parameter, time) in [
            (AnimatedParameter::Opacity, 0),
            (AnimatedParameter::Opacity, 100),
            (AnimatedParameter::Blur, 100),
            (AnimatedParameter::Blur, 250),
        ] {
            keyframes.set(
                parameter,
                Keyframe::new(at(time), 1.0, Interpolation::Linear),
            );
        }
        let times: Vec<i64> = keyframes.times().iter().map(|t| t.ticks()).collect();
        assert_eq!(times, vec![0, 100, 250]);
    }

    #[test]
    fn navigation_finds_the_neighbouring_keys() {
        let track = track(&[(100, 0.0), (200, 1.0), (300, 0.0)], Interpolation::Linear);
        assert_eq!(track.key_before(at(200)).map(|k| k.time.ticks()), Some(100));
        assert_eq!(track.key_after(at(200)).map(|k| k.time.ticks()), Some(300));
        assert_eq!(track.key_before(at(100)), None);
        assert_eq!(track.key_after(at(300)), None);
    }

    /// Project files round-trip, including the custom curve.
    #[test]
    fn keyframes_round_trip_through_json() {
        let mut keyframes = Keyframes::default();
        keyframes.set(
            AnimatedParameter::PositionX,
            Keyframe::new(
                at(4800),
                -0.5,
                Interpolation::Bezier {
                    x1: 0.25,
                    y1: 0.1,
                    x2: 0.25,
                    y2: 1.0,
                },
            ),
        );
        let json = serde_json::to_string(&keyframes).expect("serialize");
        assert!(json.starts_with('['), "should be a bare array: {json}");
        assert_eq!(
            serde_json::from_str::<Keyframes>(&json).expect("deserialize"),
            keyframes
        );
    }

    /// A Bézier with an overshooting `y` handle is allowed to leave the range,
    /// because that is the whole point of one.
    #[test]
    fn a_custom_curve_may_overshoot() {
        let overshoot = Interpolation::Bezier {
            x1: 0.5,
            y1: 0.0,
            x2: 0.5,
            y2: 1.8,
        };
        let peak = (0..=100)
            .filter_map(|step| track(&[(0, 0.0), (100, 1.0)], overshoot).value_at(at(step)))
            .fold(f32::MIN, f32::max);
        assert!(peak > 1.0, "expected overshoot, peaked at {peak}");
    }
}

#[cfg(test)]
mod default_tests {
    use super::*;
    use crate::clip::{ColorAdjust, Transform};
    use crate::{ClipLook, VideoClip};

    /// The defaults have to be what a freshly created clip actually has, or
    /// "reset" would move a parameter somewhere the clip never was.
    #[test]
    fn the_defaults_match_a_new_clip() {
        let source = crate::SourceRange::new(
            bettercut_foundation::MediaTime::ZERO,
            bettercut_foundation::MediaTime::from_seconds(1),
        )
        .expect("range");
        let clip = VideoClip::new(
            bettercut_foundation::MediaId::new(),
            bettercut_foundation::TimelineTime::ZERO,
            source,
        )
        .expect("clip");

        for parameter in AnimatedParameter::ALL {
            let actual = clip.parameter(parameter);
            assert_eq!(
                actual,
                parameter.default_value(),
                "{} defaults to {actual} on a new clip but {} here",
                parameter.label(),
                parameter.default_value()
            );
            assert!(parameter.is_default(actual));
        }
    }

    /// Every default has to be a value the parameter is allowed to hold, or
    /// resetting would immediately be clamped to something else.
    #[test]
    fn every_default_is_inside_its_limits() {
        for parameter in AnimatedParameter::ALL {
            let value = parameter.default_value();
            assert_eq!(
                parameter.clamp(value),
                value,
                "{} clamps its own default",
                parameter.label()
            );
        }
    }

    /// A clip at every default renders as the identity, which is what makes
    /// "reset" mean "as if untouched" rather than "some other look".
    #[test]
    fn the_defaults_are_the_identity_look() {
        let look = ClipLook {
            transform: Transform::default(),
            opacity: AnimatedParameter::Opacity.default_value(),
            color: ColorAdjust::default(),
            blur: AnimatedParameter::Blur.default_value(),
        };
        assert!(look.transform.is_identity());
        assert!(look.color.is_identity());
        assert_eq!(look.opacity, 1.0);
        assert_eq!(look.blur, 0.0);
    }
}
