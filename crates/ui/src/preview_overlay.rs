//! Moving and scaling a clip directly in the preview.
//!
//! The Inspector's sliders set position and scale by number. That is precise
//! and it is the wrong tool for framing a shot: you are looking at the picture,
//! so the picture is where the handles belong.
//!
//! ## Where the box comes from
//!
//! Not from the transform alone. The compositor **fits** each source inside the
//! frame before the clip's own scale applies (`renderer::fit_scale`), so a 16:9
//! clip in a 9:16 sequence covers a wide, short band even at scale 1.0. The
//! handles have to sit on that band, which means the interface has to do the
//! same arithmetic the shader does.
//!
//! It does not repeat it: `fit_scale` is public from the renderer for exactly
//! this reason, and the rest is the inverse of `layer_uniform`'s matrix, worked
//! out once in [`layer_box`] and tested against the values that matrix produces.
//!
//! ## Why gestures dispatch through the editor
//!
//! §54: the UI never mutates `Project`. A drag calls `set_clip_value` like the
//! sliders do, with `continuing` true after the first frame, so the whole drag
//! collapses into one undo step (§11) — and so a keyed parameter is written as
//! a keyframe rather than a static value, without this module knowing which.

use bettercut_editor_core::ClipProperty;
use bettercut_editor_core::foundation::ClipId;
use bettercut_editor_core::timeline::{Transform, Vec2};

/// How close to a corner counts as grabbing it, in canvas pixels.
const HANDLE_RADIUS: f32 = 7.0;
/// Drawn radius of the corner circles.
const HANDLE_DRAW: f32 = 5.5;
/// Scale may not go below this, matching the model's own floor: zero renders
/// nothing and cannot be dragged back out of.
const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 10.0;

/// Which corner is being dragged, in the order [`corners`] returns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomRight,
        Self::BottomLeft,
    ];
}

/// A drag in progress on the preview.
#[derive(Debug, Clone, Copy)]
pub struct PreviewDrag {
    pub clip: ClipId,
    pub gesture: Gesture,
    /// False until the pointer has actually moved, so a click that happens to
    /// land on the picture does not write an undo entry.
    pub started: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Gesture {
    /// Moving the whole clip. Holds where it started so the drag is absolute
    /// rather than an accumulation of per-frame deltas, which would drift.
    Move { from: Vec2, grab: egui::Pos2 },
    Scale {
        corner: Corner,
        from: Vec2,
        /// Distance from the box centre to the pointer when the drag began.
        grab_distance: f32,
    },
}

/// Where a clip's picture sits inside the output frame, in 0..1 frame units.
///
/// The inverse of `layer_uniform`'s matrix at zero rotation. That matrix maps
/// the unit quad through
///
/// ```text
/// a  = 2 · fit_x · scale_x        tx = 2·position.x − a·anchor.x
/// d  = −2 · fit_y · scale_y       ty = −2·position.y − d·anchor.y
/// ```
///
/// into clip space, and clip space maps to frame units by `(x + 1) / 2` across
/// and `(1 − y) / 2` down. Composing those gives a box centred on
/// `0.5 + position` with half-extents `fit · scale / 2`, which is what this
/// returns.
///
/// Rotation is deliberately ignored: the handles are axis-aligned, so a rotated
/// clip gets a box around where it would be unrotated. Rotating from the canvas
/// needs its own handle and its own gesture; until then the Inspector's slider
/// is the honest way to do it.
pub fn layer_box(source_aspect: f32, output_aspect: f32, transform: Transform) -> egui::Rect {
    let (fit_x, fit_y) = bettercut_renderer::fit_scale(source_aspect, output_aspect);
    let half = egui::vec2(
        fit_x * transform.scale.x / 2.0,
        fit_y * transform.scale.y / 2.0,
    );
    let centre = egui::pos2(0.5 + transform.position.x, 0.5 + transform.position.y);
    egui::Rect::from_center_size(centre, half * 2.0)
}

/// The four corners of a box, in [`Corner::ALL`] order.
pub fn corners(box_: egui::Rect) -> [egui::Pos2; 4] {
    [
        box_.left_top(),
        box_.right_top(),
        box_.right_bottom(),
        box_.left_bottom(),
    ]
}

/// Which corner the pointer is on, if any.
pub fn corner_at(box_on_canvas: egui::Rect, pointer: egui::Pos2) -> Option<Corner> {
    corners(box_on_canvas)
        .iter()
        .zip(Corner::ALL)
        .find(|(at, _)| at.distance(pointer) <= HANDLE_RADIUS)
        .map(|(_, corner)| corner)
}

/// Map a box in 0..1 frame units onto the canvas rectangle on screen.
pub fn to_canvas(box_: egui::Rect, canvas: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            canvas.left() + box_.left() * canvas.width(),
            canvas.top() + box_.top() * canvas.height(),
        ),
        egui::pos2(
            canvas.left() + box_.right() * canvas.width(),
            canvas.top() + box_.bottom() * canvas.height(),
        ),
    )
}

