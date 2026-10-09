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
use bettercut_editor_core::timeline::{Crop, MIN_CROP_REMAINING, Transform, Vec2};

/// How close to a corner counts as grabbing it, in canvas pixels.
const HANDLE_RADIUS: f32 = 7.0;
/// How far above the top edge the rotate handle sits, in canvas pixels. Far
/// enough that it is not mistaken for a corner, near enough to read as part of
/// the box.
const ROTATE_OFFSET: f32 = 24.0;
/// Within this many degrees of a right angle, a rotate drag lands exactly on it.
/// Level and upright are what a turn is usually aiming for, and a hand cannot
/// hit 0.0° on its own.
const ROTATE_SNAP: f32 = 3.0;
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
    /// Turning the clip about its centre, from the handle above the box.
    Rotate {
        /// The rotation the clip had when the drag began, in degrees.
        from: f32,
        /// The pointer's angle around the centre at that moment. The drag
        /// adds how far it has turned since, so grabbing the handle does not
        /// snap the clip to wherever the pointer happens to be.
        grab_angle: f32,
    },
    Scale {
        corner: Corner,
        from: Vec2,
        /// Distance from the box centre to the pointer when the drag began.
        grab_distance: f32,
    },
    /// Sliding a mask across the picture it was drawn on.
    ///
    /// The mask's own centre, not the clip's: a mask is in the clip's frame, so
    /// moving the clip takes its mask along and this moves the mask *within*
    /// it.
    MaskMove {
        /// Where the mask's centre was when the drag began, in the clip's own
        /// 0–1 frame.
        from: [f32; 2],
        grab: egui::Pos2,
    },
    /// Resizing a mask from the handle at its edge.
    MaskResize {
        /// The mask's half-width and half-height when the drag began.
        from: [f32; 2],
        grab: egui::Pos2,
    },
    /// Dragging one corner of §45's pin: the picture's corner follows the
    /// pointer, and the other three stay where they are.
    ///
    /// The whole pin is captured at the press, so every frame of the drag is
    /// worked out from where it started rather than added to the last one —
    /// the same reasoning `Crop` gives.
    Pin {
        corner: Corner,
        from: bettercut_editor_core::timeline::CornerPin,
        /// Where that corner sat on the canvas before the drag, unpinned.
        origin: egui::Pos2,
    },
    /// Dragging one edge of §22's crop.
    ///
    /// Everything is captured at the press, because the picture does not hold
    /// still under the pointer: cropping changes its shape, and the renderer
    /// re-fits it every frame. Worked out against the live picture, the edge
    /// being dragged would run away from the hand. Against the picture as it was
    /// when the drag began, a pointer position means one crop and only one.
    Crop {
        edge: CropEdge,
        /// The crop when the drag began.
        from: Crop,
        /// The cropped picture's box when the drag began, unturned.
        picture: egui::Rect,
        /// The clip's rotation, which the picture turns about its own centre.
        degrees: f32,
    },
    /// A key of the motion path picked up: the key at `time` (source time),
    /// from `from`, grabbed at `grab` on the canvas
    /// (`crate::motion_path`).
    PathKey {
        time: bettercut_editor_core::foundation::MediaTime,
        from: Vec2,
        grab: egui::Pos2,
    },
    /// The pivot handle picked up: the point the picture turns about, moved
    /// without the picture moving (`editor_core::pivot`).
    Pivot,
}

/// Which side of the crop is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropEdge {
    Left,
    Top,
    Right,
    Bottom,
}

impl CropEdge {
    pub const ALL: [Self; 4] = [Self::Left, Self::Top, Self::Right, Self::Bottom];

    /// Where this edge's handle sits, in the cropped picture's own 0–1 frame:
    /// the middle of its side.
    fn picture_uv(self) -> [f32; 2] {
        match self {
            Self::Left => [0.0, 0.5],
            Self::Top => [0.5, 0.0],
            Self::Right => [1.0, 0.5],
            Self::Bottom => [0.5, 1.0],
        }
    }
}

/// Where a point of the clip's own frame lands on the canvas.
///
/// A mask is in the clip's frame — 0–1 across the picture — and the preview
/// shows that picture as a box which may be moved, scaled and turned. So a
/// mask's centre is found by walking into the box and then turning with it,
/// which is why this takes the rotation separately: `box_on_canvas` is the
/// *unturned* rectangle, as everything else in this module treats it.
/// Where the picture is drawn inside the preview's `area`.
///
/// Fit letterboxes it with a margin, as large as fits. A scale draws the
/// sequence's pixels at that many screen pixels each (`points_per_pixel`
/// screen points per frame pixel at 100%), centred and moved by `pan` — held
/// so the picture never slides entirely out of view: some of it always
/// covers the middle of the area. Returns the canvas and the pan actually
/// used.
/// The space kept clear round a fitted picture, on each side.
pub const PICTURE_MARGIN: f32 = 28.0;

pub fn preview_canvas(
    area: egui::Rect,
    resolution: (u32, u32),
    zoom: crate::state::PreviewZoom,
    pan: egui::Vec2,
    points_per_pixel: f32,
) -> (egui::Rect, egui::Vec2) {
    let aspect = (resolution.0.max(1) as f32 / resolution.1.max(1) as f32).max(0.01);
    match zoom {
        crate::state::PreviewZoom::Fit => {
            // A clear margin all round: room for the rotation handle above a
            // full-frame clip, and for the picture to sit on its ground rather
            // than against the panels.
            let margin = 2.0 * PICTURE_MARGIN;
            let mut size = egui::vec2(area.width() - margin, (area.width() - margin) / aspect);
            if size.y > area.height() - margin {
                size = egui::vec2((area.height() - margin) * aspect, area.height() - margin);
            }
            (
                egui::Rect::from_center_size(area.center(), size),
                egui::Vec2::ZERO,
            )
        }
        crate::state::PreviewZoom::Scale(scale) => {
            let size = egui::vec2(resolution.0 as f32, resolution.1 as f32)
                * scale.max(0.01)
                * points_per_pixel;
            // The middle of the area stays over the picture.
            let limit = size / 2.0;
            let pan = egui::vec2(
                pan.x.clamp(-limit.x, limit.x),
                pan.y.clamp(-limit.y, limit.y),
            );
            (egui::Rect::from_center_size(area.center() + pan, size), pan)
        }
    }
}

/// One thing a preview guide draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GuideShape {
    Line(egui::Pos2, egui::Pos2),
    /// An outline.
    Frame(egui::Rect),
    /// A part of the picture to keep clear, drawn darkened.
    Shade(egui::Rect),
}

/// The share of a vertical frame each phone app covers: the header along the
/// top, the caption and name along the bottom, and the column of buttons down
/// the right. Taken from the common safe-zone templates, rounded outwards.
pub const SOCIAL_TOP: f32 = 0.08;
pub const SOCIAL_BOTTOM: f32 = 0.2;
pub const SOCIAL_RIGHT: f32 = 0.13;
/// Where the button column starts and ends, down the frame.
pub const SOCIAL_BUTTONS: (f32, f32) = (0.35, 0.8);

