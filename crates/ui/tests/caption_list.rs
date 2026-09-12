//! The Captions window (§27's caption editing, §28's typing).
//!
//! The window is a view onto the caption lane, so the things worth holding are
//! that it shows the lane in order, that it knows which caption the playhead is
//! in, and that editing a row goes through the editor rather than into a copy
//! that can drift.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::captions::CaptionSegment;
use bettercut_editor_core::command::TextProperty;
use bettercut_editor_core::foundation::{ClipId, TimelineTime};
use bettercut_ui::UiState;
use bettercut_ui::caption_list::{row_at, rows, still_typing};
use egui::{Pos2, RawInput, Rect, vec2};

fn ms(v: i64) -> TimelineTime {
    TimelineTime::from_millis(v)
}

/// A project with three captions on the lane, the middle one still empty.
fn setup() -> (Editor, UiState) {
    let (mut editor, _events) = Editor::new_project("Captions");
    editor
        .replace_captions(
            vec![
                CaptionSegment::new(ms(0), ms(2_000), "the first thing said"),
                CaptionSegment::new(ms(2_500), ms(4_000), ""),
                CaptionSegment::new(ms(5_000), ms(6_500), "and the last"),
            ],
            "Import",
        )
        .unwrap();
    let mut state = UiState::default();
    state.captions_open = true;
    (editor, state)
}

/// Draw the window and return the words it drew.
fn frame(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::caption_list::show(ui.ctx(), editor, state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push(' ');
            }
        }
    }
    words
}

#[test]
fn the_lane_is_listed_in_order() {
    let (editor, _state) = setup();
    let listed = rows(&editor);

    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].text, "the first thing said");
    assert_eq!(
        listed[1].text, "",
        "an empty caption was dropped from the list"
    );
    assert_eq!(listed[2].start, ms(5_000));
    assert!(
        listed.windows(2).all(|pair| pair[0].start <= pair[1].start),
        "the list is out of order"
    );
}

/// Titles are not captions. They are placed, styled and animated one at a time,
/// and mixing them into this list would put two different jobs in one window.
#[test]
fn titles_are_not_in_the_caption_list() {
    let (mut editor, _state) = setup();
    editor.set_playhead(ms(8_000));
    editor.add_text("A TITLE").unwrap();

    let listed = rows(&editor);
    assert_eq!(listed.len(), 3, "a title appeared in the caption list");
    assert!(listed.iter().all(|row| row.text != "A TITLE"));
}

#[test]
fn the_playhead_picks_out_the_caption_it_is_in() {
    let (editor, _state) = setup();
    let listed = rows(&editor);

    assert_eq!(row_at(&listed, ms(500)), Some(0));
    assert_eq!(row_at(&listed, ms(3_000)), Some(1));
    assert_eq!(row_at(&listed, ms(6_400)), Some(2));
    // The gaps belong to nobody: a highlight clinging to the last caption
    // through ten seconds of silence would say the wrong thing about where the
    // playhead is.
    assert_eq!(row_at(&listed, ms(2_200)), None);
    assert_eq!(row_at(&listed, ms(9_000)), None);
    // Half-open, like every other range (§8): the instant a caption ends is
    // the next one's, never both.
    assert_eq!(row_at(&listed, ms(2_000)), None);
}

#[test]
fn the_window_shows_every_caption_and_a_hint_for_the_empty_one() {
    let (mut editor, mut state) = setup();
    let words = frame(&mut editor, &mut state);

    assert!(words.contains("the first thing said"), "{words}");
    assert!(words.contains("and the last"), "{words}");
    assert!(
        words.contains("type what is said here"),
        "an empty caption drew as a blank box with no invitation: {words}"
    );
    assert!(words.contains("3 captions"), "{words}");
}

#[test]
fn an_empty_lane_says_how_to_fill_it() {
    let (mut editor, _events) = Editor::new_project("Empty");
    let mut state = UiState::default();
    state.captions_open = true;

    let words = frame(&mut editor, &mut state);
    assert!(words.contains("No captions yet"), "{words}");
    assert!(words.contains("Time Captions"), "{words}");
}

/// The list is a view, not a copy: typing goes through the editor, so it is
/// undoable and the timeline sees it immediately.
///
/// `type_into` is what the window does per keystroke — `continuing` false on
/// the first, true after — with [`still_typing`] making that decision.
fn type_into(editor: &mut Editor, state: &mut UiState, clip: ClipId, text: &str) {
    let continuing = still_typing(state.caption_typing, clip);
    editor
        .set_text_property(clip, TextProperty::Content(text.to_owned()), continuing)
        .unwrap();
    state.caption_typing = Some(clip);
}

