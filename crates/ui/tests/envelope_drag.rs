//! Dragging a volume point on the timeline (§24, §53).
//!
//! Seeing the duck drawn on the clip invites the next thing a user tries:
//! grabbing it. These are the rules that has to keep — the point follows the
//! pointer in both axes, the whole drag is one undo step, and a press on a
//! point moves the point rather than the clip under it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Modifiers, Pos2, RawInput, Rect, vec2};

/// Mirrors the default zoom, for moving a point a known distance in time.
const PX_PER_SECOND: f32 = 30.0;

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    state: UiState,
}

impl Harness {
    /// Thirty seconds of sound on A1 from zero, with a duck in the middle.
    fn new() -> (Self, ClipId) {
        let (mut editor, _events) = Editor::new_project("Envelope");
        let mut asset = MediaAsset::new(
            MediaKind::Audio,
            "C:/media/bed.wav",
            MediaTime::from_seconds(30),
        );
        asset.audio_codec = Some("pcm".to_owned());
        let media = editor.import_media(asset);
        let sound = editor.place_media(media).unwrap()[0];
        editor
            .set_gain_envelope(
                sound,
                &[
                    (TimelineTime::from_seconds(4), 1.0),
                    (TimelineTime::from_seconds(6), 0.25),
                    (TimelineTime::from_seconds(10), 0.25),
                    (TimelineTime::from_seconds(12), 1.0),
                ],
                false,
            )
            .unwrap();

        (
            Self {
                ctx: egui::Context::default(),
                editor,
                state: UiState::default(),
            },
            sound,
        )
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 600.0))),
            events,
            ..Default::default()
        };
        let editor = &mut self.editor;
        let state = &mut self.state;
        let mut output = self.ctx.run_ui(input, |ui| {
            bettercut_ui::timeline::draw(ui, editor, state);
        });
        output.textures_delta.clear();
    }

    fn press(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        }]);
    }

    fn drag_to(&mut self, pos: Pos2) {
        for _ in 0..3 {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
        }
    }

    fn release(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        }]);
        self.frame(vec![]);
    }

    /// Where the timeline actually drew each volume point, left to right.
    ///
    /// Read from the frame rather than worked out from the layout constants:
    /// a test that recomputes the lane geometry is testing its own arithmetic,
    /// and would keep passing if the dots moved somewhere the pointer cannot
    /// reach them.
    fn dots(&mut self) -> Vec<Pos2> {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 600.0))),
            ..Default::default()
        };
        let editor = &mut self.editor;
        let state = &mut self.state;
        let mut output = self.ctx.run_ui(input, |ui| {
            bettercut_ui::timeline::draw(ui, editor, state);
        });
        output.textures_delta.clear();

        fn walk(shape: &egui::Shape, out: &mut Vec<Pos2>) {
            match shape {
                egui::Shape::Circle(circle)
                    if circle.fill == egui::Color32::from_rgb(250, 226, 138) =>
                {
                    out.push(circle.center);
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut dots = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut dots);
        }
        dots.sort_by(|a, b| a.x.total_cmp(&b.x));
        dots
    }

    fn double_click(&mut self, pos: Pos2) {
        self.press(pos);
        self.release(pos);
        self.press(pos);
        self.release(pos);
    }

    fn gain_at(&self, clip: ClipId, seconds: i64) -> f32 {
        self.editor
            .audio_clip(clip)
            .unwrap()
            .gain_at(TimelineTime::from_seconds(seconds))
    }
}

#[test]
fn dragging_a_point_down_deepens_the_duck() {
    let (mut harness, sound) = Harness::new();
    assert_eq!(harness.gain_at(sound, 8), 0.25);

    // Grab the point at 6 s and pull it well below the band: it clamps to
    // silence rather than running off the clip.
    let point = harness.dots()[1];
    let floor = Pos2::new(point.x, point.y + 60.0);
    harness.press(point);
    harness.drag_to(floor);
    harness.release(floor);

    let after = harness.gain_at(sound, 6);
    assert!(
        after < 0.1,
        "the point did not follow the pointer: {after} at six seconds"
    );
}

