//! Driving the preview's handles with real pointer events.
//!
//! The geometry is tested beside `preview_overlay`, and checked against a real
//! render in `preview_handles_gpu`. Neither of those touches the part that
//! turns a press and a drag into a command — which is where "only one corner
//! works" would live, and which no amount of arithmetic testing would catch.
//!
//! egui runs headless, so this feeds it pointer positions and button events and
//! then asks the project what happened.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Resolution, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};
use bettercut_ui::UiState;
use egui::{Event, Modifiers, Pos2, RawInput, Rect, Vec2, pos2, vec2};

/// The window the test pretends to be.
const WINDOW: Vec2 = vec2(900.0, 600.0);

/// A project with one 16:9 clip under the playhead, in a 16:9 sequence, so the
/// picture fills the frame exactly and its handles sit on the canvas corners.
fn editor_with_clip() -> (Editor, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Preview gestures");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = editor.import_media(asset);

    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    editor
        .set_sequence_format(
            Resolution::HD_1080,
            editor.active_sequence().unwrap().frame_rate,
        )
        .expect("format");
    editor.set_playhead(TimelineTime::from_seconds(1));
    (editor, id)
}

/// A live egui context across frames.
///
/// **One context for the whole gesture.** egui remembers a press from one frame
/// to the next — that is what makes it a drag rather than two unrelated clicks —
/// so a fresh `Context` per frame reports every drag as nothing happening. The
/// first version of this file did exactly that and failed all five tests,
/// including click-to-select, which works perfectly in the application.
struct Harness {
    ctx: egui::Context,
    /// Advanced every frame. egui will not call a pointer movement a drag
    /// without a clock: leaving this out reported every drag as nothing
    /// happening, which is what the first version of this file did.
    time: f64,
}

impl Harness {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    /// One frame: move the pointer, deliver `events`, draw the preview.
    fn frame(
        &mut self,
        editor: &mut Editor,
        state: &mut UiState,
        pointer: Pos2,
        events: Vec<Event>,
    ) {
        let mut all = vec![Event::PointerMoved(pointer)];
        all.extend(events);

        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, WINDOW)),
            events: all,
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        };

        let mut output = self.ctx.run_ui(input, |ui| {
            bettercut_ui::panels::preview(ui, editor, state, None);
        });
        output.textures_delta.clear();
    }
}

fn press(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::default(),
    }
}

fn release(at: Pos2) -> Event {
    Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::default(),
    }
}

/// Where the preview draws its canvas, mirroring `panels::preview`: the
/// sequence's aspect, letterboxed inside the panel with a 24px margin.
///
/// The panel is the whole window here, because the test draws only the preview.
fn canvas() -> Rect {
    let panel = Rect::from_min_size(Pos2::ZERO, WINDOW);
    let aspect = 16.0 / 9.0_f32;
    let mut size = vec2(panel.width() - 24.0, (panel.width() - 24.0) / aspect);
    if size.y > panel.height() - 24.0 {
        size = vec2((panel.height() - 24.0) * aspect, panel.height() - 24.0);
    }
    Rect::from_center_size(panel.center(), size)
}

fn scale_of(editor: &Editor, clip: ClipId) -> f32 {
    editor.video_clip(clip).unwrap().transform.scale.x
}

/// Put the clip at half size, so its box sits well inside the canvas and every
/// corner has room to be dragged in both directions.
fn shrink_to_half(editor: &mut Editor, clip: ClipId) {
    editor
        .set_clip_value(
            clip,
            bettercut_editor_core::ClipProperty::Scale { x: 0.5, y: 0.5 },
            false,
        )
        .expect("scale");
}

fn position_of(editor: &Editor, clip: ClipId) -> (f32, f32) {
    let t = editor.video_clip(clip).unwrap().transform;
    (t.position.x, t.position.y)
}

/// Press at `from`, drag to `to` in steps, release.
///
/// **In steps, like a real mouse.** egui only calls a movement a drag once it
/// has passed a threshold, and it reports `drag_started` on the frame that
/// happens — by which time the pointer is wherever the test put it. Jumping
/// straight to the destination therefore starts the gesture *at* the
/// destination and records no movement at all, which is what made every drag
/// test here look like a broken feature.
fn drag(editor: &mut Editor, state: &mut UiState, from: Pos2, to: Pos2) {
    const STEPS: u32 = 12;

    let mut harness = Harness::new();
    // A frame with the pointer in place before the press, so the widget exists
    // and egui knows where the pointer is when the button goes down.
    harness.frame(editor, state, from, vec![]);
    harness.frame(editor, state, from, vec![press(from)]);

    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let at = from + (to - from) * t;
        harness.frame(editor, state, at, vec![]);
    }
    harness.frame(editor, state, to, vec![release(to)]);
}

