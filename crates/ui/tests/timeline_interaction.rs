//! Headless tests for the timeline canvas (§53).
//!
//! The canvas has no widgets, so nothing about dragging, trimming, or snapping
//! is covered by the core's unit tests — the interaction logic lives here and
//! would otherwise only ever be tested by hand.
//!
//! `egui::Context::run_ui` runs a full layout-and-interaction pass with no
//! window, so real pointer events can be fed in and the resulting `Editor`
//! state asserted.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};
use bettercut_ui::UiState;
use egui::{Modifiers, Pos2, RawInput, Rect, Vec2, vec2};

/// Mirrors `theme::TRACK_HEADER_WIDTH`, `RULER_HEIGHT`, `TRACK_HEIGHT`.
const HEADER_W: f32 = 148.0;
const RULER_H: f32 = 26.0;
const TRACK_H: f32 = 58.0;
const SCREEN: Vec2 = vec2(1200.0, 600.0);

/// Ticks in one second, for readable test positions.
fn secs(n: i64) -> i64 {
    TimelineTime::from_seconds(n).ticks()
}

/// Keeps the egui context alive across frames, because a drag is inherently
/// multi-frame: press, move, release.
struct Harness {
    ctx: egui::Context,
    editor: Editor,
    state: UiState,
    /// Empty space left above the timeline, standing in for the toolbar and
    /// preview panel. Zero means the canvas fills the screen.
    top_offset: f32,
}

impl Harness {
    fn new() -> Self {
        let (editor, _events) = Editor::new_project("Interaction");
        Self {
            ctx: egui::Context::default(),
            editor,
            state: UiState::default(),
            top_offset: 0.0,
        }
    }

    /// Lay the timeline out at the bottom, as the real window does.
    ///
    /// Needed to test anything about the pointer being *outside* the canvas:
    /// with the default full-screen layout there is no outside.
    fn with_panels_above(offset: f32) -> Self {
        Self {
            top_offset: offset,
            ..Self::new()
        }
    }

    fn video_track(&self) -> TrackId {
        self.editor.active_sequence().unwrap().video_tracks[0].id
    }

