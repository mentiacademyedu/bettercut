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
    /// Where [`Harness::drawn_text`] collects, while it is collecting.
    words: Option<std::rc::Rc<std::cell::RefCell<String>>>,
}

impl Harness {
    fn new() -> Self {
        let (editor, _events) = Editor::new_project("Interaction");
        Self {
            ctx: egui::Context::default(),
            editor,
            state: UiState::default(),
            top_offset: 0.0,
            words: None,
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

    /// Draw one frame and hand back every word it put on screen.
    ///
    /// Most tests assert on the editor's state rather than on pixels; this is
    /// for the few marks that exist only as drawing — a badge has no other
    /// evidence that it was shown.
    fn drawn_text(&mut self) -> String {
        let text = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        self.words = Some(std::rc::Rc::clone(&text));
        self.frame(vec![]);
        self.words = None;
        let borrowed = text.borrow();
        borrowed.clone()
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

        if let Some(words) = &self.words {
            let mut words = words.borrow_mut();
            for clipped in &output.shapes {
                collect_text(&clipped.shape, &mut words);
            }
        }
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

    /// Vertical centre of the first video lane.
    ///
    /// §26's text lanes are drawn above the video ones — they composite over
    /// everything, and the lane order on screen is the compositing order upside
    /// down — so the video lane sits one lane lower for each of them.
    fn video_lane_y(&self) -> f32 {
        // Titles, then adjustments, then video: the compositing order upside
        // down. Counting only the titles put every video press on the
        // adjustment lane as soon as one existed.
        let above =
            self.editor
                .active_sequence()
                .map_or(0, |s| s.text_tracks.len() + s.adjustment_tracks.len()) as f32;
        RULER_H + 2.0 + above * (TRACK_H + 2.0) + TRACK_H / 2.0
    }

    /// Vertical centre of the first adjustment lane: directly beneath the
    /// title lanes, above the video.
    fn adjustment_lane_y(&self) -> f32 {
        let text_lanes = self
            .editor
            .active_sequence()
            .map_or(0, |s| s.text_tracks.len()) as f32;
        RULER_H + 2.0 + text_lanes * (TRACK_H + 2.0) + TRACK_H / 2.0
    }

    /// An adjustment stretched to six seconds, so its body is wide enough to
    /// press rather than a trim handle.
    fn add_wide_adjustment(&mut self) -> ClipId {
        let clip = self.editor.add_adjustment().unwrap();
        let track = self.editor.active_sequence().unwrap().adjustment_tracks[0].id;
        self.editor
            .trim_clip(
                track,
                clip,
                bettercut_editor_core::TrimEdge::End,
                TimelineTime::from_seconds(6),
            )
            .unwrap();
        clip
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

/// Dragging the playhead to the very start must work with the mouse.
///
/// It did not. The handler returned early whenever the pointer sat left of the
/// lane area — correct for a click on a track header, wrong for a drag already
/// under way. Tick 0 is at exactly the lane area's left edge, so reaching it
/// meant landing on one specific pixel; a pixel further left and the playhead
/// simply stopped following. The reported symptom was having to use the -10s
/// button to get back to the beginning.
#[test]
fn dragging_off_the_left_edge_reaches_the_start() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    h.press(Pos2::new(h.x_of(secs(10)), RULER_H / 2.0));
    // Well into the track-header column, as an unhurried drag to the start
    // naturally goes.
    h.drag_to(Pos2::new(HEADER_W / 2.0, RULER_H / 2.0));

    assert_eq!(
        h.editor.playhead().ticks(),
        0,
        "dragging past the left edge did not reach the start of the sequence"
    );
}

/// The same, once the view has scrolled — which is the state playback leaves
/// it in, and where the start was not merely fiddly to hit but unreachable:
/// the leftmost visible tick was the scroll position, not zero.
#[test]
fn dragging_left_while_scrolled_still_reaches_the_start() {
    let mut h = Harness::new();
    h.add_clip(2, 4);

    h.state.scroll_ticks = secs(30);
    assert!(h.state.scroll_ticks > 0, "the test needs a scrolled view");

    h.press(Pos2::new(h.x_of(secs(40)), RULER_H / 2.0));
    // Hold at the left edge. Each frame scrolls a little further back, exactly
    // as holding the pointer there does in the app.
    for _ in 0..80 {
        h.drag_to(Pos2::new(HEADER_W / 2.0, RULER_H / 2.0));
    }

    assert_eq!(
        h.editor.playhead().ticks(),
        0,
        "could not drag back to the start from a scrolled view"
    );
    assert_eq!(
        h.state.scroll_ticks, 0,
        "the view did not scroll back with the drag"
    );
}

/// The guard that was relaxed still has to do its job: clicking a track header
/// is not a click on the timeline, and must leave the playhead alone.
#[test]
fn clicking_a_track_header_does_not_move_the_playhead() {
    let mut h = Harness::new();
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::from_seconds(7));
    let before = h.editor.playhead().ticks();

    h.click(Pos2::new(HEADER_W / 2.0, h.video_lane_y()));

    assert_eq!(
        h.editor.playhead().ticks(),
        before,
        "clicking a track header scrubbed the timeline"
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
        Some(bettercut_ui::state::ContextTarget::Empty { at, track }) => {
            // On a lane, so "Close Gap" knows which track to close it on.
            assert!(track.is_some(), "the lane under the click was not recorded");
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

/// A drag that starts *outside* the timeline and wanders into the ruler must
/// not scrub.
///
/// This is the reported bug, and the previous test missed it because it never
/// crossed the boundary: dragging the picture in the preview and letting the
/// cursor stray down over the ruler sent the playhead to whatever second the
/// mouse happened to be over, mid-gesture.
///
/// The ruler path was gated on the button being down *anywhere* rather than
/// down on the timeline, and `pos` falls back to the pointer's position
/// anywhere in the window — so crossing the boundary was enough.
#[test]
fn a_drag_that_began_elsewhere_does_not_scrub_when_it_crosses_the_ruler() {
    let mut h = Harness::with_panels_above(300.0);
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::from_seconds(3));
    let before = h.editor.playhead().ticks();

    // Press well above the timeline, as a preview drag would.
    h.press(Pos2::new(400.0, 120.0));
    // Then wander down across the ruler and along it.
    h.drag_to(Pos2::new(500.0, 200.0));
    h.drag_to(Pos2::new(600.0, 300.0 + RULER_H / 2.0));
    h.drag_to(Pos2::new(900.0, 300.0 + RULER_H / 2.0));
    h.release(Pos2::new(900.0, 300.0 + RULER_H / 2.0));

    assert_eq!(
        h.editor.playhead().ticks(),
        before,
        "a drag that started in the preview scrubbed the timeline on the way past"
    );
}

/// And the ruler still works for a press that really did land on it — the fix
/// must not cost the feature it guards.
#[test]
fn pressing_the_ruler_still_scrubs_after_the_guard() {
    let mut h = Harness::with_panels_above(300.0);
    h.add_clip(2, 4);
    h.editor.set_playhead(TimelineTime::ZERO);

    let x = HEADER_W + 300.0;
    h.press(Pos2::new(x, 300.0 + RULER_H / 2.0));

    assert!(
        h.editor.playhead().ticks() > 0,
        "pressing the ruler no longer scrubs"
    );
}

/// §12: pressing on a linked clip captures its partner, so the drag can show
/// where the sound is going instead of letting it jump on release.
#[test]
fn pressing_a_linked_clip_captures_its_partner_for_the_drag() {
    let mut h = Harness::new();
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/linked.mp4",
        MediaTime::from_seconds(600),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = h.editor.import_media(asset);
    let placed = h.editor.place_media(media).expect("placed");
    assert_eq!(placed.len(), 2, "setup: picture and sound");

    // Somewhere in the body of the picture, away from the trim handles.
    let pos = Pos2::new(h.x_of(secs(30)), h.video_lane_y());
    h.press(pos);

    let drag = h
        .state
        .drag
        .as_ref()
        .expect("the press did not start a drag");
    assert_eq!(drag.clip, placed[0], "the drag grabbed the wrong clip");
    assert_eq!(
        drag.partners.len(),
        1,
        "the sound was not captured, so the drag cannot show it moving"
    );
    h.release(pos);
}

/// An unlinked clip drags alone, with no ghost of anything else.
#[test]
fn pressing_an_unlinked_clip_captures_no_partner() {
    let mut h = Harness::new();
    h.add_clip(0, 10);
    let pos = Pos2::new(h.x_of(secs(5)), h.video_lane_y());
    h.press(pos);

    let drag = h
        .state
        .drag
        .as_ref()
        .expect("the press did not start a drag");
    assert!(drag.partners.is_empty());
    h.release(pos);
}

/// While playing, the view keeps up with the playhead — and only while
/// playing, so a paused editor stays where the user scrolled it.
#[test]
fn the_view_follows_the_playhead_while_playing() {
    use bettercut_ui::state::PlaybackStats;

    let mut h = Harness::new();
    h.add_clip(0, 600);

    // Paused, with the playhead far off to the right: nothing moves.
    h.editor.set_playhead(TimelineTime::from_seconds(120));
    h.frame(vec![]);
    assert_eq!(h.state.scroll_ticks, 0, "a paused view scrolled by itself");

    h.state.playback = Some(PlaybackStats {
        playing: true,
        dropped_frames: 0,
        underruns: 0,
        limited_samples: 0,
        peaks: (0.0, 0.0),
        loudness: (None, None),
        prefetch_hits: 0,
        ring_frames: 0,
        quality: "full",
    });
    h.frame(vec![]);

    let scrolled = h.state.scroll_ticks;
    assert!(scrolled > 0, "the view did not follow the playhead");
    let span = (SCREEN.x - HEADER_W) as i64 * h.state.ticks_per_pixel();
    let playhead = secs(120);
    assert!(
        playhead >= scrolled && playhead < scrolled + span,
        "the playhead is off screen at {playhead}, view {scrolled}..{}",
        scrolled + span
    );
}

/// A held frame says so, and does not draw a filmstrip pretending to move.
#[test]
fn a_held_clip_is_badged() {
    let mut h = Harness::new();
    let clip = h.add_clip(0, 20);
    h.editor.set_playhead(TimelineTime::from_seconds(5));
    let held = h
        .editor
        .freeze_frame(clip, TimelineTime::from_seconds(3))
        .unwrap();

    h.frame(vec![]);

    let words = h.drawn_text();
    assert!(words.contains("hold"), "no hold badge: {words}");
    assert!(
        h.editor.video_clip(held).unwrap().frozen,
        "the clip under the badge is not a hold"
    );
}

/// Every string in a shape tree, for [`Harness::drawn_text`].
fn collect_text(shape: &egui::Shape, into: &mut String) {
    match shape {
        egui::Shape::Text(text) => {
            into.push_str(text.galley.text());
            into.push(' ');
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, into);
            }
        }
        _ => {}
    }
}

/// What a clip says about itself in the lane (§53).
///
/// A mask, a key, a blend, an entrance, an eased key — every one of them looks
/// like an ordinary clip unless the timeline says otherwise, and a user left to
/// wonder why the preview disagrees with the timeline is a user who distrusts
/// both.
///
/// Some of it is drawn as words and some as shapes, so the tests reach for
/// whichever the drawing actually produces: a badge is text, a ramp is a
/// triangle, an eased key is a circle where a plain one is a diamond.
mod clip_marks {
    use super::*;
    use bettercut_editor_core::timeline::{BlendMode, ChromaKey, Mask};

    fn harness_with_clip() -> (Harness, ClipId) {
        let mut h = Harness::new();
        let clip = h.add_clip(0, 20);
        h.state.select_only(clip);
        (h, clip)
    }

    /// An ordinary clip carries none: a badge on everything is noise.
    #[test]
    fn an_ordinary_clip_has_no_badges() {
        let (mut h, _clip) = harness_with_clip();
        let words = h.drawn_text();
        for badge in ["mask", "key", "Screen", "hold"] {
            assert!(
                !words.contains(badge),
                "an untouched clip drew {badge}: {words}"
            );
        }
    }

    #[test]
    fn a_masked_clip_says_so() {
        let (mut h, clip) = harness_with_clip();
        h.editor
            .set_clip_property(
                clip,
                bettercut_editor_core::ClipProperty::Mask(Some(Mask::default())),
                false,
            )
            .unwrap();

        assert!(h.drawn_text().contains("mask"), "no mask badge");
    }

    #[test]
    fn a_keyed_clip_says_so() {
        let (mut h, clip) = harness_with_clip();
        h.editor
            .set_clip_property(
                clip,
                bettercut_editor_core::ClipProperty::ChromaKey(Some(ChromaKey::default())),
                false,
            )
            .unwrap();

        assert!(h.drawn_text().contains("key"), "no key badge");
    }

    /// The blend badge names the mode: "not normal" is not enough to act on.
    #[test]
    fn a_blended_clip_names_its_mode() {
        let (mut h, clip) = harness_with_clip();
        h.editor
            .set_clip_property(
                clip,
                bettercut_editor_core::ClipProperty::Blend(BlendMode::Screen),
                false,
            )
            .unwrap();

        let words = h.drawn_text();
        assert!(words.contains("Screen"), "no blend badge: {words}");
    }

    /// Where each badge was drawn, so overlap can be seen rather than assumed.
    ///
    /// Text alone cannot show it: two badges drawn on top of each other still
    /// put both their labels in the frame, and the user sees one smudge.
    fn badge_positions(h: &mut Harness) -> Vec<(String, egui::Pos2)> {
        let ctx = egui::Context::default();
        let editor = &mut h.editor;
        let state = &mut h.state;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::timeline::draw(ui, editor, state);
        });
        output.textures_delta.clear();

        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push((text.galley.text().to_owned(), text.pos));
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
        }
        found
    }