/// The position a move gesture has reached.
///
/// Absolute from where the drag started, not a sum of per-frame deltas: the
/// pointer can outrun a frame, and accumulating would leave the picture behind
/// the cursor by however much was dropped.
pub fn moved_position(from: Vec2, grab: egui::Pos2, now: egui::Pos2, canvas: egui::Rect) -> Vec2 {
    let dx = (now.x - grab.x) / canvas.width().max(1.0);
    let dy = (now.y - grab.y) / canvas.height().max(1.0);
    Vec2::new(from.x + dx, from.y + dy)
}

/// The scale a corner drag has reached.
///
/// Uniform, about the box's centre, from how much further the pointer is than
/// when it was grabbed. Dragging a corner is a request to resize the picture,
/// not to reshape it — a squashed clip is nearly always a mistake, and the
/// Inspector has separate axes for when it is not.
pub fn scaled(from: Vec2, grab_distance: f32, centre: egui::Pos2, now: egui::Pos2) -> Vec2 {
    // Below a pixel the ratio is noise: grabbing exactly at the centre would
    // otherwise divide by nearly zero and fling the scale to its ceiling.
    if grab_distance < 1.0 {
        return from;
    }
    let ratio = centre.distance(now) / grab_distance;
    Vec2::new(
        (from.x * ratio).clamp(MIN_SCALE, MAX_SCALE),
        (from.y * ratio).clamp(MIN_SCALE, MAX_SCALE),
    )
}

/// Draw the box and its four corner circles.
pub fn draw(painter: &egui::Painter, box_on_canvas: egui::Rect, active: bool) {
    let colour = if active {
        crate::theme::SELECTION
    } else {
        crate::theme::CLIP_TEXT
    };

    painter.rect_stroke(
        box_on_canvas,
        0,
        egui::Stroke::new(1.5, colour),
        egui::StrokeKind::Middle,
    );

    for at in corners(box_on_canvas) {
        // Filled, with a dark ring: on bright footage a plain light circle
        // disappears, and on dark footage a plain dark one does.
        painter.circle_filled(at, HANDLE_DRAW + 1.0, egui::Color32::from_black_alpha(160));
        painter.circle_filled(at, HANDLE_DRAW, colour);
    }
}