    /// Add a clip using **seconds**.
    ///
    /// At the default zoom of 30 px/s that is what makes a clip wide enough to
    /// grab. A clip a few thousand ticks long is two pixels wide, and every
    /// drag on it lands on a trim handle instead of its body.
    fn add_clip(&mut self, start_secs: i64, len_secs: i64) -> ClipId {
        let media = self.editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{start_secs}-{len_secs}.mp4"),
            MediaTime::from_seconds(600),
        ));
        // The source starts well inside the media, so trims have room in both
        // directions without hitting the media's own bounds.
        let source = SourceRange::new(
            MediaTime::from_seconds(60),
            MediaTime::from_seconds(60 + len_secs),
        )
        .unwrap();
        let clip = VideoClip::new(media, TimelineTime::from_seconds(start_secs), source).unwrap();
        let id = clip.id;
        let track = self.video_track();
        self.editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .expect("clip should be accepted");
        id
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            events,
            ..Default::default()
        };

        let editor = &mut self.editor;
        let state = &mut self.state;
        let offset = self.top_offset;
        let mut output = self.ctx.run_ui(input, |ui| {
            if offset <= 0.0 {
                bettercut_ui::timeline::draw(ui, editor, state);
                return;
            }
            let full = ui.available_rect_before_wrap();
            let below = Rect::from_min_max(Pos2::new(full.left(), full.top() + offset), full.max);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(below));
            bettercut_ui::timeline::draw(&mut child, editor, state);
        });

        // egui hands back font atlas updates that a real backend would upload.
        // There is no backend here, and dropping them unhandled panics.
        output.textures_delta.clear();
    }

    fn press(&mut self, pos: Pos2) {
        // Move first so egui knows where the pointer is before the button goes
        // down; a press with no prior position is ignored.
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        }]);
    }

    fn drag_to(&mut self, pos: Pos2) {
        // Several frames: egui needs one to register the drag, and the canvas
        // updates its preview on each.
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
        // One more pass so anything committed on release is reflected.
        self.frame(vec![]);
    }

    fn click(&mut self, pos: Pos2) {
        self.press(pos);
        self.release(pos);
    }

    /// Ctrl+click: adds to the selection rather than replacing it.
    ///
    /// The held-modifier state comes from `ModifiersChanged`, not from the
    /// pointer event's own `modifiers` field — without it `i.modifiers` stays
    /// empty and the click reads as a plain one.
    fn ctrl_click(&mut self, pos: Pos2) {
        let modifiers = Modifiers {
            command: true,
            ctrl: true,
            ..Modifiers::default()
        };
        self.frame(vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
        ]);
        for pressed in [true, false] {
            self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            }]);
        }
        // Release them, or every later click in the test is additive too.
        self.frame(vec![egui::Event::ModifiersChanged(Modifiers::default())]);
    }

    fn right_click(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::default(),
        }]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::default(),
        }]);
    }

    /// Screen x of a timeline instant, mirroring the canvas's own mapping.
    fn x_of(&self, ticks: i64) -> f32 {
        HEADER_W + ((ticks - self.state.scroll_ticks) / self.state.ticks_per_pixel()) as f32
    }

    /// Vertical centre of the first video lane. With one video and one audio
    /// track, the video lane is drawn first, just under the ruler.
    fn video_lane_y(&self) -> f32 {
        RULER_H + 2.0 + TRACK_H / 2.0
    }

    fn tpp(&self) -> i64 {
        self.state.ticks_per_pixel()
    }

    fn clip_start(&self, id: ClipId) -> i64 {
        self.editor.active_sequence().unwrap().video_tracks[0]
            .get(id)
            .expect("clip exists")
            .timeline
            .start
            .ticks()
    }

    fn clip_end(&self, id: ClipId) -> i64 {
        self.editor.active_sequence().unwrap().video_tracks[0]
            .get(id)
            .expect("clip exists")
            .timeline
            .end
            .ticks()
    }
}

/// Moving the mouse over the preview or toolbar must not touch the playhead.
///
/// It did: the ruler test was `pos.y < rect.top() + RULER_HEIGHT` against a
/// position taken from `pointer_latest_pos`, which is the pointer anywhere in
/// the window. Everything above the canvas therefore read as "on the ruler",
/// and simply moving the mouse across the upper half of the app scrubbed.
#[test]
fn moving_the_pointer_above_the_timeline_does_not_scrub() {
    let mut h = Harness::with_panels_above(300.0);
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::from_seconds(3));
    let before = h.editor.playhead().ticks();

    // Across the preview area, left to right — no buttons held.
    for x in [200.0, 500.0, 900.0] {
        h.frame(vec![egui::Event::PointerMoved(Pos2::new(x, 120.0))]);
    }

    assert_eq!(
        h.editor.playhead().ticks(),
        before,
        "the playhead followed a pointer that was never over the timeline"
    );
}

/// The same, but with the button held — dragging in the preview panel.
///
/// `response.dragged()` is false here because the press did not land on the
/// canvas, so only the "is the pointer actually inside this widget?" check
/// stands between a preview-area drag and the playhead.
#[test]
fn dragging_above_the_timeline_does_not_scrub() {
    let mut h = Harness::with_panels_above(300.0);
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::from_seconds(3));
    let before = h.editor.playhead().ticks();

    h.press(Pos2::new(200.0, 120.0));
    h.drag_to(Pos2::new(900.0, 140.0));
    h.release(Pos2::new(900.0, 140.0));

    assert_eq!(
        h.editor.playhead().ticks(),
        before,
        "a drag in the preview area scrubbed the timeline"
    );
}

/// Hovering the ruler is not scrubbing either — the button has to be down.
#[test]
fn hovering_the_ruler_does_not_scrub() {
    let mut h = Harness::new();
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::from_seconds(3));
    let before = h.editor.playhead().ticks();

    let x = h.x_of(secs(8));
    h.frame(vec![egui::Event::PointerMoved(Pos2::new(x, RULER_H / 2.0))]);
    h.frame(vec![egui::Event::PointerMoved(Pos2::new(x, RULER_H / 2.0))]);

    assert_eq!(
        h.editor.playhead().ticks(),
        before,
        "merely hovering the ruler moved the playhead"
    );
}