/// What `guide` draws over a picture shown in `canvas`.
pub fn guide_shapes(guide: crate::state::PreviewGuide, canvas: egui::Rect) -> Vec<GuideShape> {
    use crate::state::PreviewGuide;
    let at = |x: f32, y: f32| {
        egui::pos2(
            canvas.left() + canvas.width() * x,
            canvas.top() + canvas.height() * y,
        )
    };
    let inset =
        |share: f32| egui::Rect::from_min_max(at(share, share), at(1.0 - share, 1.0 - share));
    match guide {
        PreviewGuide::Off => Vec::new(),
        PreviewGuide::Thirds => [1.0 / 3.0, 2.0 / 3.0]
            .into_iter()
            .flat_map(|t| {
                [
                    GuideShape::Line(at(t, 0.0), at(t, 1.0)),
                    GuideShape::Line(at(0.0, t), at(1.0, t)),
                ]
            })
            .collect(),
        PreviewGuide::TitleSafe => vec![
            GuideShape::Frame(inset(0.05)),
            GuideShape::Frame(inset(0.1)),
        ],
        // A tenth of the frame each way, centred: long enough to line a shot
        // up against, short enough to stay out of the picture.
        PreviewGuide::Centre => vec![
            GuideShape::Line(at(0.45, 0.5), at(0.55, 0.5)),
            GuideShape::Line(at(0.5, 0.45), at(0.5, 0.55)),
        ],
        PreviewGuide::Social => vec![
            GuideShape::Shade(egui::Rect::from_min_max(at(0.0, 0.0), at(1.0, SOCIAL_TOP))),
            GuideShape::Shade(egui::Rect::from_min_max(
                at(0.0, 1.0 - SOCIAL_BOTTOM),
                at(1.0, 1.0),
            )),
            GuideShape::Shade(egui::Rect::from_min_max(
                at(1.0 - SOCIAL_RIGHT, SOCIAL_BUTTONS.0),
                at(1.0, SOCIAL_BUTTONS.1),
            )),
            // The part that is always seen.
            GuideShape::Frame(egui::Rect::from_min_max(
                at(0.0, SOCIAL_TOP),
                at(1.0 - SOCIAL_RIGHT, 1.0 - SOCIAL_BOTTOM),
            )),
        ],
    }
}

/// Paint `guide` over the picture.
pub fn draw_guides(painter: &egui::Painter, guide: crate::state::PreviewGuide, canvas: egui::Rect) {
    let line = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(140));
    for shape in guide_shapes(guide, canvas) {
        match shape {
            GuideShape::Line(from, to) => {
                painter.line_segment([from, to], line);
            }
            GuideShape::Frame(rect) => {
                painter.rect_stroke(rect, 0, line, egui::StrokeKind::Inside);
            }
            GuideShape::Shade(rect) => {
                painter.rect_filled(rect, 0, egui::Color32::from_black_alpha(110));
            }
        }
    }
}

pub fn clip_point_on_canvas(uv: [f32; 2], box_on_canvas: egui::Rect, degrees: f32) -> egui::Pos2 {
    let inside = egui::pos2(
        box_on_canvas.left() + uv[0] * box_on_canvas.width(),
        box_on_canvas.top() + uv[1] * box_on_canvas.height(),
    );
    rotate_about(inside, box_on_canvas.center(), degrees)
}

/// The inverse: where a point on the canvas falls in the clip's own frame.
///
/// Exact enough to round-trip, which is what a drag needs — a mask that drifted
/// a little every time it was picked up would be worse than one that could not
/// be dragged at all.
pub fn canvas_point_in_clip(at: egui::Pos2, box_on_canvas: egui::Rect, degrees: f32) -> [f32; 2] {
    let unturned = rotate_about(at, box_on_canvas.center(), -degrees);
    let width = if box_on_canvas.width().abs() < f32::EPSILON {
        1.0
    } else {
        box_on_canvas.width()
    };
    let height = if box_on_canvas.height().abs() < f32::EPSILON {
        1.0
    } else {
        box_on_canvas.height()
    };
    [
        (unturned.x - box_on_canvas.left()) / width,
        (unturned.y - box_on_canvas.top()) / height,
    ]
}

/// Where a mask's centre ends up after a drag.
///
/// Absolute from where the drag began rather than an accumulation of per-frame
/// deltas, for the reason [`Gesture::Move`] gives: deltas drift.
pub fn mask_moved(
    from: [f32; 2],
    grab: egui::Pos2,
    now: egui::Pos2,
    box_on_canvas: egui::Rect,
    degrees: f32,
) -> [f32; 2] {
    let was = canvas_point_in_clip(grab, box_on_canvas, degrees);
    let is = canvas_point_in_clip(now, box_on_canvas, degrees);
    [from[0] + (is[0] - was[0]), from[1] + (is[1] - was[1])]
}