    /// Every triangle drawn anywhere on the timeline.
    ///
    /// A fade ramp is a three-point polygon and nothing else on a clip is, so
    /// counting them is enough to tell a clip that is arriving from one that
    /// merely says it is.
    fn drawn_triangles(h: &mut Harness) -> usize {
        let Harness {
            ctx, editor, state, ..
        } = h;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::timeline::draw(ui, editor, state);
        });
        output.textures_delta.clear();

        fn walk(shape: &egui::Shape, count: &mut usize) {
            match shape {
                egui::Shape::Path(path) if path.points.len() == 3 => *count += 1,
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, count)),
                _ => {}
            }
        }
        let mut found = 0;
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
        }
        found
    }

    /// Every circle drawn on the timeline.
    ///
    /// §24's eased keys are drawn as circles where a plain ramp is a diamond,
    /// so counting them says whether the curve reached the screen.
    fn drawn_circles(h: &mut Harness) -> usize {
        let Harness {
            ctx, editor, state, ..
        } = h;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::timeline::draw(ui, editor, state);
        });
        output.textures_delta.clear();

        fn walk(shape: &egui::Shape, count: &mut usize) {
            match shape {
                egui::Shape::Circle(_) => *count += 1,
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, count)),
                _ => {}
            }
        }
        let mut found = 0;
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
        }
        found
    }

    /// §24: a key that eases is drawn differently from one that ramps. Without
    /// that the easing is invisible — the user sets it, the row of diamonds
    /// does not move, and there is no way to tell which keys carry it.
    #[test]
    fn an_eased_key_is_drawn_as_a_different_shape() {
        use bettercut_editor_core::timeline::{AnimatedParameter, Interpolation, Keyframe};

        let (mut h, clip) = harness_with_clip();
        let at = bettercut_editor_core::foundation::MediaTime::from_seconds(61);

        h.editor
            .set_keyframe(
                clip,
                AnimatedParameter::Opacity,
                Keyframe::new(at, 0.5, Interpolation::Linear),
            )
            .unwrap();
        let ramped = drawn_circles(&mut h);

        h.editor
            .set_keyframe(
                clip,
                AnimatedParameter::Opacity,
                Keyframe::new(at, 0.5, Interpolation::EaseInOut),
            )
            .unwrap();

        assert_eq!(
            drawn_circles(&mut h),
            ramped + 1,
            "easing a key did not change how it is drawn"
        );
    }

    /// Where a lane sits, counted the way the draw pass lays them out.
    ///
    /// Titles first — they composite over every video track, and the lane
    /// order on screen is the compositing order upside down — so the video
    /// lane of a default sequence is the second one, not the first.
    fn lane_rect(index: usize) -> Rect {
        let step = bettercut_ui::theme::TRACK_HEIGHT + bettercut_ui::theme::TRACK_GAP;
        let top = bettercut_ui::theme::RULER_HEIGHT
            + bettercut_ui::theme::TRACK_GAP
            + step * index as f32;
        Rect::from_min_size(
            Pos2::new(0.0, top),
            egui::vec2(
                bettercut_ui::theme::TRACK_HEADER_WIDTH,
                bettercut_ui::theme::TRACK_HEIGHT,
            ),
        )
    }

    /// The two buttons the mixer needs most, reachable without a right-click.
    ///
    /// Clicked through the real interaction path rather than by calling the
    /// handler, because the thing worth proving is that a press at the
    /// button's *drawn* position reaches it.
    #[test]
    fn the_header_buttons_mute_and_solo_the_track() {
        use bettercut_editor_core::TrackFlag;

        let mut h = Harness::new();
        let _clip = h.add_clip(0, 20);
        let track = h.video_track();
        let [_target, mute, solo] = bettercut_ui::timeline::header_buttons(lane_rect(1));

        assert!(h.editor.track_flag(track, TrackFlag::Enabled));
        h.click(mute.center());
        assert!(
            !h.editor.track_flag(track, TrackFlag::Enabled),
            "pressing M did not mute the track"
        );

        assert!(!h.editor.track_flag(track, TrackFlag::Solo));
        h.click(solo.center());
        assert!(
            h.editor.track_flag(track, TrackFlag::Solo),
            "pressing S did not solo the track"
        );
    }

    /// And the rest of the header is still not a control: a press beside the
    /// buttons must change nothing, which is what the column's early return
    /// has always been for.
    #[test]
    fn a_press_elsewhere_on_the_header_still_changes_nothing() {
        use bettercut_editor_core::TrackFlag;

        let mut h = Harness::new();
        let _clip = h.add_clip(0, 20);
        let track = h.video_track();

        let lane = lane_rect(1);
        h.click(Pos2::new(lane.left() + 8.0, lane.center().y));

        assert!(h.editor.track_flag(track, TrackFlag::Enabled));
        assert!(!h.editor.track_flag(track, TrackFlag::Solo));
    }

    /// §20a.4: a solo left on is why every other lane has gone quiet. If the
    /// header does not say so, the only clue is a menu the user has no reason
    /// to open — so this is not decoration, it is the explanation.
    #[test]
    fn a_soloed_track_says_so_on_its_header() {
        let mut h = Harness::new();
        let _clip = h.add_clip(0, 20);
        assert!(
            !h.drawn_text().contains("SOLO"),
            "an ordinary track claimed to be soloed"
        );

        let track = h.video_track();
        h.editor
            .set_track_flag(track, bettercut_editor_core::TrackFlag::Solo, true)
            .unwrap();

        assert!(
            h.drawn_text().contains("SOLO"),
            "a soloed track gave no sign of it: {}",
            h.drawn_text()
        );
    }

    /// A clip that is arriving should *look* like it, the same way a
    /// sound's fades and a title's entrance do. The badge says an animation is
    /// there; the ramp says where it starts and how long it takes.
    #[test]
    fn an_animated_clip_draws_its_entrance_and_exit() {
        use bettercut_editor_core::timeline::{ClipMotion, Motion, MotionKind};

        let (mut h, clip) = harness_with_clip();
        let plain = drawn_triangles(&mut h);

        h.editor
            .set_clip_property(
                clip,
                bettercut_editor_core::ClipProperty::Motion(ClipMotion {
                    intro: Some(Motion::new(
                        MotionKind::Fade,
                        bettercut_editor_core::foundation::TimelineTime::from_seconds(1),
                    )),
                    outro: Some(Motion::new(
                        MotionKind::Fade,
                        bettercut_editor_core::foundation::TimelineTime::from_seconds(1),
                    )),
                }),
                false,
            )
            .unwrap();

        assert_eq!(
            drawn_triangles(&mut h),
            plain + 2,
            "an entrance and an exit should have drawn a ramp each"
        );
    }

    /// All three at once, side by side. The old layout drew every badge at one
    /// anchor, so a second would have sat on top of the first.
    #[test]
    fn three_effects_all_fit() {
        let (mut h, clip) = harness_with_clip();
        for property in [
            bettercut_editor_core::ClipProperty::Mask(Some(Mask::default())),
            bettercut_editor_core::ClipProperty::ChromaKey(Some(ChromaKey::default())),
            bettercut_editor_core::ClipProperty::Blend(BlendMode::Multiply),
        ] {
            h.editor.set_clip_property(clip, property, false).unwrap();
        }

        let words = h.drawn_text();
        for badge in ["mask", "key", "Multiply"] {
            assert!(words.contains(badge), "{badge} was crowded out: {words}");
        }

        // And each is somewhere of its own.
        let drawn = badge_positions(&mut h);
        let mut places: Vec<f32> = ["mask", "key", "Multiply"]
            .into_iter()
            .map(|badge| {
                drawn
                    .iter()
                    .find(|(text, _)| text == badge)
                    .unwrap_or_else(|| panic!("{badge} was not drawn"))
                    .1
                    .x
            })
            .collect();
        places.sort_by(f32::total_cmp);
        for pair in places.windows(2) {
            assert!(
                pair[1] - pair[0] > 12.0,
                "two badges were drawn on top of each other at {:?}",
                places
            );
        }
    }
}