/// The other half of the same fix: pressing on the ruler must still scrub, or
/// the bug would have been traded for a dead control.
#[test]
fn pressing_the_ruler_scrubs() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    let target = secs(8);
    let x = h.x_of(target);
    h.press(Pos2::new(x, RULER_H / 2.0));

    let landed = h.editor.playhead().ticks();
    assert!(
        (landed - target).abs() <= h.tpp() * 2,
        "pressing the ruler at {target} ticks put the playhead at {landed}"
    );
}

/// Dragging along the ruler keeps scrubbing, including past the canvas edge —
/// releasing outside the window is normal and must not strand the drag.
#[test]
fn dragging_along_the_ruler_keeps_scrubbing() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    h.press(Pos2::new(h.x_of(secs(4)), RULER_H / 2.0));
    let target = secs(14);
    h.drag_to(Pos2::new(h.x_of(target), RULER_H / 2.0));

    let landed = h.editor.playhead().ticks();
    assert!(
        (landed - target).abs() <= h.tpp() * 2,
        "dragging the ruler to {target} ticks left the playhead at {landed}"
    );
    h.release(Pos2::new(h.x_of(target), RULER_H / 2.0));
}

// ---- rubber-band selection -----------------------------------------------

/// §10 "Multi-select clips": drag a box over empty canvas and everything it
/// touches is selected.
#[test]
fn dragging_over_empty_canvas_selects_the_clips_it_covers() {
    let mut h = Harness::new();
    let first = h.add_clip(2, 3);
    let second = h.add_clip(7, 3);
    h.add_clip(20, 3); // outside the band

    // Start above and left of the clips, in empty lane space, and drag past
    // the second one.
    let y = h.video_lane_y();
    h.press(Pos2::new(h.x_of(secs(1)) - 4.0, y - 20.0));
    h.drag_to(Pos2::new(h.x_of(secs(11)), y + 20.0));
    h.release(Pos2::new(h.x_of(secs(11)), y + 20.0));

    assert!(
        h.state.selected_clips.contains(&first),
        "missed the first clip"
    );
    assert!(
        h.state.selected_clips.contains(&second),
        "missed the second clip"
    );
    assert_eq!(
        h.state.selected_clips.len(),
        2,
        "selected a clip the band did not cover"
    );
}

/// The band must not become a scrub. Before this existed, any drag on empty
/// canvas moved the playhead.
#[test]
fn dragging_a_band_does_not_move_the_playhead() {
    let mut h = Harness::new();
    h.add_clip(2, 3);
    h.editor.set_playhead(TimelineTime::from_seconds(1));
    let before = h.editor.playhead().ticks();

    let y = h.video_lane_y();
    h.press(Pos2::new(h.x_of(secs(6)), y - 20.0));
    h.drag_to(Pos2::new(h.x_of(secs(14)), y + 20.0));
    h.release(Pos2::new(h.x_of(secs(14)), y + 20.0));

    assert_eq!(
        h.editor.playhead().ticks(),
        before,
        "the rubber band scrubbed the playhead"
    );
}

/// A press that never moves is still a click: it must move the playhead and
/// clear the selection, exactly as before.
#[test]
fn a_click_on_empty_canvas_still_moves_the_playhead() {
    let mut h = Harness::new();
    let clip = h.add_clip(2, 3);
    h.click(Pos2::new(h.x_of(secs(3)), h.video_lane_y()));
    assert!(
        h.state.selected_clips.contains(&clip),
        "setup: clip selected"
    );

    let target = secs(16);
    h.click(Pos2::new(h.x_of(target), h.video_lane_y()));

    assert!(
        h.state.selected_clips.is_empty(),
        "clicking empty canvas did not clear the selection"
    );
    let landed = h.editor.playhead().ticks();
    assert!(
        (landed - target).abs() <= h.tpp() * 2,
        "clicking empty canvas at {target} left the playhead at {landed}"
    );
}