/// A duck is adjusted as much by moving *when* it happens as by how deep it is.
#[test]
fn dragging_a_point_sideways_moves_the_duck_in_time() {
    let (mut harness, sound) = Harness::new();
    // At five seconds the envelope is halfway down the 4-6 s ramp.
    let before = harness.gain_at(sound, 5);
    assert!((before - 0.625).abs() < 0.05, "starting shape: {before}");

    // Drag the 6 s point left to 5 s: the ramp is now steeper and the duck
    // reaches full depth a second earlier.
    let point = harness.dots()[1];
    let earlier = Pos2::new(point.x - PX_PER_SECOND, point.y);
    harness.press(point);
    harness.drag_to(earlier);
    harness.release(earlier);

    let after = harness.gain_at(sound, 5);
    assert!(
        after < before - 0.2,
        "the duck did not move earlier: {before} then {after}"
    );
}

/// §11: the history holds intentions, not mouse samples. A drag across a dozen
/// frames is one thing the user did.
#[test]
fn a_whole_drag_is_one_undo_step() {
    let (mut harness, sound) = Harness::new();
    let before = harness.editor.undo_depth();

    let point = harness.dots()[1];
    harness.press(point);
    for offset in [-12.0, -6.0, 14.0] {
        harness.drag_to(Pos2::new(point.x, point.y + offset));
    }
    harness.release(Pos2::new(point.x, point.y + 14.0));

    assert_eq!(
        harness.editor.undo_depth(),
        before + 1,
        "the drag left a step per frame in the history"
    );

    harness.editor.undo().unwrap();
    assert_eq!(
        harness.gain_at(sound, 6),
        0.25,
        "one undo did not put the point back where it started"
    );
}

/// A press on a point moves the point. Without that the same press would drag
/// the clip along the timeline, which is a much bigger edit than the one the
/// user asked for.
#[test]
fn pressing_a_point_does_not_drag_the_clip() {
    let (mut harness, sound) = Harness::new();
    let start = harness.editor.audio_clip(sound).unwrap().timeline.start;

    let point = harness.dots()[1];
    let across = Pos2::new(point.x + 3.0 * PX_PER_SECOND, point.y);
    harness.press(point);
    harness.drag_to(across);
    harness.release(across);

    assert_eq!(
        harness.editor.audio_clip(sound).unwrap().timeline.start,
        start,
        "the clip moved instead of the point"
    );
}

/// Points cannot cross. A key dragged past its neighbour is reordered when the
/// envelope is written, so the point under the pointer becomes a *different*
/// key and the shape ends up somewhere nobody asked for — the classic jumping
/// point. Held at its neighbour instead.
#[test]
fn a_point_cannot_be_dragged_past_its_neighbour() {
    let (mut harness, sound) = Harness::new();
    // Past the last key the envelope holds its final level, so the music is at
    // full volume from twelve seconds on.
    assert_eq!(harness.gain_at(sound, 15), 1.0);

    // Drag the 6 s point far to the right, past both later points.
    let point = harness.dots()[1];
    let far = Pos2::new(point.x + 14.0 * PX_PER_SECOND, point.y);
    harness.press(point);
    harness.drag_to(far);
    harness.release(far);

    let keys = harness
        .editor
        .audio_clip(sound)
        .unwrap()
        .keyframes
        .track(bettercut_editor_core::timeline::AnimatedParameter::Gain)
        .unwrap()
        .keys()
        .to_vec();
    assert_eq!(keys.len(), 4, "a key was swallowed by another");
    assert_eq!(
        harness.gain_at(sound, 15),
        1.0,
        "the dragged point ended up past the end of the duck, so the music is still coming back up at fifteen seconds"
    );
    let last = harness
        .editor
        .audio_clip(sound)
        .unwrap()
        .keyframes
        .track(bettercut_editor_core::timeline::AnimatedParameter::Gain)
        .unwrap()
        .keys()
        .last()
        .copied()
        .expect("a key");
    assert_eq!(
        last.time,
        MediaTime::from_seconds(12),
        "the dragged point became the last key, so it crossed the two after it"
    );
}

