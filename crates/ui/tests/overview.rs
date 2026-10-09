//! The overview strip along the bottom of the timeline: the whole edit at a
//! glance, and a click on it moves the view there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Modifiers, Pos2, RawInput, Rect, Vec2, vec2};

/// Mirrors `theme::OVERVIEW_HEIGHT` and `TRACK_HEADER_WIDTH`.
const OVERVIEW_H: f32 = 30.0;
const HEADER_W: f32 = bettercut_ui::theme::TRACK_HEADER_WIDTH;
const SCREEN: Vec2 = vec2(1200.0, 600.0);

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    state: UiState,
}

impl Harness {
    /// Ten minutes of footage on one track — far more than a screen holds at
    /// the default zoom, which is when an overview is worth anything.
    fn new() -> Self {
        let (mut editor, _events) = Editor::new_project("Overview");
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/long.mp4",
            MediaTime::from_seconds(600),
        ));
        editor.place_media(media).unwrap();
        Self {
            ctx: egui::Context::default(),
            editor,
            state: UiState::default(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
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

    fn click(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            }]);
        }
        self.frame(vec![]);
    }

    /// A point inside the strip, `through` of the way along it.
    fn in_strip(&self, through: f32) -> Pos2 {
        let left = 4.0;
        let width = SCREEN.x - 8.0;
        Pos2::new(left + through * width, SCREEN.y - OVERVIEW_H / 2.0)
    }
}

#[test]
fn clicking_the_strip_moves_the_view_there() {
    let mut harness = Harness::new();
    harness.frame(vec![]);
    assert_eq!(harness.state.scroll_ticks, 0);

    harness.click(harness.in_strip(0.5));

    // Halfway through ten minutes, centred: five minutes, less half a screen.
    let lane_width = SCREEN.x - HEADER_W;
    let half_view = (lane_width as i64) * harness.state.ticks_per_pixel() / 2;
    let wanted = TimelineTime::from_seconds(300).ticks() - half_view;
    assert!(
        (harness.state.scroll_ticks - wanted).abs() < TimelineTime::from_seconds(2).ticks(),
        "scrolled to {} rather than about {wanted}",
        harness.state.scroll_ticks
    );
}

#[test]
fn a_click_at_the_start_of_the_strip_stays_at_the_start() {
    let mut harness = Harness::new();
    harness.click(harness.in_strip(0.5));
    assert!(harness.state.scroll_ticks > 0);

    harness.click(harness.in_strip(0.0));
    assert_eq!(harness.state.scroll_ticks, 0);
}

/// The strip owns its own gesture: clicking it must not also drop the playhead
/// somewhere, which is what a click anywhere else on the canvas does.
#[test]
fn clicking_the_strip_leaves_the_playhead_alone() {
    let mut harness = Harness::new();
    let was = harness.editor.playhead();
    harness.click(harness.in_strip(0.7));
    assert_eq!(harness.editor.playhead(), was);
}