/// Dragging the ruler must still scrub rather than starting a band.
#[test]
fn dragging_the_ruler_still_scrubs_not_selects() {
    let mut h = Harness::new();
    h.add_clip(2, 3);

    h.press(Pos2::new(h.x_of(secs(4)), RULER_H / 2.0));
    let target = secs(12);
    h.drag_to(Pos2::new(h.x_of(target), RULER_H / 2.0));
    h.release(Pos2::new(h.x_of(target), RULER_H / 2.0));

    assert!(
        h.state.selected_clips.is_empty(),
        "the ruler started a rubber band"
    );
    let landed = h.editor.playhead().ticks();
    assert!(
        (landed - target).abs() <= h.tpp() * 2,
        "ruler did not scrub"
    );
}

/// Dragging from *on* a clip moves it — the band must not steal that gesture.
#[test]
fn dragging_from_a_clip_still_moves_it() {
    let mut h = Harness::new();
    let clip = h.add_clip(2, 4);
    let start_before = h.clip_start(clip);

    let y = h.video_lane_y();
    h.press(Pos2::new(h.x_of(secs(4)), y));
    h.drag_to(Pos2::new(h.x_of(secs(8)), y));
    h.release(Pos2::new(h.x_of(secs(8)), y));

    assert_ne!(
        h.clip_start(clip),
        start_before,
        "dragging a clip started a rubber band instead of moving it"
    );
}

// ---- right-click menu ----------------------------------------------------

/// The target has to be captured at click time. The pointer moves onto the menu
/// as soon as it opens, so hit-testing when an item is chosen would act on
/// whatever is under the cursor then — which is the menu itself.
#[test]
fn right_clicking_a_clip_targets_that_clip() {
    let mut h = Harness::new();
    let clip = h.add_clip(2, 4);

    h.right_click(Pos2::new(h.x_of(secs(3)), h.video_lane_y()));

    let track = h.video_track();
    assert_eq!(
        h.state.context,
        Some(bettercut_ui::state::ContextTarget::Clip { clip, track })
    );
}

/// Right-clicking an unselected clip selects it, or "Delete" in the menu would
/// silently act on some clip the user selected earlier and forgot about.
#[test]
fn right_clicking_an_unselected_clip_selects_it() {
    let mut h = Harness::new();
    let first = h.add_clip(2, 4);
    let second = h.add_clip(10, 4);

    h.click(Pos2::new(h.x_of(secs(3)), h.video_lane_y()));
    assert!(h.state.selected_clips.contains(&first));

    h.right_click(Pos2::new(h.x_of(secs(11)), h.video_lane_y()));

    assert!(
        h.state.selected_clips.contains(&second),
        "right-click did not move the selection to the clip under the cursor"
    );
    assert!(
        !h.state.selected_clips.contains(&first),
        "the previous selection survived a right-click elsewhere"
    );
}

/// But a clip already inside a multi-selection must not collapse it — "Delete"
/// is expected to take all of them.
#[test]
fn right_clicking_within_a_multi_selection_keeps_it() {
    let mut h = Harness::new();
    let first = h.add_clip(2, 4);
    let second = h.add_clip(10, 4);

    // Set up the selection directly: this test is about what right-click does
    // to an existing multi-selection, not about how one gets made.
    h.state.selected_clips.insert(first);
    h.state.selected_clips.insert(second);

    h.right_click(Pos2::new(h.x_of(secs(11)), h.video_lane_y()));

    assert_eq!(
        h.state.selected_clips.len(),
        2,
        "right-clicking inside a multi-selection collapsed it to one clip"
    );
    assert!(h.state.selected_clips.contains(&first));
    assert!(h.state.selected_clips.contains(&second));
}

/// The menu has to actually open, not just record a target.
#[test]
fn right_clicking_opens_the_menu() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    assert!(
        !egui::Popup::is_any_open(&h.ctx),
        "a menu was open before anything was clicked"
    );

    h.right_click(Pos2::new(h.x_of(secs(3)), h.video_lane_y()));
    h.frame(vec![]);

    assert!(
        egui::Popup::is_any_open(&h.ctx),
        "right-clicking a clip did not open the context menu"
    );
}