/// Double-clicking the line puts a point where it can be taken hold of — at
/// the level the line already has, so the shape does not jump.
#[test]
fn double_clicking_the_line_adds_a_point() {
    let (mut harness, sound) = Harness::new();
    let before = harness.dots();
    assert_eq!(before.len(), 4);

    // Halfway along the ducked plateau, between the 6 s and 10 s points.
    let middle = Pos2::new(
        (before[1].x + before[2].x) / 2.0,
        (before[1].y + before[2].y) / 2.0,
    );
    harness.double_click(middle);

    let after = harness.dots();
    assert_eq!(after.len(), 5, "no point was added");
    assert_eq!(
        harness.gain_at(sound, 8),
        0.25,
        "adding a point changed the shape it was added to"
    );
}

#[test]
fn double_clicking_a_point_takes_it_out() {
    let (mut harness, sound) = Harness::new();
    let point = harness.dots()[1];
    harness.double_click(point);

    assert_eq!(harness.dots().len(), 3, "the point was not removed");
    // With the 6 s point gone the duck now ramps all the way from 4 s to 10 s,
    // so the middle of that stretch is no longer at full depth.
    let middle = harness.gain_at(sound, 7);
    assert!(
        middle > 0.3,
        "the shape did not change, so nothing was really removed: {middle}"
    );
}

/// An envelope of one point is a level, not a shape. Emptying it by
/// double-clicking would be a surprise; Clear Ducking is the way to do that.
#[test]
fn the_last_two_points_cannot_be_double_clicked_away() {
    let (mut harness, sound) = Harness::new();
    harness
        .editor
        .set_gain_envelope(
            sound,
            &[
                (TimelineTime::from_seconds(4), 1.0),
                (TimelineTime::from_seconds(8), 0.3),
            ],
            false,
        )
        .unwrap();

    let point = harness.dots()[0];
    harness.double_click(point);

    assert_eq!(
        harness.dots().len(),
        2,
        "an envelope was left with one point"
    );
}

/// A clip with no envelope has no line to click on, so a double-click on it is
/// an ordinary double-click and must not invent an envelope.
#[test]
fn double_clicking_a_clip_without_an_envelope_adds_nothing() {
    let (mut harness, sound) = Harness::new();
    harness.editor.set_gain_envelope(sound, &[], false).unwrap();
    assert!(harness.dots().is_empty());

    harness.double_click(Pos2::new(400.0, 110.0));

    assert!(
        harness.dots().is_empty(),
        "an envelope appeared on a clip that had none"
    );
}

/// The line is a target a few pixels wide — a 1.5 px stroke is not a 1.5 px
/// target — but it is still a *line*: a double-click on the clip well away
/// from it is not a request for a point there.
#[test]
fn the_line_is_a_target_with_edges() {
    let (mut harness, _sound) = Harness::new();
    let dots = harness.dots();
    let on_the_plateau = Pos2::new((dots[1].x + dots[2].x) / 2.0, dots[1].y);

    // A few pixels off is still the line.
    harness.double_click(Pos2::new(on_the_plateau.x, on_the_plateau.y - 4.0));
    assert_eq!(
        harness.dots().len(),
        5,
        "the line was not grabbable just off it"
    );

    // Well away from it is not.
    let (mut harness, _sound) = Harness::new();
    let dots = harness.dots();
    let far = Pos2::new((dots[1].x + dots[2].x) / 2.0, dots[1].y - 22.0);
    harness.double_click(far);
    assert_eq!(
        harness.dots().len(),
        4,
        "a double-click nowhere near the line put a point on it"
    );
}