#[test]
fn typing_into_a_row_edits_the_clip() {
    let (mut editor, mut state) = setup();
    let clip = rows(&editor)[1].clip;
    let before = editor.undo_depth();

    for text in ["half", "half a", "half a sentence"] {
        type_into(&mut editor, &mut state, clip, text);
    }

    assert_eq!(rows(&editor)[1].text, "half a sentence");
    assert_eq!(
        editor.undo_depth(),
        before + 1,
        "typing one caption filled the history with a step per letter (§11)"
    );

    editor.undo().unwrap();
    assert_eq!(
        rows(&editor)[1].text,
        "",
        "undo left half the typing behind"
    );
}

/// Every caption edit carries the same history label, so a keystroke marked as
/// continuing is folded into whatever text edit came last. Moving to another
/// caption has to start a new step, or one undo wipes a caption the user was
/// not touching.
#[test]
fn each_caption_is_its_own_undo_step() {
    let (mut editor, mut state) = setup();
    let (first, second) = (rows(&editor)[0].clip, rows(&editor)[1].clip);
    let before = editor.undo_depth();

    type_into(&mut editor, &mut state, first, "rewritten");
    type_into(&mut editor, &mut state, first, "rewritten twice");
    type_into(&mut editor, &mut state, second, "the middle one");

    assert_eq!(editor.undo_depth(), before + 2);

    editor.undo().unwrap();
    assert_eq!(rows(&editor)[1].text, "", "the second caption is back");
    assert_eq!(
        rows(&editor)[0].text,
        "rewritten twice",
        "undoing one caption also undid the caption typed before it"
    );
}

/// A caption with words in it exports; the still-empty one does not.
#[test]
fn a_half_typed_lane_exports_only_what_has_words() {
    let (editor, _state) = setup();
    let segments = editor.caption_segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].text, "the first thing said");
    assert_eq!(segments[1].text, "and the last");
}

/// A look belongs to the lane, not to a caption: subtitles that changed style
/// halfway through read as a mistake.
#[test]
fn a_look_dresses_every_caption_in_one_step() {
    let (mut editor, _state) = setup();
    let before = editor.undo_depth();

    let count = editor
        .set_caption_look(bettercut_editor_core::text::CaptionLook::Highlight)
        .unwrap();

    assert_eq!(count, 3);
    assert_eq!(editor.undo_depth(), before + 1, "one decision, one step");
    let wanted = bettercut_editor_core::text::TextStyle::look(
        bettercut_editor_core::text::CaptionLook::Highlight,
    );
    let sequence = editor.active_sequence().unwrap();
    let lane = sequence
        .text_tracks
        .iter()
        .find(|t| t.name == Editor::CAPTION_TRACK)
        .unwrap();
    assert!(
        lane.clips().iter().all(|clip| clip.style == wanted),
        "a caption kept its old look"
    );
}

/// Titles are dressed one at a time — that is the difference between a title
/// and a caption.
#[test]
fn a_look_leaves_titles_alone() {
    let (mut editor, _state) = setup();
    editor.set_playhead(ms(8_000));
    let title = editor.add_text("A TITLE").unwrap();
    let before = editor
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .find_map(|t| t.get(title))
        .unwrap()
        .style
        .clone();

    editor
        .set_caption_look(bettercut_editor_core::text::CaptionLook::Highlight)
        .unwrap();

    let after = editor
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .find_map(|t| t.get(title))
        .unwrap()
        .style
        .clone();
    assert_eq!(after, before, "the title was restyled with the captions");
}

/// Applying the look the lane already wears is not an edit.
#[test]
fn the_look_it_already_wears_changes_nothing() {
    let (mut editor, _state) = setup();
    let look = bettercut_editor_core::text::CaptionLook::Outlined;
    editor.set_caption_look(look).unwrap();
    let before = editor.undo_depth();

    assert_eq!(editor.set_caption_look(look).unwrap(), 0);
    assert_eq!(
        editor.undo_depth(),
        before,
        "an empty step went into history"
    );
}

/// The window shows which look is on, so the row answers a question rather
/// than offering four equal buttons.
#[test]
fn the_window_knows_which_look_is_on() {
    let (mut editor, _state) = setup();
    let listed = rows(&editor);
    assert_eq!(
        bettercut_ui::caption_list::lane_look(&editor, &listed),
        Some(bettercut_editor_core::text::CaptionLook::Boxed),
        "captions start boxed, which is one of the four"
    );

    editor
        .set_caption_look(bettercut_editor_core::text::CaptionLook::Soft)
        .unwrap();
    assert_eq!(
        bettercut_ui::caption_list::lane_look(&editor, &rows(&editor)),
        Some(bettercut_editor_core::text::CaptionLook::Soft)
    );
}