/// And a plain left-click closes it again.
#[test]
fn clicking_elsewhere_closes_the_menu() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    h.right_click(Pos2::new(h.x_of(secs(3)), h.video_lane_y()));
    h.frame(vec![]);
    assert!(egui::Popup::is_any_open(&h.ctx), "setup: menu open");

    h.click(Pos2::new(h.x_of(secs(25)), h.video_lane_y()));
    h.frame(vec![]);

    assert!(
        !egui::Popup::is_any_open(&h.ctx),
        "the menu stayed open after a click elsewhere"
    );
}

/// Ctrl+click is the only way to select more than one clip (§10 "Multi-select
/// clips"), and it had no test at all.
#[test]
fn ctrl_click_adds_to_the_selection() {
    let mut h = Harness::new();
    let first = h.add_clip(2, 4);
    let second = h.add_clip(10, 4);

    h.click(Pos2::new(h.x_of(secs(3)), h.video_lane_y()));
    assert_eq!(
        h.state.selected_clips.len(),
        1,
        "the plain click selects one"
    );

    h.ctrl_click(Pos2::new(h.x_of(secs(11)), h.video_lane_y()));

    assert!(
        h.state.selected_clips.contains(&first) && h.state.selected_clips.contains(&second),
        "Ctrl+click did not extend the selection; got {:?}",
        h.state.selected_clips.len()
    );
}

#[test]
fn right_clicking_a_track_header_targets_the_track() {
    let mut h = Harness::new();
    h.add_clip(2, 4);
    let track = h.video_track();

    // Left of the lanes, vertically inside the first video lane.
    h.right_click(Pos2::new(HEADER_W / 2.0, h.video_lane_y()));

    assert_eq!(
        h.state.context,
        Some(bettercut_ui::state::ContextTarget::TrackHeader { track })
    );
    assert_eq!(h.state.selected_track, Some(track));
}

/// Empty canvas carries the instant that was clicked, so "Move Playhead Here"
/// has somewhere to go.
#[test]
fn right_clicking_empty_canvas_records_the_instant() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    let target = secs(20);
    h.right_click(Pos2::new(h.x_of(target), h.video_lane_y()));

    match h.state.context {
        Some(bettercut_ui::state::ContextTarget::Empty { at }) => {
            assert!(
                (at.ticks() - target).abs() <= h.tpp() * 2,
                "recorded {} for a click at {target}",
                at.ticks()
            );
        }
        other => panic!("expected an empty-canvas target, got {other:?}"),
    }
}

/// A right-click must not move the playhead — that is what left-click does, and
/// having both do it would make the menu impossible to open without an edit.
#[test]
fn right_clicking_does_not_move_the_playhead() {
    let mut h = Harness::new();
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::from_seconds(3));
    let before = h.editor.playhead().ticks();

    h.right_click(Pos2::new(h.x_of(secs(20)), h.video_lane_y()));

    assert_eq!(h.editor.playhead().ticks(), before);
}

#[test]
fn clicking_a_clip_selects_it() {
    let mut h = Harness::new();
    let clip = h.add_clip(0, 10);

    let y = h.video_lane_y();
    h.click(Pos2::new(h.x_of(secs(5)), y));

    assert!(
        h.state.selected_clips.contains(&clip),
        "clicking a clip did not select it"
    );
}

#[test]
fn clicking_empty_canvas_deselects_and_moves_the_playhead() {
    let mut h = Harness::new();
    let clip = h.add_clip(0, 5);
    h.state.select_only(clip);

    let y = h.video_lane_y();
    h.click(Pos2::new(h.x_of(secs(20)), y));

    assert!(h.state.selected_clips.is_empty(), "selection survived");
    assert!(
        h.editor.playhead().ticks() > secs(15),
        "playhead did not follow the click"
    );
}

