//! The Inspector's note field: a note typed and then left — by selecting
//! another clip — is kept, as one undo step.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn draw(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(420.0, 2000.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::panels::inspector(ui, editor, state);
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
fn a_typed_note_is_kept_when_another_clip_is_selected() {
    let (mut editor, _events) = Editor::new_project("Notes");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    ));
    let first = editor.place_media(media).unwrap()[0];
    let second = editor.place_media(media).unwrap()[0];
    let mut state = UiState::default();

    state.select_only(first);
    assert!(draw(&mut editor, &mut state).contains("Add a note to this clip"));

    // What typing leaves behind, then a click on the other clip.
    state.note_draft = Some((first, "needs a grade".to_owned()));
    state.select_only(second);
    let depth = editor.undo_depth();
    let words = draw(&mut editor, &mut state);

    assert_eq!(editor.clip_note(first), Some("needs a grade"));
    assert_eq!(editor.undo_depth(), depth + 1);
    assert!(state.note_draft.is_none());
    assert!(
        !words.contains("needs a grade"),
        "the other clip shows the first one's note"
    );
}