/// Press and release without moving.
fn click(editor: &mut Editor, state: &mut UiState, at: Pos2) {
    let mut harness = Harness::new();
    harness.frame(editor, state, at, vec![]);
    harness.frame(editor, state, at, vec![press(at)]);
    harness.frame(editor, state, at, vec![release(at)]);
    harness.frame(editor, state, at, vec![]);
}

/// The reported bug: only one corner resized the picture.
///
/// Every corner is dragged the same distance away from the centre, so all four
/// should scale up by about the same amount. Results are collected rather than
/// asserted one at a time, because "which corners work" is the whole question
/// and stopping at the first failure hides it.
///
/// The clip starts at half size. At full size the box *is* the canvas, so
/// dragging a corner outward leaves the window — which looks exactly like a
/// broken corner and is really a test with nowhere to drag.
#[test]
fn every_corner_scales_the_picture() {
    let canvas = canvas();
    let cases = [
        ("top left", canvas.left_top(), vec2(-1.0, -1.0)),
        ("top right", canvas.right_top(), vec2(1.0, -1.0)),
        ("bottom right", canvas.right_bottom(), vec2(1.0, 1.0)),
        ("bottom left", canvas.left_bottom(), vec2(-1.0, 1.0)),
    ];

    let mut results = Vec::new();
    for (name, _, direction) in cases {
        let (mut editor, clip) = editor_with_clip();
        let mut state = UiState::default();
        state.selected_clips.insert(clip);
        shrink_to_half(&mut editor, clip);

        // Half size, so the box is a quarter of the canvas around the centre.
        let corner = canvas.center()
            + vec2(
                direction.x * canvas.width() / 4.0,
                direction.y * canvas.height() / 4.0,
            );
        let before = scale_of(&editor, clip);
        drag(&mut editor, &mut state, corner, corner + direction * 80.0);
        results.push((name, before, scale_of(&editor, clip)));
    }

    let broken: Vec<String> = results
        .iter()
        .filter(|(_, before, after)| after <= &(before * 1.05))
        .map(|(name, before, after)| format!("{name}: {before} -> {after}"))
        .collect();

    assert!(
        broken.is_empty(),
        "these corners did not enlarge the picture:
  {}",
        broken.join(
            "
  "
        )
    );
}

/// And inward shrinks it, from every corner.
#[test]
fn every_corner_shrinks_the_picture_when_dragged_inward() {
    let canvas = canvas();
    let cases = [
        ("top left", vec2(-1.0, -1.0)),
        ("top right", vec2(1.0, -1.0)),
        ("bottom right", vec2(1.0, 1.0)),
        ("bottom left", vec2(-1.0, 1.0)),
    ];

    let mut results = Vec::new();
    for (name, direction) in cases {
        let (mut editor, clip) = editor_with_clip();
        let mut state = UiState::default();
        state.selected_clips.insert(clip);
        shrink_to_half(&mut editor, clip);

        let corner = canvas.center()
            + vec2(
                direction.x * canvas.width() / 4.0,
                direction.y * canvas.height() / 4.0,
            );
        let before = scale_of(&editor, clip);
        // Inward: back towards the centre.
        drag(&mut editor, &mut state, corner, corner - direction * 60.0);
        results.push((name, before, scale_of(&editor, clip)));
    }

    let broken: Vec<String> = results
        .iter()
        .filter(|(_, before, after)| after >= &(before * 0.95))
        .map(|(name, before, after)| format!("{name}: {before} -> {after}"))
        .collect();

    assert!(
        broken.is_empty(),
        "these corners did not shrink the picture:
  {}",
        broken.join(
            "
  "
        )
    );
}

/// Dragging the middle moves rather than scales.
#[test]
fn dragging_the_middle_moves_the_picture() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    state.selected_clips.insert(clip);

    let canvas = canvas();
    drag(
        &mut editor,
        &mut state,
        canvas.center(),
        canvas.center() + vec2(90.0, 45.0),
    );

    let (x, y) = position_of(&editor, clip);
    assert!(x > 0.05, "did not move right: {x}");
    assert!(y > 0.05, "did not move down: {y}");
    assert!(
        (scale_of(&editor, clip) - 1.0).abs() < 1e-4,
        "a move should not change the scale"
    );
}