/// The headline interaction: drag a clip and it moves, in exactly one command.
#[test]
fn dragging_a_clip_moves_it_in_one_undo_step() {
    let mut h = Harness::new();
    let clip = h.add_clip(0, 10);
    let undo_before = h.editor.undo_label();

    let y = h.video_lane_y();
    let grab = h.x_of(secs(5));
    h.press(Pos2::new(grab, y));
    h.drag_to(Pos2::new(grab + 200.0, y));
    h.release(Pos2::new(grab + 200.0, y));

    let moved = h.clip_start(clip);
    assert!(moved > 0, "clip did not move (start still {moved})");
    assert!(h.state.drag.is_none(), "drag state survived the release");

    // One command for the whole gesture, not one per frame.
    assert_eq!(h.editor.undo_label().as_deref(), Some("Move Clip"));
    h.editor.undo().expect("undo");
    assert_eq!(
        h.clip_start(clip),
        0,
        "one undo did not fully revert the drag"
    );
    assert_eq!(h.editor.undo_label(), undo_before);
}

#[test]
fn dragging_from_the_right_edge_trims_instead_of_moving() {
    let mut h = Harness::new();
    let clip = h.add_clip(0, 10);
    let original_end = h.clip_end(clip);

    let y = h.video_lane_y();
    // Just inside the right edge: the trim handle is 7 px wide.
    let edge_x = h.x_of(original_end) - 3.0;
    h.press(Pos2::new(edge_x, y));
    h.drag_to(Pos2::new(edge_x - 150.0, y));
    h.release(Pos2::new(edge_x - 150.0, y));

    assert_eq!(h.clip_start(clip), 0, "trimming the end moved the start");
    assert!(
        h.clip_end(clip) < original_end,
        "right edge did not trim (end still {})",
        h.clip_end(clip)
    );
    assert_eq!(h.editor.undo_label().as_deref(), Some("Trim Clip End"));
}

#[test]
fn dragging_from_the_left_edge_trims_the_start() {
    let mut h = Harness::new();
    let clip = h.add_clip(10, 10);
    let original_end = h.clip_end(clip);

    let y = h.video_lane_y();
    let edge_x = h.x_of(secs(10)) + 3.0;
    h.press(Pos2::new(edge_x, y));
    h.drag_to(Pos2::new(edge_x + 150.0, y));
    h.release(Pos2::new(edge_x + 150.0, y));

    assert!(
        h.clip_start(clip) > secs(10),
        "left edge did not trim inward (start {})",
        h.clip_start(clip)
    );
    assert_eq!(
        h.clip_end(clip),
        original_end,
        "trimming the start moved the end"
    );
    assert_eq!(h.editor.undo_label().as_deref(), Some("Trim Clip Start"));
}

/// §10 "Snapping": a clip dragged near another clip's edge lands exactly on it.
#[test]
fn a_dragged_clip_snaps_to_a_neighbouring_edge() {
    let mut h = Harness::new();
    h.add_clip(0, 10); // occupies 0..10 s
    let moving = h.add_clip(25, 5);

    assert!(h.state.snapping, "snapping should default to on");

    let y = h.video_lane_y();
    let grab_offset = 20.0;
    let grab = h.x_of(secs(25)) + grab_offset;
    h.press(Pos2::new(grab, y));

    // Aim a few pixels past the anchor's end, inside the 8 px snap radius.
    let target = h.x_of(secs(10)) + grab_offset + 4.0;
    h.drag_to(Pos2::new(target, y));
    h.release(Pos2::new(target, y));

    assert_eq!(
        h.clip_start(moving),
        secs(10),
        "clip did not snap flush against its neighbour"
    );
}

