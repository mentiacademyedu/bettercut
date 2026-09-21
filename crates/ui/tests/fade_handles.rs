//! Dragging a sound clip's fade handles on the timeline.
//!
//! A fade was a number typed into the Inspector. The handles make it what every
//! editor makes it: grab the corner, pull, and the ramp follows. The rules: the
//! fade follows the pointer, the two fades cannot overlap, the whole drag is
//! one undo step, and grabbing a corner does not drag the clip.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Modifiers, Pos2, RawInput, Rect, vec2};

/// Mirrors the default zoom, for moving a handle a known distance in time.
const PX_PER_SECOND: f32 = 30.0;
/// `theme::FADE_HANDLE`, which is how the handles are found in the frame.
const HANDLE: egui::Color32 = egui::Color32::from_rgb(240, 240, 240);

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    state: UiState,
}

impl Harness {
    /// Twenty seconds of sound on A1 from zero, selected so its handles show.
    fn new() -> (Self, ClipId) {
        let (mut editor, _events) = Editor::new_project("Fades");
        let mut asset = MediaAsset::new(
            MediaKind::Audio,
            "C:/media/bed.wav",
            MediaTime::from_seconds(20),
        );
        asset.audio_codec = Some("pcm".to_owned());
        let media = editor.import_media(asset);
        let sound = editor.place_media(media).unwrap()[0];
        let mut state = UiState::default();
        state.select_only(sound);
        (
            Self {
                ctx: egui::Context::default(),
                editor,
                state,
            },
            sound,
        )
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
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
        output
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

    /// Where the timeline drew the handles, left to right — read from the
    /// frame, so the test presses where a user would see them.
    fn handles(&mut self) -> Vec<Pos2> {
        let output = self.frame(vec![]);
        fn walk(shape: &egui::Shape, out: &mut Vec<Pos2>) {
            match shape {
                egui::Shape::Rect(rect) if rect.fill == HANDLE => out.push(rect.rect.center()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
        }
        found.sort_by(|a, b| a.x.total_cmp(&b.x));
        found
    }

    fn fades(&self, clip: ClipId) -> (TimelineTime, TimelineTime) {
        let clip = self.editor.audio_clip(clip).unwrap();
        (clip.fade_in, clip.fade_out)
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.press(from);
        self.drag_to(Pos2::new((from.x + to.x) / 2.0, from.y));
        self.drag_to(to);
        self.release(to);
    }
}

fn near_seconds(t: TimelineTime, seconds: f64) -> bool {
    (t.ticks() as f64 / 960_000.0 - seconds).abs() < 0.1
}

/// Pulling the left handle three seconds in fades the clip in over three
/// seconds, leaving the fade out alone.
#[test]
fn dragging_the_left_handle_sets_the_fade_in() {
    let (mut harness, sound) = Harness::new();
    let handles = harness.handles();
    assert_eq!(
        handles.len(),
        2,
        "a selected sound clip should show two handles"
    );

    let from = handles[0];
    harness.drag(from, Pos2::new(from.x + 3.0 * PX_PER_SECOND, from.y));

    let (fade_in, fade_out) = harness.fades(sound);
    assert!(near_seconds(fade_in, 3.0), "fade in is {fade_in:?}");
    assert_eq!(fade_out, TimelineTime::ZERO);
}

/// And the right handle, pulled in from the end, fades it out.
#[test]
fn dragging_the_right_handle_sets_the_fade_out() {
    let (mut harness, sound) = Harness::new();
    let from = harness.handles()[1];
    harness.drag(from, Pos2::new(from.x - 2.0 * PX_PER_SECOND, from.y));

    let (fade_in, fade_out) = harness.fades(sound);
    assert_eq!(fade_in, TimelineTime::ZERO);
    assert!(near_seconds(fade_out, 2.0), "fade out is {fade_out:?}");
}

/// Once there is a fade, its handle sits where the fade ends — which is where
/// the user will look for it to change it again.
#[test]
fn a_handle_sits_where_its_fade_ends() {
    let (mut harness, sound) = Harness::new();
    let before = harness.handles();
    harness
        .editor
        .set_clip_fades(
            sound,
            TimelineTime::from_seconds(4),
            TimelineTime::ZERO,
            false,
        )
        .unwrap();

    let after = harness.handles();

    assert!(
        (after[0].x - before[0].x - 4.0 * PX_PER_SECOND).abs() < 10.0,
        "the handle is at {} rather than four seconds in from {}",
        after[0].x,
        before[0].x
    );
}

/// The fade in cannot be pulled past where the fade out begins.
#[test]
fn the_fades_cannot_overlap() {
    let (mut harness, sound) = Harness::new();
    harness
        .editor
        .set_clip_fades(
            sound,
            TimelineTime::ZERO,
            TimelineTime::from_seconds(15),
            false,
        )
        .unwrap();

    let from = harness.handles()[0];
    harness.drag(from, Pos2::new(from.x + 12.0 * PX_PER_SECOND, from.y));

    let (fade_in, fade_out) = harness.fades(sound);
    let length = TimelineTime::from_seconds(20);
    assert!(
        fade_in.ticks() + fade_out.ticks() <= length.ticks(),
        "the fades overlap: {fade_in:?} + {fade_out:?}"
    );
    assert!(
        near_seconds(fade_in, 5.0),
        "fade in stopped at {fade_in:?}, not where the fade out starts"
    );
}

/// A drag is one undo step, and grabbing the corner never moves the clip.
#[test]
fn a_fade_drag_is_one_step_and_leaves_the_clip_in_place() {
    let (mut harness, sound) = Harness::new();
    let depth = harness.editor.undo_depth();
    let start = harness.editor.audio_clip(sound).unwrap().timeline.start;

    let from = harness.handles()[0];
    harness.press(from);
    for step in [1.0, 2.0, 3.0, 4.0] {
        harness.drag_to(Pos2::new(from.x + step * PX_PER_SECOND, from.y));
    }
    harness.release(Pos2::new(from.x + 4.0 * PX_PER_SECOND, from.y));

    assert_eq!(
        harness.editor.undo_depth(),
        depth + 1,
        "the drag left several steps"
    );
    assert_eq!(
        harness.editor.audio_clip(sound).unwrap().timeline.start,
        start,
        "the clip moved"
    );
    harness.editor.undo().unwrap();
    assert_eq!(
        harness.fades(sound),
        (TimelineTime::ZERO, TimelineTime::ZERO)
    );
}

/// A picture has handles too: dragging its left one in gives it a fade-in
/// entrance about as long as the drag, in one undo step.
#[test]
fn a_picture_fade_handle_sets_its_entrance() {
    let (mut editor, _events) = Editor::new_project("Fades");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/v.mp4",
        MediaTime::from_seconds(8),
    ));
    let picture = editor.place_media(media).unwrap()[0];
    let mut harness = Harness {
        ctx: egui::Context::default(),
        editor,
        state: UiState::default(),
    };
    harness.state.select_only(picture);

    let handles = harness.handles();
    assert_eq!(handles.len(), 2, "a selected picture has no handles");
    let depth = harness.editor.undo_depth();
    let start = handles[0];
    harness.drag(start, Pos2::new(start.x + PX_PER_SECOND, start.y));

    let intro = harness.editor.video_clip(picture).unwrap().motion.intro;
    let intro = intro.expect("the drag gave no entrance");
    assert_eq!(
        intro.kind,
        bettercut_editor_core::timeline::MotionKind::Fade
    );
    let seconds = intro.duration.ticks() as f32 / TimelineTime::from_seconds(1).ticks() as f32;
    assert!((0.7..1.3).contains(&seconds), "the fade is {seconds} s");
    assert_eq!(
        harness.editor.undo_depth(),
        depth + 1,
        "the drag was not one step"
    );
    assert_eq!(
        harness
            .editor
            .active_sequence()
            .unwrap()
            .clip_span(picture)
            .unwrap()
            .timeline
            .start,
        TimelineTime::ZERO,
        "grabbing the handle moved the clip"
    );
}

/// A tagged clip shows its colour on the timeline: its top strip takes the
/// label's colour, which is the whole point of tagging it.
#[test]
fn a_colour_label_is_drawn_on_the_clip() {
    use bettercut_editor_core::timeline::ColorLabel;

    let (mut harness, sound) = Harness::new();
    let [r, g, b] = ColorLabel::Purple.rgb().unwrap();
    let purple = egui::Color32::from_rgb(r, g, b);
    let drawn = |harness: &mut Harness| {
        let output = harness.frame(vec![]);
        fn walk(shape: &egui::Shape, colour: egui::Color32) -> bool {
            match shape {
                egui::Shape::Rect(rect) => rect.fill == colour,
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| walk(s, colour)),
                _ => false,
            }
        }
        output
            .shapes
            .iter()
            .any(|clipped| walk(&clipped.shape, purple))
    };
    assert!(!drawn(&mut harness), "setup: purple already on screen");

    harness
        .editor
        .set_color_label(&[sound], ColorLabel::Purple)
        .unwrap();

    assert!(drawn(&mut harness), "the label's colour is not on the clip");
}