// ---- adjustment clips -------------------------------------------------------

/// An adjustment is a clip on its own lane, so it is selected and moved on the
/// timeline like any other. Each of those depends on a lookup that has to know
/// about adjustment lanes, which the compiler does not check.
#[test]
fn clicking_an_adjustment_selects_it() {
    let mut h = Harness::new();
    let clip = h.add_wide_adjustment();

    let y = h.adjustment_lane_y();
    h.click(Pos2::new(h.x_of(secs(3)), y));

    assert!(
        h.state.selected_clips.contains(&clip),
        "clicking the adjustment did not select it"
    );
}

#[test]
fn dragging_an_adjustment_moves_it() {
    let mut h = Harness::new();
    let clip = h.add_wide_adjustment();
    let before = h.editor.adjustment_clip(clip).unwrap().timeline.start;

    let y = h.adjustment_lane_y();
    h.press(Pos2::new(h.x_of(secs(3)), y));
    h.drag_to(Pos2::new(h.x_of(secs(9)), y));
    h.release(Pos2::new(h.x_of(secs(9)), y));

    assert_ne!(
        h.editor.adjustment_clip(clip).unwrap().timeline.start,
        before,
        "dragging the adjustment did not move it"
    );
}

/// With an adjustment lane in place the video lane has moved down a row, and a
/// drag aimed at a video clip must still land on it.
#[test]
fn a_video_clip_still_drags_beneath_an_adjustment_lane() {
    let mut h = Harness::new();
    h.editor.add_adjustment().unwrap();
    let clip = h.add_clip(2, 4);
    let before = h.clip_start(clip);

    let y = h.video_lane_y();
    h.press(Pos2::new(h.x_of(secs(4)), y));
    h.drag_to(Pos2::new(h.x_of(secs(8)), y));
    h.release(Pos2::new(h.x_of(secs(8)), y));

    assert_ne!(h.clip_start(clip), before, "the video clip did not move");
}