#[test]
fn snapping_can_be_turned_off() {
    let mut h = Harness::new();
    h.add_clip(0, 10);
    let moving = h.add_clip(25, 5);
    h.state.snapping = false;

    let y = h.video_lane_y();
    let grab_offset = 20.0;
    let grab = h.x_of(secs(25)) + grab_offset;
    h.press(Pos2::new(grab, y));

    let offset_pixels = 4.0;
    let target = h.x_of(secs(10)) + grab_offset + offset_pixels;
    h.drag_to(Pos2::new(target, y));
    h.release(Pos2::new(target, y));

    let landed = h.clip_start(moving);
    assert_ne!(
        landed,
        secs(10),
        "clip snapped even though snapping was off"
    );

    // Near the anchor's end, just not exactly on it.
    let expected = secs(10) + (offset_pixels as i64) * h.tpp();
    assert!(
        (landed - expected).abs() <= h.tpp() * 2,
        "landed at {landed}, expected about {expected}"
    );
}

/// A drag that ends where it started must not create an undo entry.
#[test]
fn a_drag_that_changes_nothing_creates_no_history() {
    let mut h = Harness::new();
    h.add_clip(0, 10);
    let before = h.editor.undo_label();

    let y = h.video_lane_y();
    let x = h.x_of(secs(5));
    h.press(Pos2::new(x, y));
    h.drag_to(Pos2::new(x, y));
    h.release(Pos2::new(x, y));

    assert_eq!(
        h.editor.undo_label(),
        before,
        "a no-op drag pushed an undo entry"
    );
}

/// A drag onto an occupied span is refused, and the clip stays put.
#[test]
fn a_drag_onto_another_clip_is_refused_without_losing_the_clip() {
    let mut h = Harness::new();
    h.add_clip(0, 10);
    let moving = h.add_clip(25, 5);
    h.state.snapping = false;

    let y = h.video_lane_y();
    let grab_offset = 20.0;
    let grab = h.x_of(secs(25)) + grab_offset;
    h.press(Pos2::new(grab, y));
    // Straight on top of the first clip.
    let target = h.x_of(secs(3)) + grab_offset;
    h.drag_to(Pos2::new(target, y));
    h.release(Pos2::new(target, y));

    assert_eq!(
        h.clip_start(moving),
        secs(25),
        "refused drag moved the clip anyway"
    );
    assert_eq!(h.editor.project().clip_count(), 2, "a clip went missing");
    assert!(
        h.state.status.as_ref().is_some_and(|s| s.is_error),
        "a refused drag should say why"
    );
}

/// Dragging the ruler scrubs rather than editing.
#[test]
fn dragging_the_ruler_moves_the_playhead() {
    let mut h = Harness::new();
    h.add_clip(0, 10);

    let x = h.x_of(secs(6));
    h.press(Pos2::new(x, RULER_H / 2.0));
    h.drag_to(Pos2::new(x, RULER_H / 2.0));
    h.release(Pos2::new(x, RULER_H / 2.0));

    assert!(h.editor.playhead().ticks() > 0, "ruler drag did not scrub");
    assert_eq!(
        h.editor.project().clip_count(),
        1,
        "ruler drag disturbed the timeline"
    );
}

/// The playhead always lands on a frame boundary, however ragged the pixel is.
#[test]
fn scrubbing_lands_on_a_frame_boundary() {
    let mut h = Harness::new();
    h.add_clip(0, 20);

    for offset in [1.0, 7.0, 13.0, 29.0] {
        let x = h.x_of(secs(6)) + offset;
        h.click(Pos2::new(x, RULER_H / 2.0));

        // The default sequence is 30 fps: 32,000 ticks per frame.
        assert_eq!(
            h.editor.playhead().ticks() % 32_000,
            0,
            "playhead off the frame grid at offset {offset}"
        );
    }
}

/// Dragging a clip must not leave the playhead scrubbing along with it.
#[test]
fn dragging_a_clip_does_not_move_the_playhead() {
    let mut h = Harness::new();
    h.add_clip(0, 10);
    h.editor.set_playhead(TimelineTime::from_seconds(2));
    let before = h.editor.playhead();

    let y = h.video_lane_y();
    let grab = h.x_of(secs(5));
    h.press(Pos2::new(grab, y));
    h.drag_to(Pos2::new(grab + 120.0, y));
    h.release(Pos2::new(grab + 120.0, y));

    assert_eq!(
        h.editor.playhead(),
        before,
        "dragging a clip scrubbed the playhead"
    );
}
