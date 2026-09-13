//! The Markers window (`bettercut_ui::marker_list`).
//!
//! Driven through egui itself — focus, typed text, Enter and Escape — because
//! what matters is when a typed name reaches the editor: once, when the field
//! is left, and not at all when the typing is abandoned.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_ui::UiState;
use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn setup() -> (Editor, UiState, egui::Context) {
    let (mut editor, _events) = Editor::new_project("Markers");
    editor
        .add_markers(&[seconds(2), seconds(5), seconds(9)])
        .unwrap();
    editor.set_marker_label(seconds(5), "chorus").unwrap();
    let mut state = UiState::default();
    state.markers_open = true;
    (editor, state, egui::Context::default())
}

/// One frame with `events`; returns the words drawn.
fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut UiState,
    events: Vec<Event>,
) -> String {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::marker_list::show(ui.ctx(), editor, state);
    });
    output.textures_delta.clear();
    let mut words = String::new();
    for clipped in &output.shapes {
        if let egui::Shape::Text(text) = &clipped.shape {
            words.push_str(text.galley.text());
            words.push(' ');
        }
    }
    words
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// Focus the name field of the marker at `at` and type `text` into it.
fn type_name(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut UiState,
    at: TimelineTime,
    text: &str,
) {
    ctx.memory_mut(|m| m.request_focus(egui::Id::new(("marker", at.ticks()))));
    frame(ctx, editor, state, Vec::new());
    frame(ctx, editor, state, vec![Event::Text(text.to_owned())]);
}

#[test]
fn every_marker_is_listed_with_its_name() {
    let (mut editor, mut state, ctx) = setup();
    frame(&ctx, &mut editor, &mut state, Vec::new());
    let words = frame(&ctx, &mut editor, &mut state, Vec::new());
    assert!(words.contains("3 markers"), "{words}");
    assert!(words.contains("chorus"), "{words}");
    assert!(words.contains("name this marker"), "{words}");
}

#[test]
fn no_markers_says_how_to_add_one() {
    let (mut editor, _events) = Editor::new_project("Empty");
    let mut state = UiState::default();
    state.markers_open = true;
    let ctx = egui::Context::default();
    frame(&ctx, &mut editor, &mut state, Vec::new());
    let words = frame(&ctx, &mut editor, &mut state, Vec::new());
    assert!(words.contains("No markers yet"), "{words}");
    assert!(words.contains("Press M"), "{words}");
}

/// Typing does not touch the project; leaving the field with Enter names the
/// marker in one undo step.
#[test]
fn a_name_is_applied_once_when_the_field_is_left() {
    let (mut editor, mut state, ctx) = setup();
    frame(&ctx, &mut editor, &mut state, Vec::new());
    let depth = editor.undo_depth();

    type_name(&ctx, &mut editor, &mut state, seconds(2), "intro");
    assert!(
        editor.markers()[0].label.is_empty(),
        "a keystroke reached the project"
    );
    assert_eq!(editor.undo_depth(), depth);

    frame(&ctx, &mut editor, &mut state, vec![key(Key::Enter)]);
    assert_eq!(editor.markers()[0].label, "intro");
    assert_eq!(editor.undo_depth(), depth + 1);
    assert!(state.marker_draft.is_none());
}

/// Escape abandons the typing: the name stays as it was.
#[test]
fn escape_keeps_the_old_name() {
    let (mut editor, mut state, ctx) = setup();
    frame(&ctx, &mut editor, &mut state, Vec::new());
    let depth = editor.undo_depth();

    type_name(&ctx, &mut editor, &mut state, seconds(5), " and bridge");
    frame(&ctx, &mut editor, &mut state, vec![key(Key::Escape)]);
    assert_eq!(editor.markers()[1].label, "chorus");
    assert_eq!(editor.undo_depth(), depth);
    assert!(state.marker_draft.is_none());
}

/// Closing the window mid-name — here with the toolbar button, which only
/// flips the flag — keeps what was typed rather than losing it.
#[test]
fn closing_the_window_keeps_a_half_typed_name() {
    let (mut editor, mut state, ctx) = setup();
    frame(&ctx, &mut editor, &mut state, Vec::new());
    type_name(&ctx, &mut editor, &mut state, seconds(9), "outro");
    assert!(editor.markers()[2].label.is_empty());

    state.markers_open = false;
    frame(&ctx, &mut editor, &mut state, Vec::new());
    assert_eq!(editor.markers()[2].label, "outro");
    assert!(state.marker_draft.is_none());
}

/// "Copy Chapters" puts the chapter list on the clipboard — through egui, as
/// a real click does — and says what a video site would reject.
#[test]
fn copy_chapters_puts_the_list_on_the_clipboard() {
    let (mut editor, mut state, ctx) = setup();
    // Chapters need somewhere to end: a picture a minute long.
    let media = editor.import_media(bettercut_editor_core::media::MediaAsset::new(
        bettercut_editor_core::media::MediaKind::Video,
        "C:/media/long.mp4",
        bettercut_editor_core::foundation::MediaTime::from_seconds(60),
    ));
    editor.place_media(media).unwrap();

    // Find the button's label on screen, then click it.
    let input = |events| RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
        events,
        ..Default::default()
    };
    let mut at = None;
    for _ in 0..2 {
        let mut output = ctx.run_ui(input(Vec::new()), |ui| {
            bettercut_ui::marker_list::show(ui.ctx(), &mut editor, &mut state);
        });
        output.textures_delta.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape
                && text.galley.text() == "Copy Chapters"
            {
                at = Some(text.pos + text.galley.rect.center().to_vec2());
            }
        }
    }
    let at = at.expect("no Copy Chapters button");
    let press = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut copied = None;
    for events in [
        vec![Event::PointerMoved(at)],
        vec![press(true)],
        vec![press(false)],
    ] {
        let mut output = ctx.run_ui(input(events), |ui| {
            bettercut_ui::marker_list::show(ui.ctx(), &mut editor, &mut state);
        });
        output.textures_delta.clear();
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text.clone());
            }
        }
    }
    // Markers at 2, 5 (named "chorus") and 9 s: all under ten seconds apart.
    assert_eq!(
        copied.as_deref(),
        Some("0:00 Intro\n0:02 Chapter 2\n0:05 chorus\n0:09 Chapter 4\n"),
    );
    let status = state.status.as_ref().expect("nothing was said");
    assert!(status.text.contains("Copied 4 chapters"), "{}", status.text);
    assert!(status.text.contains("under 10 seconds"), "{}", status.text);
}
