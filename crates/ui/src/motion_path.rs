//! The motion path: a clip's keyed position drawn over the picture as the
//! line it will travel, with a handle at every key.
//!
//! Keyframed position is otherwise invisible until it plays: the Inspector
//! shows one number at one instant, and the shape of the move — where it
//! starts, where it ends, whether it bows — is in the editor's head only.
//! Drawn on the preview it is a line, and a line can be picked up: dragging a
//! handle moves that key, at its own time, without scrubbing to it first.
//!
//! Pure geometry here, so it can be tested without a window; the drawing and
//! the drag live with the rest of the preview's handles.

use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::timeline::{AnimatedParameter, Vec2, VideoClip};

/// How close a pointer has to be to a key's handle, in points.
pub const PATH_GRAB_PIXELS: f32 = 8.0;

/// How many points are drawn between two keys, so an eased move reads as the
/// curve it is rather than a straight line pretending.
pub const STEPS_PER_SEGMENT: usize = 12;

/// One key of the path: when, and where the picture's centre is then.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathKey {
    pub time: MediaTime,
    pub position: Vec2,
}

/// The keys of a clip's path, earliest first: every instant either axis has
/// a key at, with the position the clip actually takes there — so a key on
/// x alone still gets its y from the line y is following.
///
/// Empty for a clip whose position is not keyed: a still picture has no path.
pub fn path_keys(clip: &VideoClip) -> Vec<PathKey> {
    let mut times: Vec<MediaTime> = [AnimatedParameter::PositionX, AnimatedParameter::PositionY]
        .into_iter()
        .filter_map(|parameter| clip.keyframes.track(parameter))
        .flat_map(|track| track.keys().iter().map(|key| key.time))
        .collect();
    times.sort_by_key(|time| time.ticks());
    times.dedup();
    times
        .into_iter()
        .map(|time| PathKey {
            time,
            position: clip.look_at(time).transform.position,
        })
        .collect()
}

/// The path between the first key and the last, sampled `steps` times a
/// segment: what the clip's centre passes through as it plays.
pub fn path_curve(clip: &VideoClip, keys: &[PathKey], steps: usize) -> Vec<Vec2> {
    let steps = steps.max(1);
    let mut curve = Vec::new();
    for pair in keys.windows(2) {
        let (from, to) = (pair[0].time.ticks(), pair[1].time.ticks());
        for step in 0..steps {
            let ticks = from + (to - from) * step as i64 / steps as i64;
            curve.push(
                clip.look_at(MediaTime::from_ticks(ticks))
                    .transform
                    .position,
            );
        }
    }
    if let Some(last) = keys.last() {
        curve.push(last.position);
    }
    curve
}

/// Where a position lands on the canvas: the picture's centre, which is what
/// a position *is* — half a frame across plus the offset, in frame units.
pub fn on_canvas(position: Vec2, canvas: egui::Rect) -> egui::Pos2 {
    egui::pos2(
        canvas.left() + (0.5 + position.x) * canvas.width(),
        canvas.top() + (0.5 + position.y) * canvas.height(),
    )
}

/// The inverse: the position whose centre would sit at `at`.
pub fn position_from_canvas(at: egui::Pos2, canvas: egui::Rect) -> Vec2 {
    Vec2::new(
        (at.x - canvas.left()) / canvas.width().max(1.0) - 0.5,
        (at.y - canvas.top()) / canvas.height().max(1.0) - 0.5,
    )
}