/// Clicking one clip of a group selects the whole group, and grouped clips
/// say so with a badge.
#[test]
fn clicking_a_grouped_clip_selects_its_group() {
    let mut h = Harness::new();
    let first = h.add_clip(0, 4);
    let second = h.add_clip(6, 4);
    let loose = h.add_clip(12, 4);
    h.editor.group_clips(&[first, second]).unwrap();

    let y = h.video_lane_y();
    h.click(Pos2::new(h.x_of(secs(8)), y));
    h.frame(vec![]);
    assert!(
        h.state.selected_clips.contains(&first),
        "the rest of the group was not selected"
    );
    assert!(h.state.selected_clips.contains(&second));
    assert!(!h.state.selected_clips.contains(&loose));
    assert!(
        h.drawn_text().contains("group"),
        "grouped clips are not marked"
    );
}

/// Every lane height is laid out and hit-tested alike: a click in the middle
/// of the video lane at that height selects the clip there, and the lanes the
/// canvas reports are that tall.
#[test]
fn every_lane_height_lays_out_and_clicks_the_same() {
    use bettercut_ui::state::LaneHeight;

    for height in LaneHeight::ALL {
        let mut h = Harness::new();
        h.state.lane_height = height;
        let clip = h.add_clip(0, 10);
        let above =
            h.editor
                .active_sequence()
                .map_or(0, |s| s.text_tracks.len() + s.adjustment_tracks.len()) as f32;
        let lane = height.pixels();
        let y = RULER_H + 2.0 + above * (lane + 2.0) + lane / 2.0;

        h.click(Pos2::new(h.x_of(secs(5)), y));
        assert!(
            h.state.selected_clips.contains(&clip),
            "{height:?}: a click in the middle of the video lane missed the clip"
        );

        // Half a lane lower is the gap-and-next-lane side: not this clip.
        h.state.clear_selection();
        h.click(Pos2::new(h.x_of(secs(5)), y + lane / 2.0 + 6.0));
        assert!(
            !h.state.selected_clips.contains(&clip),
            "{height:?}: the clip answered a click below its lane"
        );
    }
    assert!(LaneHeight::Compact.pixels() < LaneHeight::Normal.pixels());
    assert!(LaneHeight::Normal.pixels() < LaneHeight::Tall.pixels());
}

