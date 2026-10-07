//! The Cover row of the Export window: which frame is saved beside the file,
//! chosen there rather than only from the transport.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::export_dialog::ExportDialog;
use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};

fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut UiState,
    dialog: &mut ExportDialog,
    events: Vec<Event>,
) -> Vec<(String, Rect)> {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        let requests = bettercut_ui::export_dialog::show(ui.ctx(), editor, state, dialog, false);
        assert!(requests.is_empty(), "nothing asked to export");
    });
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            _ => None,
        })
        .collect()
}

fn click(at: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        },
    ]
}

fn find(words: &[(String, Rect)], word: &str) -> Option<Pos2> {
    words
        .iter()
        .find(|(w, _)| w == word)
        .map(|(_, r)| r.center())
}

#[test]
fn the_cover_is_chosen_and_cleared_in_the_export_window() {
    let (mut editor, _events) = Editor::new_project("Trip");
    let clip = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/trip.mp4",
        MediaTime::from_seconds(10),
    ));
    editor.place_media(clip).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    let mut state = UiState::default();
    let mut dialog = ExportDialog::default();
    dialog.open(&editor);
    let ctx = egui::Context::default();

    let _ = frame(&ctx, &mut editor, &mut state, &mut dialog, Vec::new());
    let words = frame(&ctx, &mut editor, &mut state, &mut dialog, Vec::new());
    assert!(find(&words, "Cover").is_some(), "the row is there");
    assert!(find(&words, "None").is_some(), "no cover yet");
    assert!(find(&words, "No Cover").is_none(), "nothing to clear");

    let use_playhead = find(&words, "Use Playhead").expect("a way to choose one");
    let _ = frame(
        &ctx,
        &mut editor,
        &mut state,
        &mut dialog,
        click(use_playhead),
    );
    assert_eq!(editor.cover_frame(), Some(TimelineTime::from_seconds(3)));
    assert!(dialog.open, "choosing a cover does not export or close");

    let words = frame(&ctx, &mut editor, &mut state, &mut dialog, Vec::new());
    assert!(
        words.iter().any(|(w, _)| w.contains("cover.png")),
        "the picture's name is said: {:?}",
        words.iter().map(|(w, _)| w.as_str()).collect::<Vec<_>>()
    );
    assert!(
        find(&words, "Use Playhead").is_none(),
        "already the playhead's frame"
    );

    let no_cover = find(&words, "No Cover").expect("a way to clear it");
    let _ = frame(&ctx, &mut editor, &mut state, &mut dialog, click(no_cover));
    assert_eq!(editor.cover_frame(), None);
}
