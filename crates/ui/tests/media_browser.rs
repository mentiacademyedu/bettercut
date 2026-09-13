//! The media browser's search, kind filter and "remove unused".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::state::{MediaFilter, media_name_matches};
use egui::{Pos2, RawInput, Rect, vec2};

#[test]
fn every_word_typed_must_be_in_the_name() {
    assert!(media_name_matches("Beach Sunset 04.MP4", ""));
    assert!(media_name_matches("Beach Sunset 04.MP4", "sunset"));
    assert!(media_name_matches("Beach Sunset 04.MP4", "04 beach"));
    assert!(!media_name_matches("Beach Sunset 04.MP4", "beach night"));
    assert!(MediaFilter::Photos.accepts(MediaKind::Image));
    assert!(!MediaFilter::Sound.accepts(MediaKind::Video));
    assert!(MediaFilter::All.accepts(MediaKind::Audio));
}

fn library() -> Editor {
    let (mut editor, _events) = Editor::new_project("Library");
    for (kind, name) in [
        (MediaKind::Video, "C:/media/beach sunset.mp4"),
        (MediaKind::Video, "C:/media/city night.mp4"),
        (MediaKind::Audio, "C:/media/sunset song.mp3"),
    ] {
        editor.import_media(MediaAsset::new(kind, name, MediaTime::from_seconds(5)));
    }
    editor
}

/// Draw the browser and return what it wrote.
fn words(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1200.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::panels::media_browser(ui, editor, state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push('\n');
            }
        }
    }
    words
}

#[test]
fn the_list_follows_the_search_and_the_kind() {
    let mut editor = library();
    let mut state = UiState::default();

    let all = words(&mut editor, &mut state);
    assert!(
        all.contains("city night.mp4") && all.contains("Remove 3 unused"),
        "{all}"
    );
    assert!(
        !all.contains("of 3 files"),
        "an unfiltered list said it was filtered"
    );

    state.media_search = "sunset".to_owned();
    let found = words(&mut editor, &mut state);
    assert!(found.contains("beach sunset.mp4") && found.contains("sunset song.mp3"));
    assert!(!found.contains("city night.mp4"), "{found}");
    assert!(found.contains("2 of 3 files"), "{found}");

    state.media_kind = MediaFilter::Sound;
    let sound = words(&mut editor, &mut state);
    assert!(sound.contains("sunset song.mp3") && !sound.contains("beach sunset.mp4"));

    state.media_search = "nothing like this".to_owned();
    assert!(words(&mut editor, &mut state).contains("No file matches"));
}