/// A title lane's header has a menu too — hide, lock, rename, duplicate —
/// where it used to open empty.
#[test]
fn a_title_lane_header_has_a_menu() {
    let mut h = Harness::new();
    // The title lane is the first lane under the ruler.
    let y = RULER_H + 2.0 + TRACK_H / 2.0;
    h.right_click(Pos2::new(HEADER_W / 2.0, y));
    let words = h.drawn_text();
    assert!(words.contains("Duplicate Track"), "{words}");
    assert!(words.contains("Hide Track"), "{words}");
}

/// A name typed into the track menu is applied when the menu closes, as one
/// undo step.
#[test]
fn a_track_name_typed_in_the_menu_is_kept_when_it_closes() {
    let mut h = Harness::new();
    h.add_clip(2, 4);
    let track = h.video_track();
    h.right_click(Pos2::new(HEADER_W / 2.0, h.video_lane_y()));
    h.frame(vec![]);
    assert!(egui::Popup::is_any_open(&h.ctx), "setup: menu open");

    // What typing into the field leaves behind.
    h.state.track_name_draft = Some((track, "  Interview  ".to_owned()));
    h.click(Pos2::new(h.x_of(secs(25)), h.video_lane_y()));
    h.frame(vec![]);

    let name = h
        .editor
        .active_sequence()
        .unwrap()
        .track_name(track)
        .unwrap()
        .to_owned();
    assert_eq!(name, "Interview");
    assert!(h.state.track_name_draft.is_none());
    assert_eq!(h.editor.undo_label().as_deref(), Some("Rename Track"));
}

/// A clip with a note says so on the timeline.
#[test]
fn a_clip_with_a_note_is_badged() {
    let mut h = Harness::new();
    let clip = h.add_clip(0, 10);
    assert!(!h.drawn_text().contains("note"));
    h.editor.set_clip_note(clip, "swap for take 3").unwrap();
    assert!(h.drawn_text().contains("note"), "the note is not shown");
}
