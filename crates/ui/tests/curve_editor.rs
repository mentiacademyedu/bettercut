//! The Keyframe Curve window (`bettercut_ui::curve_editor`): a named curve
//! clicked, or a handle dragged and let go, eases the keys under the playhead
//! — each one step to undo. Headless: real pointer events on the window as
//! drawn, found by what it paints.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, Interpolation, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, ClipProperty, Editor};
use bettercut_ui::UiState;
use bettercut_ui::curve_editor::CurveEditor;
use egui::{Modifiers, Pos2, RawInput, Rect, vec2};

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    state: UiState,
    clip: ClipId,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Harness {
    /// A clip with its scale keyed at the playhead, and the window open on it.
    fn new() -> Self {
        let (mut editor, _rx) = Editor::new_project("Curves");
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/a.mp4",
            MediaTime::from_seconds(60),
        ));
        let source =
            SourceRange::new(MediaTime::from_seconds(0), MediaTime::from_seconds(4)).unwrap();
        let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
        let id = clip.id;
        let track = editor.active_sequence().unwrap().video_tracks[0].id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
        editor
            .toggle_keyframe(id, ClipProperty::Scale { x: 1.0, y: 1.0 })
            .unwrap();
        let mut state = UiState::default();
        state.curve_editor = Some(CurveEditor::open(id));
        let mut harness = Self {
            ctx: egui::Context::default(),
            editor,
            state,
            clip: id,
            shapes: Vec::new(),
        };
        harness.frame(vec![]);
        harness.frame(vec![]);
        harness
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
            events,
            ..Default::default()
        };
        let (editor, state) = (&mut self.editor, &mut self.state);
        let mut output = self.ctx.run_ui(input, |ui| {
            bettercut_ui::curve_editor::show(ui.ctx(), editor, state);
        });
        output.textures_delta.clear();
        self.shapes = output.shapes;
    }

    fn easing(&self) -> Interpolation {
        let at = self.editor.source_time_at_playhead(self.clip).unwrap();
        self.editor
            .video_clip(self.clip)
            .unwrap()
            .keyframes
            .get(AnimatedParameter::ScaleX, at)
            .unwrap()
            .interpolation
    }

    /// Where the window painted `text`, at its middle.
    fn text_at(&self, text: &str) -> Pos2 {
        self.shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.pos + shape.galley.size() / 2.0)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} was not drawn"))
    }

    /// The handles: circles of their size, in the order drawn.
    fn handles(&self) -> Vec<Pos2> {
        self.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Circle(circle) if (circle.radius - 5.0).abs() < 0.01 => {
                    Some(circle.center)
                }
                _ => None,
            })
            .collect()
    }

    fn button(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        }]);
    }

    fn click(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.button(pos, true);
        self.button(pos, false);
        self.frame(vec![]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.button(from, true);
        for step in 1..=6 {
            let at = from + (to - from) * (step as f32 / 6.0);
            self.frame(vec![egui::Event::PointerMoved(at)]);
        }
        self.button(to, false);
        self.frame(vec![]);
    }
}

fn named(wanted: &str) -> Interpolation {
    Interpolation::CURVES
        .into_iter()
        .find(|(name, _)| *name == wanted)
        .unwrap()
        .1
}

#[test]
fn a_named_curve_clicked_eases_the_keys_in_one_step() {
    let mut harness = Harness::new();
    assert_eq!(harness.easing(), Interpolation::Linear);

    // The tile is the picture above its name.
    let name = harness.text_at("overshoot");
    harness.click(name - vec2(0.0, 20.0));
    assert_eq!(harness.easing(), named("overshoot"));

    harness.editor.undo().unwrap();
    assert_eq!(harness.easing(), Interpolation::Linear, "not one undo step");
}

#[test]
fn a_dragged_handle_draws_a_curve_of_its_own() {
    let mut harness = Harness::new();
    harness.click(harness.text_at("ease in-out") - vec2(0.0, 20.0));
    harness.frame(vec![]);
    let handles = harness.handles();
    assert_eq!(handles.len(), 2, "two handles, one per key");

    // The first handle pulled up past the "to" line: an overshoot leaving.
    harness.drag(handles[0], handles[0] - vec2(0.0, 120.0));
    let Interpolation::Bezier { x1, y1, x2, y2 } = harness.easing() else {
        panic!("a dragged curve is drawn, got {:?}", harness.easing());
    };
    assert!((x1 - 0.42).abs() < 0.05, "x1 {x1}");
    assert!(y1 > 1.0, "y1 {y1} not past the line");
    assert_eq!((x2, y2), (0.58, 1.0), "the other handle moved");
    assert_eq!(harness.easing().label(), "custom");
    assert!(
        harness.easing().ease(0.5) > 1.0,
        "dragged up, it overshoots"
    );

    // Picking and dragging are a step each.
    harness.editor.undo().unwrap();
    assert_eq!(harness.easing(), Interpolation::EaseInOut);
}

#[test]
fn off_a_key_the_window_says_so_and_changes_nothing() {
    let mut harness = Harness::new();
    harness.editor.set_playhead(TimelineTime::from_seconds(2));
    harness.frame(vec![]);
    harness.text_at("Move the playhead onto one of this clip's keyframes.");
    assert!(harness.handles().is_empty());
}