/// Where a *generated* layer's picture sits, in 0..1 frame units (§26).
///
/// A title is drawn at its own size rather than fitted to the canvas, so the
/// matrix the shader ends up with is not the clip's transform but the
/// natural-size correction of it. Working that out here rather than at the call
/// site is what lets it be checked without a window — and against the GPU,
/// which is where the two ever disagreeing would actually show.
pub fn generated_layer_box(
    transform: Transform,
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
) -> egui::Rect {
    let composited = bettercut_editor_core::timeline::natural_size_transform(
        transform,
        source_width,
        source_height,
        output_width,
        output_height,
    );
    layer_box(
        source_width.max(1) as f32 / source_height.max(1) as f32,
        output_width.max(1) as f32 / output_height.max(1) as f32,
        composited,
    )
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
/// Unrotated. Rotation is applied on top by the drawing and hit-testing below,
/// about the box's centre — the clip's anchor, which the interface never moves
/// off the centre.
pub fn layer_box(source_aspect: f32, output_aspect: f32, transform: Transform) -> egui::Rect {
    let (fit_x, fit_y) = bettercut_renderer::fit_scale(source_aspect, output_aspect);
    let half = egui::vec2(
        fit_x * transform.scale.x / 2.0,
        fit_y * transform.scale.y / 2.0,
    );
    // The position is where the *pivot* sits; the box's middle is that, plus
    // however far the pivot is from the middle of the picture.
    let centre = egui::pos2(
        0.5 + transform.position.x + (0.5 - transform.anchor.x) * half.x * 2.0,
        0.5 + transform.position.y + (0.5 - transform.anchor.y) * half.y * 2.0,
    );
    egui::Rect::from_center_size(centre, half * 2.0)
}

/// A point of the cropped picture's own 0–1 frame, as a point of the *whole
/// source*'s 0–1 frame.
///
/// The two frames the crop handles move between. The picture the user sees is
/// only the part the crop kept, so its left edge is the source's `crop.left`,
/// and its full width is the fraction the crop left across.
pub fn picture_to_source_uv(picture_uv: [f32; 2], crop: Crop) -> [f32; 2] {
    let (keep_x, keep_y) = crop.remaining();
    [
        crop.left + picture_uv[0] * keep_x,
        crop.top + picture_uv[1] * keep_y,
    ]
}

/// The inverse of [`picture_to_source_uv`].
///
/// Outside 0–1 for the parts of the source the crop removed, which is exactly
/// what drawing the uncropped outline needs.
pub fn source_to_picture_uv(source_uv: [f32; 2], crop: Crop) -> [f32; 2] {
    let (keep_x, keep_y) = crop.remaining();
    let along = |at: f32, from: f32, keep: f32| {
        if keep <= f32::EPSILON {
            0.0
        } else {
            (at - from) / keep
        }
    };
    [
        along(source_uv[0], crop.left, keep_x),
        along(source_uv[1], crop.top, keep_y),
    ]
}

/// Each crop edge's handle on the canvas: the middle of each side of the
/// picture as drawn, turned with the clip.
pub fn crop_edge_handles(picture: egui::Rect, degrees: f32) -> [(CropEdge, egui::Pos2); 4] {
    CropEdge::ALL.map(|edge| {
        (
            edge,
            clip_point_on_canvas(edge.picture_uv(), picture, degrees),
        )
    })
}

/// The crop edge whose handle is under `pointer`, if any.
pub fn crop_edge_at(picture: egui::Rect, degrees: f32, pointer: egui::Pos2) -> Option<CropEdge> {
    crop_edge_handles(picture, degrees)
        .into_iter()
        .find(|(_, at)| at.distance(pointer) <= HANDLE_RADIUS)
        .map(|(edge, _)| edge)
}

/// Where a clip's four corners are on the canvas once the pin has moved them.
///
/// The corners the plain box has, each shifted by its own offset — the same
/// arithmetic the shader does, so the handles sit on the picture rather than
/// near it.
pub fn pinned_corners(
    box_on_canvas: egui::Rect,
    canvas: egui::Rect,
    pin: bettercut_editor_core::timeline::CornerPin,
) -> [egui::Pos2; 4] {
    let plain = [
        box_on_canvas.left_top(),
        box_on_canvas.right_top(),
        box_on_canvas.right_bottom(),
        box_on_canvas.left_bottom(),
    ];
    let mut out = plain;
    for (index, corner) in out.iter_mut().enumerate() {
        let offset = pin.clamped().offsets[index];
        *corner = egui::pos2(
            corner.x + offset[0] * canvas.width(),
            corner.y + offset[1] * canvas.height(),
        );
    }
    out
}

/// Which pinned corner's handle is under `pointer`, if any.
pub fn pinned_corner_at(
    box_on_canvas: egui::Rect,
    canvas: egui::Rect,
    pin: bettercut_editor_core::timeline::CornerPin,
    pointer: egui::Pos2,
) -> Option<Corner> {
    pinned_corners(box_on_canvas, canvas, pin)
        .into_iter()
        .zip(Corner::ALL)
        .find(|(at, _)| at.distance(pointer) <= HANDLE_RADIUS)
        .map(|(_, corner)| corner)
}

/// The pinned quad and a handle at each of its corners.
pub fn draw_pin(painter: &egui::Painter, corners: [egui::Pos2; 4]) {
    for pair in 0..4 {
        painter.line_segment(
            [corners[pair], corners[(pair + 1) % 4]],
            egui::Stroke::new(1.5, crate::theme::selection()),
        );
    }
    for at in corners {
        painter.circle_filled(at, HANDLE_DRAW + 1.0, egui::Color32::from_black_alpha(160));
        painter.circle_filled(at, HANDLE_DRAW, crate::theme::selection());
    }
}

/// The four corners of the whole, uncropped source, on the canvas.
///
/// What crop mode outlines, so the user can see what is being cut away and how
/// far there is to go. Turned about the *picture's* centre, not the outline's
/// own: the renderer turns the cropped quad, so an uneven crop leaves the
/// outline off-centre from the thing that is rotating.
pub fn uncropped_corners(picture: egui::Rect, crop: Crop, degrees: f32) -> [egui::Pos2; 4] {
    [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        .map(|corner| clip_point_on_canvas(source_to_picture_uv(corner, crop), picture, degrees))
}

/// The crop after dragging `edge` to `pointer`.
///
/// Absolute from the drag's start (see [`Gesture::Crop`]). The pointer is found
/// in the picture as it was then, taken into the source's frame through the
/// crop as it was then, and becomes the new position of the dragged edge — and
/// only that edge.
///
/// Dragging an edge past the one opposite **stops** it, with
/// [`MIN_CROP_REMAINING`] of the picture left between them. It does not push
/// the other edge along, and it does not split the difference the way
/// `Crop::clamped` does for a value read from a file: the hand is on one edge,
/// and the other edge moving would be the crop doing something it was not
/// asked to.
pub fn crop_dragged(
    edge: CropEdge,
    from: Crop,
    picture: egui::Rect,
    degrees: f32,
    pointer: egui::Pos2,
) -> Crop {
    let picture_uv = canvas_point_in_clip(pointer, picture, degrees);
    let [u, v] = picture_to_source_uv(picture_uv, from);

    // The furthest this edge may go: up to the opposite edge, less the least
    // the crop must leave.
    let room = |opposite: f32| (1.0 - opposite - MIN_CROP_REMAINING).max(0.0);
    let mut crop = from;
    match edge {
        CropEdge::Left => crop.left = u.clamp(0.0, room(from.right)),
        CropEdge::Right => crop.right = (1.0 - u).clamp(0.0, room(from.left)),
        CropEdge::Top => crop.top = v.clamp(0.0, room(from.bottom)),
        CropEdge::Bottom => crop.bottom = (1.0 - v).clamp(0.0, room(from.top)),
    }
    crop
}

/// Where a *clip's* picture sits, crop included: the box the handles belong on.
///
/// [`layer_box`] with §22's crop applied first, which is the order the renderer
/// runs them in. Separate from it because `layer_box` takes an aspect and knows
/// nothing about where that aspect came from — which is exactly how the preview
/// came to pass the uncropped one, and draw every cropped clip's handles around
/// a shape the picture no longer was. Taking the crop here makes that mistake a
/// missing argument rather than a wrong number.
pub fn clip_box(
    source_aspect: f32,
    output_aspect: f32,
    crop: bettercut_editor_core::timeline::Crop,
    transform: Transform,
) -> egui::Rect {
    layer_box(
        crop.clamped().applied_to(source_aspect),
        output_aspect,
        transform,
    )
}

/// Turn `point` about `centre` by `degrees`, clockwise on screen.
///
/// Clockwise because that is what a positive rotation does in the renderer
/// (`rotation_turns_clockwise_on_screen`), and the handles have to agree with
/// the picture. In screen pixels, which are square — rotating in frame units
/// on a 16:9 canvas would shear, which is the bug the renderer itself had.
pub fn rotate_about(point: egui::Pos2, centre: egui::Pos2, degrees: f32) -> egui::Pos2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let dx = point.x - centre.x;
    let dy = point.y - centre.y;
    egui::pos2(
        centre.x + dx * cos - dy * sin,
        centre.y + dx * sin + dy * cos,
    )
}

/// Where a picture that turns about `pivot` sits: the same box, moved so that
/// turning it about its own middle lands its corners where turning about the
/// pivot would. Rotating a box about a point off its middle is rotating it
/// about its middle *and* swinging the middle round the point; this does the
/// swing, so every helper that rotates about the middle stays right.
pub fn pivot_shifted(box_on_canvas: egui::Rect, pivot: egui::Pos2, degrees: f32) -> egui::Rect {
    let centre = box_on_canvas.center();
    let swung = rotate_about(centre, pivot, degrees);
    box_on_canvas.translate(swung - centre)
}

/// How close a pointer has to be to the pivot handle, in points.
pub const PIVOT_GRAB_PIXELS: f32 = 7.0;

pub fn on_pivot(pivot: egui::Pos2, pointer: egui::Pos2) -> bool {
    pivot.distance(pointer) <= PIVOT_GRAB_PIXELS
}

/// The pivot handle: a small target cross, so it is not mistaken for a corner
/// or the rotate handle.
pub fn draw_pivot(painter: &egui::Painter, at: egui::Pos2, active: bool) {
    let colour = if active {
        crate::theme::selection()
    } else {
        crate::theme::clip_text()
    };
    painter.circle_filled(
        at,
        PIVOT_GRAB_PIXELS * 0.55 + 1.0,
        egui::Color32::from_black_alpha(160),
    );
    painter.circle_stroke(at, PIVOT_GRAB_PIXELS * 0.55, egui::Stroke::new(1.5, colour));
    let arm = PIVOT_GRAB_PIXELS;
    painter.line_segment(
        [egui::pos2(at.x - arm, at.y), egui::pos2(at.x + arm, at.y)],
        egui::Stroke::new(1.0, colour),
    );
    painter.line_segment(
        [egui::pos2(at.x, at.y - arm), egui::pos2(at.x, at.y + arm)],
        egui::Stroke::new(1.0, colour),
    );
}

/// The four corners of a box turned by `degrees`, in [`Corner::ALL`] order.
pub fn corners(box_: egui::Rect, degrees: f32) -> [egui::Pos2; 4] {
    let centre = box_.center();
    [
        box_.left_top(),
        box_.right_top(),
        box_.right_bottom(),
        box_.left_bottom(),
    ]
    .map(|corner| rotate_about(corner, centre, degrees))
}

/// Which corner the pointer is on, if any.
pub fn corner_at(box_on_canvas: egui::Rect, degrees: f32, pointer: egui::Pos2) -> Option<Corner> {
    corners(box_on_canvas, degrees)
        .iter()
        .zip(Corner::ALL)
        .find(|(at, _)| at.distance(pointer) <= HANDLE_RADIUS)
        .map(|(_, corner)| corner)
}

/// Whether the pointer is over the picture of a box turned by `degrees`.
///
/// The pointer is turned back into the box's own frame, where the question is
/// an ordinary rectangle test. Testing against the unrotated rectangle instead
/// would let a click on the empty corner of a turned clip grab it, and a click
/// on its actual tip miss.
pub fn contains(box_on_canvas: egui::Rect, degrees: f32, pointer: egui::Pos2) -> bool {
    box_on_canvas.contains(rotate_about(pointer, box_on_canvas.center(), -degrees))
}

/// Where the rotate handle is: above the middle of the top edge, turned with
/// the box so it stays "above" the picture however far it has rotated.
pub fn rotate_handle(box_on_canvas: egui::Rect, degrees: f32) -> egui::Pos2 {
    let above = egui::pos2(
        box_on_canvas.center().x,
        box_on_canvas.top() - ROTATE_OFFSET,
    );
    rotate_about(above, box_on_canvas.center(), degrees)
}

pub fn on_rotate_handle(box_on_canvas: egui::Rect, degrees: f32, pointer: egui::Pos2) -> bool {
    rotate_handle(box_on_canvas, degrees).distance(pointer) <= HANDLE_RADIUS
}

/// The pointer's angle around `centre`, in degrees, clockwise from the right.
pub fn angle_of(centre: egui::Pos2, pointer: egui::Pos2) -> f32 {
    (pointer.y - centre.y)
        .atan2(pointer.x - centre.x)
        .to_degrees()
}

/// The rotation a rotate drag has reached.
///
/// Relative: how far the pointer has turned since the grab, added to where the
/// clip started. Wrapped into -180..180, which is the Inspector's range, and
/// snapped to the nearest right angle when within a few degrees of one.
pub fn rotated(from: f32, grab_angle: f32, centre: egui::Pos2, now: egui::Pos2) -> f32 {
    let turned = from + (angle_of(centre, now) - grab_angle);
    let wrapped = (turned + 180.0).rem_euclid(360.0) - 180.0;
    let nearest = (wrapped / 90.0).round() * 90.0;
    if (wrapped - nearest).abs() <= ROTATE_SNAP {
        // -180 and 180 are the same orientation; keep the one in range.
        if nearest <= -180.0 { 180.0 } else { nearest }
    } else {
        wrapped
    }
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

/// How close, in screen pixels, a moved picture must come to a centre line or
/// a frame edge to snap to it.
pub const SNAP_PIXELS: f32 = 8.0;

/// Where a snapped move landed, and the guide lines it snapped to, in the
/// frame's own units (-0.5 to 0.5 from the middle): a vertical line at `x`,
/// a horizontal one at `y`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    pub position: Vec2,
    pub vertical: Option<f32>,
    pub horizontal: Option<f32>,
}

/// Snap a moved position so the picture's middle lands on the frame's middle,
/// or one of its edges on the frame's edge, when within [`SNAP_PIXELS`].
///
/// `half` is half the picture's size in frame units; each axis snaps on its
/// own, to whichever line is nearest.
pub fn snap_position(at: Vec2, half: [f32; 2], canvas: egui::Rect) -> Snapped {
    let reach = [
        SNAP_PIXELS / canvas.width().max(1.0),
        SNAP_PIXELS / canvas.height().max(1.0),
    ];
    let axis = |value: f32, half: f32, reach: f32| -> (f32, Option<f32>) {
        // (where the position would go, the line it touches)
        let candidates = [(0.0, 0.0), (half - 0.5, -0.5), (0.5 - half, 0.5)];
        candidates
            .into_iter()
            .map(|(target, line)| ((value - target).abs(), target, line))
            .filter(|(distance, _, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map_or((value, None), |(_, target, line)| (target, Some(line)))
    };
    let (x, vertical) = axis(at.x, half[0], reach[0]);
    let (y, horizontal) = axis(at.y, half[1], reach[1]);
    Snapped {
        position: Vec2::new(x, y),
        vertical,
        horizontal,
    }
}

/// Draw the guide lines a snapped move touched.
pub fn draw_snap_guides(painter: &egui::Painter, snapped: Snapped, canvas: egui::Rect) {
    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 80, 160));
    if let Some(x) = snapped.vertical {
        let px = canvas.center().x + x * canvas.width();
        painter.line_segment(
            [
                egui::pos2(px, canvas.top()),
                egui::pos2(px, canvas.bottom()),
            ],
            stroke,
        );
    }
    if let Some(y) = snapped.horizontal {
        let py = canvas.center().y + y * canvas.height();
        painter.line_segment(
            [
                egui::pos2(canvas.left(), py),
                egui::pos2(canvas.right(), py),
            ],
            stroke,
        );
    }
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

/// Draw the box, its four corner circles and the rotate handle.
pub fn draw(painter: &egui::Painter, box_on_canvas: egui::Rect, degrees: f32, active: bool) {
    let colour = if active {
        crate::theme::selection()
    } else {
        crate::theme::clip_text()
    };
    let stroke = egui::Stroke::new(1.5, colour);

    // Four segments rather than `rect_stroke`, which only draws upright.
    let points = corners(box_on_canvas, degrees);
    for index in 0..4 {
        painter.line_segment([points[index], points[(index + 1) % 4]], stroke);
    }

    // The rotate handle on a short stalk from the top edge, so it reads as
    // attached to the picture rather than floating near it.
    let top_middle = rotate_about(
        egui::pos2(box_on_canvas.center().x, box_on_canvas.top()),
        box_on_canvas.center(),
        degrees,
    );
    let handle = rotate_handle(box_on_canvas, degrees);
    painter.line_segment([top_middle, handle], stroke);
    painter.circle_filled(
        handle,
        HANDLE_DRAW + 1.0,
        egui::Color32::from_black_alpha(160),
    );
    painter.circle_stroke(handle, HANDLE_DRAW, egui::Stroke::new(2.0, colour));

    for at in points {
        // Filled, with a dark ring: on bright footage a plain light circle
        // disappears, and on dark footage a plain dark one does.
        painter.circle_filled(at, HANDLE_DRAW + 1.0, egui::Color32::from_black_alpha(160));
        painter.circle_filled(at, HANDLE_DRAW, colour);
    }
}

/// Crop mode's overlay: what is kept, what was cut, and an edge handle on each
/// side.
///
/// The outline of the *whole* source is drawn dashed and faint, so it reads as
/// "what used to be here" rather than as a second box to grab. The picture as
/// cropped keeps the solid outline the move handles use, and its sides get
/// bars rather than circles: a bar says "this slides one way", which is what a
/// crop edge does, where a circle is a corner that goes anywhere.
pub fn draw_crop(
    painter: &egui::Painter,
    picture: egui::Rect,
    crop: Crop,
    degrees: f32,
    active: bool,
) {
    let colour = if active {
        crate::theme::selection()
    } else {
        crate::theme::clip_text()
    };

    let whole = uncropped_corners(picture, crop, degrees);
    let faint = egui::Stroke::new(1.0, colour.gamma_multiply(0.45));
    for index in 0..4 {
        painter.add(egui::Shape::dashed_line(
            &[whole[index], whole[(index + 1) % 4]],
            faint,
            6.0,
            4.0,
        ));
    }

    let kept = corners(picture, degrees);
    let stroke = egui::Stroke::new(1.5, colour);
    for index in 0..4 {
        painter.line_segment([kept[index], kept[(index + 1) % 4]], stroke);
    }

    for (edge, at) in crop_edge_handles(picture, degrees) {
        // Along the edge it sits on, turned with the clip.
        let along = match edge {
            CropEdge::Left | CropEdge::Right => egui::vec2(0.0, 1.0),
            CropEdge::Top | CropEdge::Bottom => egui::vec2(1.0, 0.0),
        };
        let (sin, cos) = degrees.to_radians().sin_cos();
        let turned = egui::vec2(along.x * cos - along.y * sin, along.x * sin + along.y * cos);
        let half = turned * (HANDLE_DRAW * 2.2);
        // Dark underneath, for the reason the corner handles have one.
        painter.line_segment(
            [at - half, at + half],
            egui::Stroke::new(7.0, egui::Color32::from_black_alpha(160)),
        );
        painter.line_segment([at - half, at + half], egui::Stroke::new(4.0, colour));
    }
}

/// The command a gesture has produced this frame.
/// The property a drag is writing, or `None` when this signature cannot say.
///
/// A mask drag is the `None`: moving a mask means writing a whole `Mask` back,
/// and that needs the mask being moved — which the caller has and this does
/// not. Returning some other property would be a wrong answer rather than an
/// absent one, and the wrong answer available here is `Mask(None)`, which
/// deletes the mask the user is dragging.
/// Smallest a mask may be dragged to, in the clip's own frame.
///
/// Not zero: a mask dragged to nothing has no handle left to drag it back by,
/// and the user would have to reach for the Inspector to undo their own
/// gesture.
pub const SMALLEST_MASK: f32 = 0.01;

/// Where a mask's size handle sits, or `None` for a shape that has no size.
///
/// A linear mask is an edge, not an area — the Inspector hides its size for the
/// same reason, and a handle offering to resize nothing would be worse than no
/// handle.
pub fn mask_size_handle(
    mask: bettercut_editor_core::timeline::Mask,
    box_on_canvas: egui::Rect,
    clip_degrees: f32,
) -> Option<egui::Pos2> {
    if mask.shape == bettercut_editor_core::timeline::MaskShape::Linear {
        return None;
    }
    // Out along the mask's own axes, which are turned by the mask's rotation
    // *within* the clip's frame — the same order the shader applies them in.
    let (sin, cos) = mask.rotation_degrees.to_radians().sin_cos();
    let corner = [
        mask.center[0] + mask.size[0] * cos - mask.size[1] * sin,
        mask.center[1] + mask.size[0] * sin + mask.size[1] * cos,
    ];
    Some(clip_point_on_canvas(corner, box_on_canvas, clip_degrees))
}

/// The size a mask ends up after a resize drag.
///
/// Measured from where the drag began rather than from the pointer's absolute
/// position, so grabbing the handle a few pixels off centre does not snap the
/// shape to the pointer.
pub fn mask_resized(
    from: [f32; 2],
    grab: egui::Pos2,
    now: egui::Pos2,
    mask_degrees: f32,
    box_on_canvas: egui::Rect,
    clip_degrees: f32,
) -> [f32; 2] {
    let local = |at: egui::Pos2| {
        let point = canvas_point_in_clip(at, box_on_canvas, clip_degrees);
        // Back out of the mask's own rotation, so dragging a turned mask's
        // handle grows it along its own axes rather than the frame's.
        let (sin, cos) = (-mask_degrees).to_radians().sin_cos();
        [
            point[0] * cos - point[1] * sin,
            point[0] * sin + point[1] * cos,
        ]
    };
    let was = local(grab);
    let is = local(now);
    [
        (from[0] + (is[0] - was[0])).max(SMALLEST_MASK),
        (from[1] + (is[1] - was[1])).max(SMALLEST_MASK),
    ]
}

/// Which pixel of the rendered frame a point on the canvas lands on.
///
/// `None` outside the picture: the canvas is letterboxed into the panel, and a
/// click on the black around it is pointing at nothing. For the eyedropper,
/// where answering anyway would put a colour into the project that is not in
/// the shot.
///
/// The frame fills the canvas exactly — it is drawn with UVs 0..1 across it —
/// so this is a scale, not a fit. The right and bottom edges belong to the last
/// pixel rather than to one past it, which is where an off-by-one would send
/// the read out of the texture.
pub fn frame_pixel_at(
    at: egui::Pos2,
    canvas: egui::Rect,
    resolution: bettercut_editor_core::timeline::Resolution,
) -> Option<(u32, u32)> {
    if !canvas.contains(at) || canvas.width() <= 0.0 || canvas.height() <= 0.0 {
        return None;
    }
    if resolution.width == 0 || resolution.height == 0 {
        return None;
    }

    let u = (at.x - canvas.left()) / canvas.width();
    let v = (at.y - canvas.top()) / canvas.height();
    let x = (u * resolution.width as f32) as u32;
    let y = (v * resolution.height as f32) as u32;
    Some((x.min(resolution.width - 1), y.min(resolution.height - 1)))
}

/// How close to the mask's centre counts as grabbing it, in screen pixels.
///
/// The same generosity the timeline's volume points get: a drawn dot is smaller
/// than a target anyone can hit.
pub const MASK_GRAB_PIXELS: f32 = 9.0;

/// Draw a mask's centre on the preview, so there is something to take hold of.
///
/// The shape itself is already visible — the picture outside it is gone — so
/// this is a handle rather than an outline: drawing an ellipse over an ellipse
/// that is already there would only make the edge harder to judge.
pub fn draw_mask_handle(painter: &egui::Painter, at: egui::Pos2, active: bool) {
    let colour = if active {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_white_alpha(200)
    };
    painter.circle_stroke(at, MASK_GRAB_PIXELS * 0.6, egui::Stroke::new(1.5, colour));
    painter.circle_filled(at, 2.0, colour);
}

/// Whether the pointer is on a mask's centre handle.
pub fn on_mask_handle(centre_on_canvas: egui::Pos2, pointer: egui::Pos2) -> bool {
    centre_on_canvas.distance(pointer) <= MASK_GRAB_PIXELS
}

pub fn property_for(
    gesture: Gesture,
    canvas: egui::Rect,
    box_on_canvas: egui::Rect,
    now: egui::Pos2,
) -> Option<ClipProperty> {
    match gesture {
        Gesture::Move { from, grab } => {
            let at = moved_position(from, grab, now, canvas);
            Some(ClipProperty::Position { x: at.x, y: at.y })
        }
        Gesture::Rotate { from, grab_angle } => Some(ClipProperty::Rotation(rotated(
            from,
            grab_angle,
            box_on_canvas.center(),
            now,
        ))),
        Gesture::Scale {
            from,
            grab_distance,
            ..
        } => {
            let at = scaled(from, grab_distance, box_on_canvas.center(), now);
            Some(ClipProperty::Scale { x: at.x, y: at.y })
        }
        Gesture::Pin {
            corner,
            from,
            origin,
        } => {
            // The offset is in frames, so the distance on the canvas is
            // divided by the canvas — which is what keeps a pin the same shape
            // when the preview is resized.
            let offset = [
                (now.x - origin.x) / canvas.width().max(1.0),
                (now.y - origin.y) / canvas.height().max(1.0),
            ];
            let index = match corner {
                Corner::TopLeft => 0,
                Corner::TopRight => 1,
                Corner::BottomRight => 2,
                Corner::BottomLeft => 3,
            };
            Some(ClipProperty::CornerPin(from.with_corner(index, offset)))
        }
        Gesture::Crop {
            edge,
            from,
            picture,
            degrees,
        } => Some(ClipProperty::Crop(crop_dragged(
            edge, from, picture, degrees, now,
        ))),
        Gesture::MaskMove { .. }
        | Gesture::MaskResize { .. }
        | Gesture::PathKey { .. }
        | Gesture::Pivot => None,
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
            corner_at(box_, 0.0, egui::pos2(100.0, 100.0)),
            Some(Corner::TopLeft)
        );
        assert_eq!(
            corner_at(box_, 0.0, egui::pos2(300.0, 250.0)),
            Some(Corner::BottomRight)
        );
        // Just inside the radius.
        assert_eq!(
            corner_at(box_, 0.0, egui::pos2(104.0, 103.0)),
            Some(Corner::TopLeft)
        );
        // Well inside the box, but not on a corner: that is a move, not a scale.
        assert_eq!(corner_at(box_, 0.0, egui::pos2(200.0, 175.0)), None);
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

    /// Positive rotation turns clockwise on screen, as it does in the renderer.
    /// A handle that turned the other way would sit on the opposite side of
    /// the picture from the corner it belongs to.
    #[test]
    fn rotation_is_clockwise_on_screen() {
        let centre = egui::pos2(100.0, 100.0);
        // The point to the right of centre swings down (screen y grows).
        let turned = rotate_about(egui::pos2(150.0, 100.0), centre, 90.0);
        assert!((turned.x - 100.0).abs() < 1e-3 && (turned.y - 150.0).abs() < 1e-3);
    }

    /// The corners turn with the box, and a turned corner is what gets grabbed.
    #[test]
    fn a_turned_boxs_corner_is_where_it_is_drawn() {
        let box_ = egui::Rect::from_min_max(egui::pos2(100.0, 150.0), egui::pos2(300.0, 250.0));
        let turned = corners(box_, 90.0);

        // Top-left of a 200×100 box turned 90° about (200, 200) is at (250, 100).
        assert!(
            turned[0].distance(egui::pos2(250.0, 100.0)) < 1e-3,
            "{:?}",
            turned[0]
        );
        assert_eq!(corner_at(box_, 90.0, turned[0]), Some(Corner::TopLeft));
        // And the unrotated spot is empty now.
        assert_eq!(corner_at(box_, 90.0, egui::pos2(100.0, 150.0)), None);
    }

    /// Hit-testing follows the turned picture: its tip is inside, the empty
    /// corner of its upright box is not.
    #[test]
    fn a_turned_box_is_grabbed_by_its_picture_not_its_upright_box() {
        let box_ = egui::Rect::from_center_size(egui::pos2(200.0, 200.0), egui::vec2(200.0, 20.0));
        // Turned 90°, a wide thin bar becomes a tall thin one.
        assert!(
            contains(box_, 90.0, egui::pos2(200.0, 290.0)),
            "the tip was missed"
        );
        assert!(
            !contains(box_, 90.0, egui::pos2(290.0, 200.0)),
            "the empty end of the upright box still grabbed it"
        );
    }

    /// The rotate handle stays "above" the picture as it turns.
    #[test]
    fn the_rotate_handle_turns_with_the_box() {
        let box_ = egui::Rect::from_center_size(egui::pos2(200.0, 200.0), egui::vec2(100.0, 100.0));
        let upright = rotate_handle(box_, 0.0);
        assert!(upright.y < box_.top(), "the handle is not above the box");

        // Turned 90° clockwise, "above" is to the right.
        let turned = rotate_handle(box_, 90.0);
        assert!(
            turned.x > box_.right(),
            "the handle did not turn with the box"
        );
        assert!(on_rotate_handle(box_, 90.0, turned));
    }

    /// The drag is relative: grabbing the handle does not snap the clip to the
    /// pointer's angle, and a quarter turn of the pointer is a quarter turn of
    /// the clip.
    #[test]
    fn a_rotate_drag_adds_how_far_the_pointer_turned() {
        let centre = egui::pos2(0.0, 0.0);
        let grab = angle_of(centre, egui::pos2(0.0, -10.0)); // straight up
        let now = egui::pos2(10.0, 0.0); // a quarter turn clockwise
        assert!((rotated(10.0, grab, centre, now) - 100.0).abs() < 1e-3);
    }

    /// Wrapped into the Inspector's -180..180, so a full turn does not run the
    /// number off to 400°.
    #[test]
    fn a_rotate_drag_wraps_into_range() {
        let centre = egui::pos2(0.0, 0.0);
        let grab = angle_of(centre, egui::pos2(10.0, 0.0));
        let now = egui::pos2(0.0, -10.0); // three quarters of a turn the long way
        let result = rotated(170.0, grab, centre, now);
        assert!((-180.0..=180.0).contains(&result), "{result}");
        assert!((result - 80.0).abs() < 1e-3, "{result}");
    }

    /// Within a few degrees of a right angle the drag lands on it exactly —
    /// level and upright are what a turn is usually aiming for.
    #[test]
    fn a_rotate_drag_snaps_to_right_angles() {
        let centre = egui::pos2(0.0, 0.0);
        let grab = 0.0;
        // Two degrees off level.
        let near_level = rotate_about(egui::pos2(10.0, 0.0), centre, 2.0);
        assert_eq!(rotated(0.0, grab, centre, near_level), 0.0);
        // Eighty-eight: snaps to ninety.
        let near_upright = rotate_about(egui::pos2(10.0, 0.0), centre, 88.0);
        assert_eq!(rotated(0.0, grab, centre, near_upright), 90.0);
        // Twenty: left alone.
        let free = rotate_about(egui::pos2(10.0, 0.0), centre, 20.0);
        assert!((rotated(0.0, grab, centre, free) - 20.0).abs() < 1e-3);
    }
}

#[cfg(test)]
mod mask_tests {
    use super::*;

    fn box_at(left: f32, top: f32, width: f32, height: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, height))
    }

    /// The middle of the clip's frame is the middle of its box, whatever the
    /// box is doing.
    #[test]
    fn the_centre_of_the_frame_is_the_centre_of_the_box() {
        let box_ = box_at(100.0, 50.0, 400.0, 200.0);
        for degrees in [0.0, 30.0, -90.0, 180.0] {
            let at = clip_point_on_canvas([0.5, 0.5], box_, degrees);
            assert!(
                at.distance(box_.center()) < 1e-3,
                "at {degrees}° the centre landed at {at:?}"
            );
        }
    }

    /// Corners go to corners on an unturned box: 0,0 is the top left.
    #[test]
    fn the_corners_of_the_frame_are_the_corners_of_the_box() {
        let box_ = box_at(100.0, 50.0, 400.0, 200.0);
        assert!(clip_point_on_canvas([0.0, 0.0], box_, 0.0).distance(box_.left_top()) < 1e-3);
        assert!(clip_point_on_canvas([1.0, 1.0], box_, 0.0).distance(box_.right_bottom()) < 1e-3);
    }

    /// A mask picked up and put down without moving must land exactly where it
    /// was. One that drifted a little every time would be worse than one that
    /// could not be dragged at all.
    #[test]
    fn a_point_survives_the_round_trip() {
        let box_ = box_at(80.0, 40.0, 360.0, 180.0);
        for degrees in [0.0, 17.0, -42.0, 155.0] {
            for uv in [[0.5, 0.5], [0.1, 0.9], [0.73, 0.22], [0.0, 0.0]] {
                let there = clip_point_on_canvas(uv, box_, degrees);
                let back = canvas_point_in_clip(there, box_, degrees);
                assert!(
                    (back[0] - uv[0]).abs() < 1e-3 && (back[1] - uv[1]).abs() < 1e-3,
                    "{uv:?} at {degrees}° came back as {back:?}"
                );
            }
        }
    }

    /// A drag that does not move the pointer does not move the mask — the
    /// property this needs for a click on the handle to be a click and not a
    /// nudge.
    #[test]
    fn a_drag_that_goes_nowhere_changes_nothing() {
        let box_ = box_at(0.0, 0.0, 320.0, 180.0);
        let grab = egui::pos2(160.0, 90.0);
        let moved = mask_moved([0.4, 0.6], grab, grab, box_, 25.0);
        assert!((moved[0] - 0.4).abs() < 1e-4 && (moved[1] - 0.6).abs() < 1e-4);
    }

    /// Dragging half the box's width across moves the mask half a frame.
    #[test]
    fn a_drag_moves_the_mask_by_what_the_pointer_crossed() {
        let box_ = box_at(0.0, 0.0, 400.0, 200.0);
        let moved = mask_moved(
            [0.25, 0.5],
            egui::pos2(100.0, 100.0),
            egui::pos2(300.0, 100.0),
            box_,
            0.0,
        );
        assert!((moved[0] - 0.75).abs() < 1e-3, "moved to {moved:?}");
        assert!((moved[1] - 0.5).abs() < 1e-3, "it moved vertically too");
    }

    /// On a turned clip the mask follows the *picture*, not the screen: dragging
    /// along the screen's x axis on a clip turned 90° moves the mask down its
    /// own frame.
    #[test]
    fn a_drag_follows_the_turned_picture() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let moved = mask_moved(
            [0.5, 0.5],
            egui::pos2(100.0, 100.0),
            egui::pos2(150.0, 100.0),
            box_,
            90.0,
        );
        assert!(
            (moved[0] - 0.5).abs() < 1e-3,
            "it moved across its own frame: {moved:?}"
        );
        assert!(
            (moved[1] - 0.5).abs() > 0.1,
            "it did not move down its own frame: {moved:?}"
        );
    }

    /// The handle is a target with edges, like every other one.
    #[test]
    fn the_handle_has_a_reachable_size() {
        let centre = egui::pos2(100.0, 100.0);
        assert!(on_mask_handle(centre, centre));
        assert!(on_mask_handle(centre, egui::pos2(105.0, 100.0)));
        assert!(!on_mask_handle(centre, egui::pos2(140.0, 100.0)));
    }

    /// A mask drag is the one gesture `property_for` refuses: the answer needs
    /// the mask being moved, and the only property it could return here is the
    /// one that deletes it.
    #[test]
    fn property_for_refuses_a_mask_drag() {
        let box_ = box_at(0.0, 0.0, 100.0, 100.0);
        let gesture = Gesture::MaskMove {
            from: [0.5, 0.5],
            grab: egui::pos2(10.0, 10.0),
        };
        assert!(property_for(gesture, box_, box_, egui::pos2(20.0, 20.0)).is_none());
    }

    fn mask(size: [f32; 2], degrees: f32) -> bettercut_editor_core::timeline::Mask {
        bettercut_editor_core::timeline::Mask {
            shape: bettercut_editor_core::timeline::MaskShape::Ellipse,
            center: [0.5, 0.5],
            size,
            feather: 0.0,
            rotation_degrees: degrees,
            invert: false,
        }
    }

    /// A linear mask is an edge, not an area: there is nothing to resize, and a
    /// handle offering to would be worse than none.
    #[test]
    fn a_linear_mask_has_no_size_handle() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let mut linear = mask([0.2, 0.2], 0.0);
        linear.shape = bettercut_editor_core::timeline::MaskShape::Linear;
        assert!(mask_size_handle(linear, box_, 0.0).is_none());
        assert!(mask_size_handle(mask([0.2, 0.2], 0.0), box_, 0.0).is_some());
    }

    /// The handle sits on the shape's edge, out along its own axes.
    #[test]
    fn the_size_handle_sits_at_the_shapes_edge() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let at = mask_size_handle(mask([0.25, 0.25], 0.0), box_, 0.0).expect("a handle");
        // Centre (0.5,0.5) plus a quarter each way, on a 200px box.
        assert!((at.x - 150.0).abs() < 1e-3, "x at {}", at.x);
        assert!((at.y - 150.0).abs() < 1e-3, "y at {}", at.y);
    }

    /// A turned mask's handle turns with it, or dragging it would grow the
    /// shape along an axis it does not have.
    #[test]
    fn the_size_handle_turns_with_the_mask() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let straight = mask_size_handle(mask([0.25, 0.0], 0.0), box_, 0.0).expect("a handle");
        let turned = mask_size_handle(mask([0.25, 0.0], 90.0), box_, 0.0).expect("a handle");
        assert!((straight.x - 150.0).abs() < 1e-3 && (straight.y - 100.0).abs() < 1e-3);
        assert!(
            (turned.x - 100.0).abs() < 1e-3 && (turned.y - 150.0).abs() < 1e-3,
            "a quarter turn should put it below the centre, not beside it: {turned:?}"
        );
    }

    /// Grabbing the handle without moving leaves the size alone: a click is not
    /// a resize.
    #[test]
    fn a_resize_that_goes_nowhere_changes_nothing() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let grab = egui::pos2(150.0, 150.0);
        let size = mask_resized([0.25, 0.25], grab, grab, 0.0, box_, 0.0);
        assert!((size[0] - 0.25).abs() < 1e-4 && (size[1] - 0.25).abs() < 1e-4);
    }

    #[test]
    fn dragging_the_handle_out_grows_the_mask() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let size = mask_resized(
            [0.25, 0.25],
            egui::pos2(150.0, 150.0),
            egui::pos2(190.0, 150.0),
            0.0,
            box_,
            0.0,
        );
        assert!((size[0] - 0.45).abs() < 1e-3, "width came out {}", size[0]);
        assert!((size[1] - 0.25).abs() < 1e-3, "the height changed too");
    }

    /// A turned mask grows along its *own* axes. On a mask at a quarter turn,
    /// dragging down the screen widens it rather than making it taller —
    /// because what is "down the screen" is its width.
    #[test]
    fn dragging_a_turned_mask_grows_its_own_axis() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let size = mask_resized(
            [0.25, 0.1],
            egui::pos2(100.0, 150.0),
            egui::pos2(100.0, 190.0),
            90.0,
            box_,
            0.0,
        );
        assert!(
            (size[0] - 0.45).abs() < 1e-3,
            "its own width did not grow: {size:?}"
        );
        assert!(
            (size[1] - 0.1).abs() < 1e-3,
            "its own height changed: {size:?}"
        );
    }

    /// A mask dragged to nothing has no handle left to drag it back by, so the
    /// gesture stops short of that.
    #[test]
    fn a_mask_cannot_be_dragged_away_to_nothing() {
        let box_ = box_at(0.0, 0.0, 200.0, 200.0);
        let size = mask_resized(
            [0.25, 0.25],
            egui::pos2(150.0, 150.0),
            egui::pos2(-400.0, -400.0),
            0.0,
            box_,
            0.0,
        );
        assert_eq!(size, [SMALLEST_MASK, SMALLEST_MASK]);
    }
}

