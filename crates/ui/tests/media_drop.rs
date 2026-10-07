//! A file dragged from the media panel and let go over the timeline lands
//! where it was dropped, without covering what is there (`Editor::drop_plan`).
//! Headless: the drag is started as the media panel starts it, then real
//! pointer events move it over the canvas and let go.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::library::LibraryDrag;
use egui::{Modifiers, Pos2, RawInput, Rect, vec2};

/// Mirrors `theme::TRACK_HEADER_WIDTH` and `RULER_HEIGHT`; 30 px a second is
/// the default zoom.
const HEADER_W: f32 = 148.0;
const RULER_H: f32 = 26.0;
const PX_PER_SECOND: f32 = 30.0;

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    state: UiState,
}

impl Harness {
    fn new() -> Self {
        let (editor, _events) = Editor::new_project("Drop");
        Self {
            ctx: egui::Context::default(),
            editor,
            state: UiState::default(),
        }
    }

    fn import(&mut self, name: &str, seconds: i64) -> MediaId {
        self.editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(seconds),
        ))
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 600.0))),
            events,
            ..Default::default()
        };
        let (editor, state) = (&mut self.editor, &mut self.state);
        let mut output = self.ctx.run_ui(input, |ui| {
            bettercut_ui::timeline::draw(ui, editor, state);
        });
        output.textures_delta.clear();
    }

    /// Drag `media` in from outside and let go at `seconds` on the lanes.
    fn drop_at(&mut self, media: MediaId, seconds: f32) {
        self.drop_item(LibraryDrag::Media(media), seconds);
    }

    /// Drag `item` in from outside and let go at `seconds`, on the picture
    /// lane (the second, under the title lane).
    fn drop_item(&mut self, item: LibraryDrag, seconds: f32) {
        let pos = Pos2::new(HEADER_W + seconds * PX_PER_SECOND, RULER_H + 58.0 + 29.0);
        // Pressed outside the canvas, where the media panel would be.
        self.frame(vec![egui::Event::PointerMoved(Pos2::new(1190.0, 590.0))]);
        self.state.dragging = Some(item);
        for _ in 0..3 {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
        }
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        }]);
        self.frame(vec![]);
    }

    /// (file name, start, end) of each picture clip, in seconds.
    fn pictures(&self) -> Vec<(String, f64, f64)> {
        let sequence = self.editor.active_sequence().unwrap();
        sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|clip| {
                let name = self
                    .editor
                    .project()
                    .media_asset(clip.media_id)
                    .unwrap()
                    .display_name()
                    .to_owned();
                (
                    name,
                    clip.timeline.start.as_seconds_f64(),
                    clip.timeline.end.as_seconds_f64(),
                )
            })
            .collect()
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn dropped_inside_a_clip_it_goes_in_at_its_nearer_edge() {
    let mut harness = Harness::new();
    let first = harness.import("first", 3);
    harness.editor.place_media(first).unwrap();
    let dropped = harness.import("dropped", 2);

    harness.drop_at(dropped, 1.0);

    assert!(harness.state.dragging.is_none(), "the drag is over");
    let pictures = harness.pictures();
    assert_eq!(pictures.len(), 2, "{pictures:?}");
    assert!(
        pictures[0].0.starts_with("dropped") && near(pictures[0].1, 0.0),
        "{pictures:?}"
    );
    assert!(
        near(pictures[1].1, 2.0),
        "the first clip was pushed along: {pictures:?}"
    );
    assert_eq!(harness.editor.undo_depth(), 2, "one step for the drop");
}

#[test]
fn dropped_after_the_end_it_goes_where_it_was_let_go() {
    let mut harness = Harness::new();
    let first = harness.import("first", 3);
    harness.editor.place_media(first).unwrap();
    let dropped = harness.import("dropped", 2);

    harness.drop_at(dropped, 6.0);

    let pictures = harness.pictures();
    assert!(
        near(pictures[0].1, 0.0) && near(pictures[0].2, 3.0),
        "{pictures:?}"
    );
    assert!(near(pictures[1].1, 6.0), "{pictures:?}");
    assert!(
        harness
            .state
            .selected_clips
            .iter()
            .any(|clip| harness.editor.video_clip(*clip).is_some()),
        "the dropped clip is selected"
    );
}

#[test]
fn a_sticker_lands_at_the_moment_it_is_let_go_and_the_playhead_stays() {
    let mut harness = Harness::new();
    let first = harness.import("first", 6);
    harness.editor.place_media(first).unwrap();
    let playhead = harness.editor.playhead();

    harness.drop_item(LibraryDrag::Sticker("★"), 2.0);

    let sequence = harness.editor.active_sequence().unwrap();
    let sticker = sequence
        .text_tracks
        .iter()
        .flat_map(|t| t.clips())
        .find(|c| c.text == "★")
        .expect("the sticker is on a title lane");
    assert!(
        near(sticker.timeline.start.as_seconds_f64(), 2.0),
        "{:?}",
        sticker.timeline
    );
    assert_eq!(
        harness.editor.playhead(),
        playhead,
        "the playhead did not move"
    );
}

#[test]
fn a_filter_or_effect_lands_on_the_clip_it_is_let_go_on() {
    use bettercut_editor_core::effects::EFFECTS;
    use bettercut_editor_core::filters::Filter;
    let mut harness = Harness::new();
    for name in ["first", "second"] {
        let media = harness.import(name, 3);
        harness.editor.place_media(media).unwrap();
    }
    let clips: Vec<_> = harness.editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.id)
        .collect();

    harness.drop_item(LibraryDrag::Filter(Filter::Vintage), 4.0);
    assert_eq!(harness.editor.filter_of(clips[1]), Some(Filter::Vintage));
    assert_ne!(
        harness.editor.filter_of(clips[0]),
        Some(Filter::Vintage),
        "only the one"
    );

    let glow = EFFECTS.iter().position(|e| e.name == "Glow").unwrap();
    harness.drop_item(LibraryDrag::Effect(glow), 1.0);
    let first = harness.editor.video_clip(clips[0]).unwrap();
    assert!(
        EFFECTS[glow].amount_on(first) > 0.0,
        "glow on the first clip"
    );
}

#[test]
fn a_transition_lands_on_the_cut_at_the_end_of_the_clip() {
    use bettercut_editor_core::timeline::TransitionKind;
    let mut harness = Harness::new();
    for name in ["first", "second"] {
        let media = harness.import(name, 3);
        harness.editor.place_media(media).unwrap();
    }
    let first = harness.editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;

    harness.drop_item(LibraryDrag::Transition(TransitionKind::Wipe), 1.5);

    let transition = harness
        .editor
        .video_clip(first)
        .and_then(|c| c.transition_out)
        .expect("a transition on the first clip's end");
    assert_eq!(transition.kind, TransitionKind::Wipe);
}