/// The command a gesture has produced this frame.
pub fn property_for(
    gesture: Gesture,
    canvas: egui::Rect,
    box_on_canvas: egui::Rect,
    now: egui::Pos2,
) -> ClipProperty {
    match gesture {
        Gesture::Move { from, grab } => {
            let at = moved_position(from, grab, now, canvas);
            ClipProperty::Position { x: at.x, y: at.y }
        }
        Gesture::Scale {
            from,
            grab_distance,
            ..
        } => {
            let at = scaled(from, grab_distance, box_on_canvas.center(), now);
            ClipProperty::Scale { x: at.x, y: at.y }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(px: f32, py: f32, scale: f32) -> Transform {
        Transform {
            position: Vec2::new(px, py),
            scale: Vec2::new(scale, scale),
            ..Transform::default()
        }
    }

    /// A source the same shape as the frame fills it exactly at scale 1.
    #[test]
    fn a_matching_source_fills_the_frame() {
        let box_ = layer_box(16.0 / 9.0, 16.0 / 9.0, transform(0.0, 0.0, 1.0));
        assert!((box_.left() - 0.0).abs() < 1e-5, "left {}", box_.left());
        assert!((box_.right() - 1.0).abs() < 1e-5, "right {}", box_.right());
        assert!((box_.top() - 0.0).abs() < 1e-5);
        assert!((box_.bottom() - 1.0).abs() < 1e-5);
    }

    /// The case the handles exist for: a wide clip in a vertical sequence
    /// covers a band across the middle, not the whole frame.
    #[test]
    fn a_wide_source_in_a_vertical_frame_is_a_band() {
        let box_ = layer_box(16.0 / 9.0, 9.0 / 16.0, transform(0.0, 0.0, 1.0));

        assert!((box_.width() - 1.0).abs() < 1e-5, "should span the width");
        // 9:16 frame, 16:9 source: the picture is (9/16)/(16/9) = 0.316 tall.
        assert!(
            (box_.height() - 0.316).abs() < 0.01,
            "height {}",
            box_.height()
        );
        // And centred.
        assert!((box_.center().y - 0.5).abs() < 1e-5);
    }

    #[test]
    fn scale_and_position_move_the_box_the_way_they_read() {
        let doubled = layer_box(1.0, 1.0, transform(0.0, 0.0, 2.0));
        assert!((doubled.width() - 2.0).abs() < 1e-5);
        assert!((doubled.center().x - 0.5).abs() < 1e-5, "still centred");

        // Position is in frame widths, and y is down.
        let moved = layer_box(1.0, 1.0, transform(0.25, -0.1, 1.0));
        assert!((moved.center().x - 0.75).abs() < 1e-5);
        assert!((moved.center().y - 0.4).abs() < 1e-5);
    }

    /// Dragging must track the pointer exactly: half the canvas across is half
    /// a frame width, whatever the canvas happens to be on screen.
    #[test]
    fn moving_tracks_the_pointer_in_frame_units() {
        let canvas = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 450.0));
        let at = moved_position(
            Vec2::ZERO,
            egui::pos2(500.0, 275.0),
            egui::pos2(900.0, 275.0),
            canvas,
        );
        assert!((at.x - 0.5).abs() < 1e-5, "x {}", at.x);
        assert!(at.y.abs() < 1e-5);

        // From a starting offset, the drag adds to it rather than replacing it.
        let from = Vec2::new(-0.25, 0.1);
        let at = moved_position(
            from,
            egui::pos2(500.0, 275.0),
            egui::pos2(700.0, 500.0),
            canvas,
        );
        assert!((at.x - 0.0).abs() < 1e-5, "x {}", at.x);
        assert!((at.y - 0.6).abs() < 1e-5, "y {}", at.y);
    }

    /// Dragging a corner outward doubles the scale when the pointer is twice as
    /// far from the centre, and inward halves it.
    #[test]
    fn scaling_follows_the_distance_from_the_centre() {
        let centre = egui::pos2(400.0, 300.0);
        let from = Vec2::new(1.0, 1.0);

        let out = scaled(from, 100.0, centre, egui::pos2(600.0, 300.0));
        assert!((out.x - 2.0).abs() < 1e-4, "out {}", out.x);

        let in_ = scaled(from, 100.0, centre, egui::pos2(450.0, 300.0));
        assert!((in_.x - 0.5).abs() < 1e-4, "in {}", in_.x);

        // Uniform: both axes move together.
        assert!((out.x - out.y).abs() < 1e-6);
    }

    /// Grabbing at the centre would divide by nearly nothing. The scale must
    /// stay where it was rather than jumping to the ceiling.
    #[test]
    fn a_degenerate_grab_does_not_fling_the_scale() {
        let from = Vec2::new(1.0, 1.0);
        let at = scaled(
            from,
            0.0,
            egui::pos2(400.0, 300.0),
            egui::pos2(900.0, 800.0),
        );
        assert_eq!((at.x, at.y), (1.0, 1.0));
    }

    /// Scale is clamped where the model clamps it, so the handles cannot put a
    /// clip somewhere the sliders could not.
    #[test]
    fn scaling_stays_inside_the_models_limits() {
        let centre = egui::pos2(400.0, 300.0);
        let huge = scaled(Vec2::new(1.0, 1.0), 1.0, centre, egui::pos2(9000.0, 300.0));
        assert!(huge.x <= MAX_SCALE, "{}", huge.x);

        let tiny = scaled(
            Vec2::new(1.0, 1.0),
            10_000.0,
            centre,
            egui::pos2(400.5, 300.0),
        );
        assert!(tiny.x >= MIN_SCALE, "{}", tiny.x);
    }

    #[test]
    fn corners_are_found_within_the_grab_radius() {
        let box_ = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 150.0));

        assert_eq!(
            corner_at(box_, egui::pos2(100.0, 100.0)),
            Some(Corner::TopLeft)
        );
        assert_eq!(
            corner_at(box_, egui::pos2(300.0, 250.0)),
            Some(Corner::BottomRight)
        );
        // Just inside the radius.
        assert_eq!(
            corner_at(box_, egui::pos2(104.0, 103.0)),
            Some(Corner::TopLeft)
        );
        // Well inside the box, but not on a corner: that is a move, not a scale.
        assert_eq!(corner_at(box_, egui::pos2(200.0, 175.0)), None);
    }

    /// The canvas mapping has to survive a letterboxed preview, where the
    /// canvas is not at the window origin.
    #[test]
    fn frame_units_map_onto_an_offset_canvas() {
        let canvas = egui::Rect::from_min_size(egui::pos2(40.0, 12.0), egui::vec2(640.0, 360.0));
        let whole = to_canvas(
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            canvas,
        );
        assert!((whole.left() - 40.0).abs() < 1e-4);
        assert!((whole.right() - 680.0).abs() < 1e-4);
        assert!((whole.bottom() - 372.0).abs() < 1e-4);

        let middle = to_canvas(
            egui::Rect::from_min_max(egui::pos2(0.25, 0.5), egui::pos2(0.75, 1.0)),
            canvas,
        );
        assert!((middle.left() - 200.0).abs() < 1e-4);
        assert!((middle.top() - 192.0).abs() < 1e-4);
    }
}