#[cfg(test)]
mod eyedropper_tests {
    use super::*;

    fn hd() -> bettercut_editor_core::timeline::Resolution {
        bettercut_editor_core::timeline::Resolution::new(1920, 1080)
    }

    /// A canvas that is not the frame's size and does not start at the origin,
    /// so a mapping that forgot to subtract the offset or to scale cannot pass.
    fn canvas() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(640.0, 360.0))
    }

    #[test]
    fn the_corners_of_the_canvas_are_the_corners_of_the_frame() {
        let canvas = canvas();
        assert_eq!(
            frame_pixel_at(canvas.left_top(), canvas, hd()),
            Some((0, 0))
        );
        assert_eq!(
            frame_pixel_at(canvas.center(), canvas, hd()),
            Some((960, 540))
        );
    }

    /// The far edges belong to the last pixel, not to one past it — that is
    /// exactly where a read would fall off the end of the texture.
    #[test]
    fn the_far_edges_land_on_the_last_pixel() {
        let canvas = canvas();
        assert_eq!(
            frame_pixel_at(canvas.right_bottom(), canvas, hd()),
            Some((1919, 1079))
        );
    }

    /// The canvas is letterboxed into the panel, so there is black around it.
    /// Pointing at that is pointing at nothing, and answering anyway would put
    /// a colour into the project that is not in the shot.
    #[test]
    fn a_point_outside_the_canvas_is_not_a_pixel() {
        let canvas = canvas();
        assert_eq!(frame_pixel_at(egui::pos2(50.0, 60.0), canvas, hd()), None);
        assert_eq!(frame_pixel_at(egui::pos2(200.0, 10.0), canvas, hd()), None);
        assert_eq!(
            frame_pixel_at(
                egui::pos2(canvas.right() + 1.0, canvas.center().y),
                canvas,
                hd()
            ),
            None
        );
    }

    /// A vertical sequence, because a mapping that used one dimension for both
    /// would still pass on a 16:9 frame roughly enough to look right.
    #[test]
    fn the_two_axes_scale_independently() {
        let canvas = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(270.0, 480.0));
        let vertical = bettercut_editor_core::timeline::Resolution::new(1080, 1920);

        assert_eq!(
            frame_pixel_at(egui::pos2(135.0, 120.0), canvas, vertical),
            Some((540, 480))
        );
    }
}