/// Clicking the picture selects it, so the timeline is not the only way in.
#[test]
fn clicking_the_picture_selects_the_clip() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    assert!(state.selected_clips.is_empty());

    click(&mut editor, &mut state, canvas().center());

    assert!(state.selected_clips.contains(&clip));
}

/// Clicking the black area around the picture clears the selection.
#[test]
fn clicking_beside_the_picture_clears_the_selection() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    state.selected_clips.insert(clip);

    // The very corner of the panel, well outside the letterboxed canvas.
    click(&mut editor, &mut state, pos2(4.0, 4.0));

    assert!(state.selected_clips.is_empty());
}

// ---- §22's crop, dragged on the picture ------------------------------------

fn crop_of(editor: &Editor, clip: ClipId) -> bettercut_editor_core::timeline::Crop {
    editor.video_clip(clip).unwrap().crop
}

/// The picture's left-edge handle, with the clip at half size: the box is a
/// quarter of the canvas either side of the centre.
fn left_edge_handle() -> Pos2 {
    let canvas = canvas();
    canvas.center() - vec2(canvas.width() / 4.0, 0.0)
}

/// In crop mode, dragging the left edge inwards crops the left of the shot —
/// and only that. Nothing moves, nothing scales.
#[test]
fn dragging_a_crop_edge_crops_that_side() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    state.selected_clips.insert(clip);
    shrink_to_half(&mut editor, clip);
    state.cropping = Some(clip);

    let from = left_edge_handle();
    drag(&mut editor, &mut state, from, from + vec2(60.0, 0.0));

    let crop = crop_of(&editor, clip);
    assert!(crop.left > 0.05, "the left edge did not crop: {crop:?}");
    assert_eq!(crop.right, 0.0, "the right edge moved too: {crop:?}");
    assert_eq!(crop.top, 0.0);
    assert_eq!(crop.bottom, 0.0);

    assert_eq!(
        position_of(&editor, clip),
        (0.0, 0.0),
        "the crop drag moved the clip"
    );
    assert!(
        (scale_of(&editor, clip) - 0.5).abs() < 1e-4,
        "the crop drag scaled the clip"
    );
}

/// Crop mode is exclusive: a corner that would scale the picture normally does
/// nothing while crop edges are out, because a drag meant for one handle
/// landing on the other is exactly the surprise the mode exists to prevent.
#[test]
fn a_corner_does_not_scale_in_crop_mode() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    state.selected_clips.insert(clip);
    shrink_to_half(&mut editor, clip);
    state.cropping = Some(clip);

    let canvas = canvas();
    let corner = canvas.center() + vec2(canvas.width() / 4.0, canvas.height() / 4.0);
    drag(&mut editor, &mut state, corner, corner + vec2(80.0, 80.0));

    assert!(
        (scale_of(&editor, clip) - 0.5).abs() < 1e-4,
        "a corner scaled the picture while cropping"
    );
    assert!(
        crop_of(&editor, clip).is_none(),
        "a corner cropped the picture"
    );
}

/// And out of crop mode, the same press on the edge's middle is a move, not a
/// crop — the edge handles are only there when asked for.
#[test]
fn an_edge_does_not_crop_outside_crop_mode() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    state.selected_clips.insert(clip);
    shrink_to_half(&mut editor, clip);

    let from = left_edge_handle();
    drag(&mut editor, &mut state, from, from + vec2(60.0, 0.0));

    assert!(
        crop_of(&editor, clip).is_none(),
        "the picture was cropped without crop mode on"
    );
}

/// §11: a crop drag is one undo step, however many frames it took.
#[test]
fn a_crop_drag_is_one_undo_step() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    state.selected_clips.insert(clip);
    shrink_to_half(&mut editor, clip);
    state.cropping = Some(clip);

    let before = editor.undo_depth();
    let from = left_edge_handle();
    drag(&mut editor, &mut state, from, from + vec2(60.0, 0.0));
    assert!(
        !crop_of(&editor, clip).is_none(),
        "nothing was cropped to undo"
    );
    assert_eq!(
        editor.undo_depth(),
        before + 1,
        "the drag left several history entries"
    );

    editor.undo().unwrap();
    assert!(
        crop_of(&editor, clip).is_none(),
        "undo did not take the crop back"
    );
}
