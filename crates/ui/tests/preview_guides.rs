//! Guides over the preview (`preview_overlay::guide_shapes`, `PreviewGuide`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_ui::UiState;
use bettercut_ui::preview_overlay::{
    GuideShape, SOCIAL_BOTTOM, SOCIAL_RIGHT, SOCIAL_TOP, guide_shapes,
};
use bettercut_ui::state::PreviewGuide;
use egui::{Pos2, RawInput, Rect, pos2, vec2};

fn canvas() -> Rect {
    Rect::from_min_size(pos2(100.0, 50.0), vec2(360.0, 640.0))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn no_guide_draws_nothing() {
    assert!(guide_shapes(PreviewGuide::Off, canvas()).is_empty());
}

/// Two lines each way, at a third and two thirds, across the whole picture.
#[test]
fn thirds_cut_the_picture_into_nine() {
    let c = canvas();
    let shapes = guide_shapes(PreviewGuide::Thirds, c);
    assert_eq!(shapes.len(), 4);
    let verticals: Vec<f32> = shapes
        .iter()
        .filter_map(|s| match s {
            GuideShape::Line(a, b) if close(a.x, b.x) => {
                assert!(close(a.y, c.top()) && close(b.y, c.bottom()));
                Some(a.x)
            }
            _ => None,
        })
        .collect();
    assert_eq!(verticals.len(), 2);
    assert!(close(verticals[0], c.left() + 120.0));
    assert!(close(verticals[1], c.left() + 240.0));
}

/// Action safe is 90% of the frame, title safe 80%, both centred.
#[test]
fn title_safe_is_two_centred_boxes() {
    let c = canvas();
    let frames: Vec<Rect> = guide_shapes(PreviewGuide::TitleSafe, c)
        .into_iter()
        .filter_map(|s| match s {
            GuideShape::Frame(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(frames.len(), 2);
    assert!(close(frames[0].width(), c.width() * 0.9));
    assert!(close(frames[1].height(), c.height() * 0.8));
    for frame in frames {
        assert!(close(frame.center().x, c.center().x) && close(frame.center().y, c.center().y));
    }
}

/// The phone app's parts are shaded along the top, bottom and right, and the
/// always-seen box is what is left between them.
#[test]
fn social_shades_the_app_and_frames_the_rest() {
    let c = canvas();
    let shapes = guide_shapes(PreviewGuide::Social, c);
    let shades: Vec<Rect> = shapes
        .iter()
        .filter_map(|s| match s {
            GuideShape::Shade(r) => Some(*r),
            _ => None,
        })
        .collect();
    assert_eq!(shades.len(), 3);
    assert!(
        shades.iter().all(|r| c.contains_rect(*r)),
        "a shade spills out"
    );
    let clear = shapes
        .iter()
        .find_map(|s| match s {
            GuideShape::Frame(r) => Some(*r),
            _ => None,
        })
        .unwrap();
    assert!(close(clear.top(), c.top() + c.height() * SOCIAL_TOP));
    assert!(close(
        clear.bottom(),
        c.bottom() - c.height() * SOCIAL_BOTTOM
    ));
    assert!(close(clear.right(), c.right() - c.width() * SOCIAL_RIGHT));
    // Nothing shaded reaches into the clear box's middle.
    assert!(shades.iter().all(|r| !r.contains(clear.center())));
}

/// The preview draws the chosen guide, and more for it than for none.
/// A cross in the middle, and nothing anywhere near the subject's face.
#[test]
fn the_centre_guide_is_a_cross_on_the_middle() {
    let c = canvas();
    let shapes = guide_shapes(PreviewGuide::Centre, c);
    assert_eq!(shapes.len(), 2);

    let middle = c.center();
    for shape in &shapes {
        let GuideShape::Line(a, b) = shape else {
            panic!("the centre guide should be two lines: {shape:?}");
        };
        // Each line is centred on the middle of the picture.
        assert!(close((a.x + b.x) / 2.0, middle.x), "{a:?} {b:?}");
        assert!(close((a.y + b.y) / 2.0, middle.y), "{a:?} {b:?}");
        // And short: a tenth of the picture, not a line across it.
        let length = (b.x - a.x).abs().max((b.y - a.y).abs());
        assert!(
            length < c.width().max(c.height()) / 5.0,
            "the cross is {length} long, which is a line across the picture"
        );
    }
    // One of each direction.
    let horizontals = shapes
        .iter()
        .filter(|s| matches!(s, GuideShape::Line(a, b) if close(a.y, b.y)))
        .count();
    assert_eq!(horizontals, 1, "{shapes:?}");
}

#[test]
fn the_preview_draws_the_guide() {
    let (mut editor, _events) = Editor::new_project("Guides");
    let ctx = egui::Context::default();
    let mut count = |guide: PreviewGuide| {
        let mut state = UiState::default();
        state.preview_guide = guide;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                bettercut_ui::panels::preview(ui, &mut editor, &mut state, None);
            });
        });
        output.textures_delta.clear();
        output.shapes.len()
    };
    let plain = count(PreviewGuide::Off);
    assert_eq!(count(PreviewGuide::Thirds), plain + 4);
    assert_eq!(count(PreviewGuide::Social), plain + 4);
}

/// Dragging a picture near the middle or an edge snaps it there, reporting
/// the line it caught; far from both, it goes where it was put.
#[test]
fn a_moved_picture_snaps_to_the_middle_and_the_edges() {
    use bettercut_editor_core::timeline::Vec2;
    use bettercut_ui::preview_overlay::snap_position;

    // 400 px wide: one frame unit is 400 px, so the reach is 8 / 400 = 0.02.
    let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
    let half = [0.25, 0.25];

    let near_middle = snap_position(Vec2::new(0.01, 0.3), half, canvas);
    assert_eq!(near_middle.position.x, 0.0);
    assert_eq!(near_middle.vertical, Some(0.0));

    // The picture's left edge (x - 0.25) near the frame's left (-0.5).
    let near_left = snap_position(Vec2::new(-0.24, 0.0), half, canvas);
    assert!(
        (near_left.position.x + 0.25).abs() < 1e-6,
        "{:?}",
        near_left.position
    );
    assert_eq!(near_left.vertical, Some(-0.5));
    assert_eq!(near_left.horizontal, Some(0.0), "y was already centred");

    let free = snap_position(Vec2::new(0.1, 0.1), half, canvas);
    assert_eq!(free.position, Vec2::new(0.1, 0.1));
    assert_eq!((free.vertical, free.horizontal), (None, None));
}