#[cfg(test)]
mod crop_handle_tests {
    use super::*;

    /// A picture box somewhere on the canvas, not at the origin and not
    /// square, so no symmetry can hide an axis mistake.
    fn picture() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(120.0, 80.0), egui::vec2(300.0, 170.0))
    }

    fn uneven() -> Crop {
        Crop {
            left: 0.2,
            top: 0.05,
            right: 0.1,
            bottom: 0.3,
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// The two frames a crop handle moves between must round-trip, or a drag
    /// would drift a little every time the edge was picked up.
    #[test]
    fn source_and_picture_coordinates_round_trip() {
        for crop in [Crop::NONE, uneven()] {
            for uv in [[0.0, 0.0], [1.0, 1.0], [0.3, 0.7], [-0.2, 1.4]] {
                let back = source_to_picture_uv(picture_to_source_uv(uv, crop), crop);
                assert!(
                    near(back[0], uv[0]) && near(back[1], uv[1]),
                    "{uv:?} came back as {back:?} through {crop:?}"
                );
            }
        }
    }

    /// **Grabbing an edge must not move it.** Pressing a handle and releasing
    /// without moving is the most common thing that happens to a handle, and a
    /// crop that jumped on the press would be unusable. Checked turned and
    /// unevenly cropped, which is where the frames are most likely to disagree.
    #[test]
    fn grabbing_an_edge_without_moving_changes_nothing() {
        for crop in [Crop::NONE, uneven()] {
            for degrees in [0.0, 23.0, -140.0] {
                for (edge, handle) in crop_edge_handles(picture(), degrees) {
                    let after = crop_dragged(edge, crop, picture(), degrees, handle);
                    for (name, a, b) in [
                        ("left", after.left, crop.left),
                        ("top", after.top, crop.top),
                        ("right", after.right, crop.right),
                        ("bottom", after.bottom, crop.bottom),
                    ] {
                        assert!(
                            near(a, b),
                            "grabbing {edge:?} at {degrees}° moved {name} from {b} to {a}"
                        );
                    }
                }
            }
        }
    }

    /// Dragging the left edge inwards crops more from the left, and touches no
    /// other edge.
    #[test]
    fn dragging_an_edge_moves_that_edge_alone() {
        let (edge, handle) = crop_edge_handles(picture(), 0.0)[0];
        assert_eq!(edge, CropEdge::Left);

        // A tenth of the picture's width to the right.
        let inwards = handle + egui::vec2(picture().width() * 0.1, 0.0);
        let after = crop_dragged(CropEdge::Left, Crop::NONE, picture(), 0.0, inwards);

        assert!(
            near(after.left, 0.1),
            "expected a tenth taken off: {after:?}"
        );
        assert_eq!(after.top, 0.0);
        assert_eq!(after.right, 0.0);
        assert_eq!(after.bottom, 0.0);
    }

    /// Each edge reaches its own field. A handle wired to the wrong side would
    /// crop the picture from the other end, and look as if something happened.
    #[test]
    fn each_edge_handle_crops_its_own_side() {
        let centre = picture().center();
        for (edge, _) in crop_edge_handles(picture(), 0.0) {
            // Drag every handle to the centre: each should crop its own side
            // by about half, and only its own side.
            let after = crop_dragged(edge, Crop::NONE, picture(), 0.0, centre);
            let moved = [
                (CropEdge::Left, after.left),
                (CropEdge::Top, after.top),
                (CropEdge::Right, after.right),
                (CropEdge::Bottom, after.bottom),
            ];
            for (side, value) in moved {
                if side == edge {
                    assert!(value > 0.4, "{edge:?} did not crop its own side: {after:?}");
                } else {
                    assert_eq!(value, 0.0, "{edge:?} cropped {side:?} as well: {after:?}");
                }
            }
        }
    }

    /// Dragged past the opposite edge, the edge stops — and the opposite edge
    /// does not move. The hand is on one edge; the other moving would be the
    /// crop doing something it was not asked to.
    #[test]
    fn an_edge_stops_short_of_the_one_opposite() {
        let from = Crop {
            right: 0.3,
            ..Crop::NONE
        };
        // Far past the right-hand side of the picture.
        let beyond = egui::pos2(picture().right() + 500.0, picture().center().y);
        let after = crop_dragged(CropEdge::Left, from, picture(), 0.0, beyond);

        assert_eq!(after.right, 0.3, "the opposite edge was pushed along");
        let (kept, _) = after.remaining();
        assert!(
            near(kept, MIN_CROP_REMAINING),
            "the edge did not stop at the least the crop must leave: kept {kept}"
        );
    }

    /// Dragged outwards past the source's own edge, the crop stops at nothing
    /// taken off rather than going negative — there is no picture out there.
    #[test]
    fn an_edge_cannot_be_dragged_past_the_source() {
        let from = Crop {
            left: 0.2,
            ..Crop::NONE
        };
        let far_left = egui::pos2(picture().left() - 1000.0, picture().center().y);
        let after = crop_dragged(CropEdge::Left, from, picture(), 0.0, far_left);
        assert_eq!(after.left, 0.0);
    }

    /// The uncropped outline sits exactly on the picture when nothing is
    /// cropped, and extends past it by the cropped fractions when something is.
    #[test]
    fn the_uncropped_outline_extends_by_what_was_cut() {
        let plain = uncropped_corners(picture(), Crop::NONE, 0.0);
        assert!(near(plain[0].x, picture().left()) && near(plain[0].y, picture().top()));
        assert!(near(plain[2].x, picture().right()) && near(plain[2].y, picture().bottom()));

        let crop = Crop {
            left: 0.25,
            ..Crop::NONE
        };
        let outline = uncropped_corners(picture(), crop, 0.0);
        // A quarter of the source is gone from the left, so the picture shown
        // is three quarters of the source's width; the outline's left edge is
        // a third of the picture's width further out.
        let expected_left = picture().left() - picture().width() / 3.0;
        assert!(
            near(outline[0].x, expected_left),
            "outline left at {}, expected {expected_left}",
            outline[0].x
        );
        assert!(near(outline[1].x, picture().right()));
    }
}
