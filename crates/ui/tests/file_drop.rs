//! Files dragged in from the desktop.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bettercut_editor_core::Editor;
use bettercut_ui::UiState;
use egui::{Event, Pos2, RawInput, Rect, vec2};

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

/// The bottom third of an 800×600 window.
fn timeline() -> Rect {
    Rect::from_min_max(Pos2::new(0.0, 400.0), Pos2::new(800.0, 600.0))
}

/// One frame with `input` applied, returning every word drawn.
fn frame(editor: &mut Editor, state: &mut UiState, input: RawInput) -> String {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::file_drop::handle(ui.ctx(), editor, state, timeline());
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

fn drop_at(pointer: Pos2, files: &[&str]) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![Event::PointerMoved(pointer)],
        dropped_files: files
            .iter()
            .map(|name| {
                Arc::new(Dropped(fixture(name))) as Arc<dyn egui::DroppedFile + Send + Sync>
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn a_file_dropped_on_the_timeline_is_imported_and_placed() {
    let (mut editor, _events) = Editor::new_project("Drop");
    let mut state = UiState::default();

    frame(
        &mut editor,
        &mut state,
        drop_at(Pos2::new(400.0, 500.0), &["still.png", "ntsc-2997.mp4"]),
    );

    assert_eq!(editor.project().media.len(), 2);
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks[0].len(),
        2,
        "both were placed, one after the other"
    );
}

#[test]
fn a_file_dropped_elsewhere_is_only_imported() {
    let (mut editor, _events) = Editor::new_project("Drop");
    let mut state = UiState::default();

    frame(
        &mut editor,
        &mut state,
        drop_at(Pos2::new(400.0, 100.0), &["still.png"]),
    );

    assert_eq!(editor.project().media.len(), 1);
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 0);
}

#[test]
fn an_unreadable_file_is_reported_not_fatal() {
    let (mut editor, _events) = Editor::new_project("Drop");
    let mut state = UiState::default();

    let mut input = drop_at(Pos2::new(400.0, 500.0), &["still.png"]);
    input
        .dropped_files
        .push(Arc::new(Dropped(PathBuf::from("C:/nowhere/missing.mp4"))));
    frame(&mut editor, &mut state, input);

    assert_eq!(
        editor.project().media.len(),
        1,
        "the good file still came in"
    );
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 1);
}

/// While files are held over the window, it says what letting go will do.
#[test]
fn hovering_files_says_what_a_drop_will_do() {
    let (mut editor, _events) = Editor::new_project("Drop");
    let mut state = UiState::default();
    let hover = |pointer: Pos2| RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![Event::PointerMoved(pointer)],
        hovered_files: vec![egui::HoveredFile {
            path: Some(fixture("still.png")),
            mime: String::new(),
        }],
        ..Default::default()
    };

    let over_timeline = frame(&mut editor, &mut state, hover(Pos2::new(400.0, 500.0)));
    assert!(
        over_timeline.contains("add to the end of the timeline"),
        "{over_timeline}"
    );
    let elsewhere = frame(&mut editor, &mut state, hover(Pos2::new(400.0, 100.0)));
    assert!(elsewhere.contains("Drop to import"), "{elsewhere}");
    assert_eq!(
        editor.project().media.len(),
        0,
        "hovering imported something"
    );
}