/// Which key's handle `pointer` is on, if any; the nearest when two overlap.
pub fn key_at(keys: &[PathKey], canvas: egui::Rect, pointer: egui::Pos2) -> Option<usize> {
    keys.iter()
        .enumerate()
        .map(|(index, key)| (index, on_canvas(key.position, canvas).distance(pointer)))
        .filter(|(_, distance)| *distance <= PATH_GRAB_PIXELS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// The path and its handles, the key at `current` (the one under the
/// playhead) drawn filled so the eye finds "now" on the line.
pub fn draw(
    painter: &egui::Painter,
    keys: &[PathKey],
    curve: &[Vec2],
    canvas: egui::Rect,
    current: Option<usize>,
) {
    let colour = crate::theme::selection();
    let points: Vec<egui::Pos2> = curve.iter().map(|p| on_canvas(*p, canvas)).collect();
    if points.len() > 1 {
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.5, colour.gamma_multiply(0.8)),
        ));
    }
    for (index, key) in keys.iter().enumerate() {
        let at = on_canvas(key.position, canvas);
        painter.circle_filled(at, 5.0, egui::Color32::from_black_alpha(160));
        if current == Some(index) {
            painter.circle_filled(at, 4.0, colour);
        } else {
            painter.circle_stroke(at, 4.0, egui::Stroke::new(2.0, colour));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::foundation::{MediaId, TimelineTime};
    use bettercut_editor_core::timeline::{Interpolation, Keyframe, SourceRange};

    fn key(seconds: i64, value: f32) -> Keyframe {
        Keyframe::new(
            MediaTime::from_seconds(seconds),
            value,
            Interpolation::Linear,
        )
    }

    /// A ten-second shot moving from the left of the frame to the right, with
    /// one y key of its own half way.
    fn moving() -> VideoClip {
        let mut clip = VideoClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
        )
        .unwrap();
        clip.keyframes
            .set(AnimatedParameter::PositionX, key(0, -0.4));
        clip.keyframes
            .set(AnimatedParameter::PositionX, key(8, 0.4));
        clip.keyframes
            .set(AnimatedParameter::PositionY, key(4, 0.2));
        clip
    }

    fn canvas() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 450.0))
    }

    #[test]
    fn a_still_picture_has_no_path() {
        let clip = VideoClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
        )
        .unwrap();
        assert!(path_keys(&clip).is_empty());
        assert!(path_curve(&clip, &[], 4).is_empty());
    }

    /// Every key of either axis is a key of the path, in time order, at the
    /// position the clip really takes there.
    #[test]
    fn the_keys_are_every_instant_either_axis_has_one() {
        let keys = path_keys(&moving());
        let seconds: Vec<i64> = keys.iter().map(|k| k.time.ticks() / 960_000).collect();
        assert_eq!(seconds, vec![0, 4, 8]);
        assert!((keys[0].position.x + 0.4).abs() < 1e-6);
        // Half way along x's line at the y key's own time.
        assert!(keys[1].position.x.abs() < 1e-6, "{:?}", keys[1]);
        assert!((keys[1].position.y - 0.2).abs() < 1e-6);
        assert!((keys[2].position.x - 0.4).abs() < 1e-6);
    }

    /// The curve starts at the first key, ends at the last, and has a point
    /// for every step between.
    #[test]
    fn the_curve_runs_from_the_first_key_to_the_last() {
        let clip = moving();
        let keys = path_keys(&clip);
        let curve = path_curve(&clip, &keys, 4);
        assert_eq!(curve.len(), 2 * 4 + 1);
        assert_eq!(curve[0], keys[0].position);
        assert_eq!(*curve.last().unwrap(), keys[2].position);
    }

    /// Canvas and position are two ways of saying the same place.
    #[test]
    fn canvas_and_position_round_trip() {
        for position in [
            Vec2::new(0.0, 0.0),
            Vec2::new(-0.4, 0.2),
            Vec2::new(0.5, -0.5),
        ] {
            let at = on_canvas(position, canvas());
            let back = position_from_canvas(at, canvas());
            assert!((back.x - position.x).abs() < 1e-5 && (back.y - position.y).abs() < 1e-5);
        }
        // The middle of the frame is the middle of the canvas.
        assert_eq!(on_canvas(Vec2::new(0.0, 0.0), canvas()), canvas().center());
    }

    /// A handle is hit within its reach and not outside it; the nearest wins.
    #[test]
    fn a_key_is_found_under_the_pointer() {
        let keys = path_keys(&moving());
        let first = on_canvas(keys[0].position, canvas());
        assert_eq!(key_at(&keys, canvas(), first), Some(0));
        assert_eq!(
            key_at(
                &keys,
                canvas(),
                first + egui::vec2(PATH_GRAB_PIXELS - 1.0, 0.0)
            ),
            Some(0)
        );
        assert_eq!(
            key_at(
                &keys,
                canvas(),
                first + egui::vec2(PATH_GRAB_PIXELS * 3.0, 0.0)
            ),
            None
        );
    }
}
