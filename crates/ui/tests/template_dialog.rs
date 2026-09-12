//! The Templates window (§31): choose, fill, apply.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::template_dialog::TemplateDialog;
use egui::{Pos2, RawInput, Rect, vec2};

fn video(editor: &mut Editor, name: &str) -> MediaId {
    editor.import_media(MediaAsset::new(
        MediaKind::Video,
        format!("C:/media/{name}.mp4"),
        MediaTime::from_seconds(20),
    ))
}

/// A templates folder that does not exist, so a test never reads the real
/// user's templates.
fn empty_folder() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("bettercut-no-templates-{}", std::process::id()))
}

fn user_template(id: &str, name: &str) -> String {
    format!(
        r#"{{ "schema_version": 1, "id": "{id}", "name": "{name}", "category": "Mine",
             "duration": 2,
             "slots": [ {{ "id": "shot", "type": "video" }} ],
             "elements": [ {{ "type": "clip", "slot": "shot", "start": 0, "duration": 2 }} ] }}"#
    )
}

#[test]
fn the_users_own_templates_follow_the_starters() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("mine.json"),
        user_template("mine", "Mine"),
    )
    .unwrap();
    std::fs::write(folder.path().join("broken.json"), "{").unwrap();

    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(folder.path().to_path_buf());

    assert_eq!(dialog.templates().last().unwrap().id, "mine");
    assert_eq!(dialog.rejected().len(), 1);
    assert_eq!(dialog.rejected()[0].0, "broken.json");
}

#[test]
fn adding_a_file_installs_it_and_chooses_it() {
    let folder = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let file = downloads.path().join("cool.json");
    std::fs::write(&file, user_template("cool", "Cool One")).unwrap();

    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(folder.path().to_path_buf());
    let message = dialog.add_file(&file).expect("a valid template is added");

    assert!(message.contains("Cool One"), "{message}");
    assert_eq!(dialog.selected().unwrap().id, "cool");
    assert!(folder.path().join("cool.json").is_file());

    std::fs::write(&file, "{ \"schema_version\": 1 }").unwrap();
    let err = dialog
        .add_file(&file)
        .expect_err("an invalid file is refused");
    assert!(err.contains("not a valid template"), "{err}");
}

/// Closing the window to go and import a clip must not lose what was chosen.
#[test]
fn reopening_keeps_the_chosen_template_and_its_slots() {
    let (mut editor, _events) = Editor::new_project("Dialog");
    let clip = video(&mut editor, "a");
    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(empty_folder());
    dialog.select(index_of(&dialog, "outro"));
    dialog.set_media("background", Some(clip));
    dialog.open = false;

    dialog.open_with_folder(empty_folder());

    assert_eq!(dialog.selected().unwrap().id, "outro");
    assert!(dialog.fills().contains_key("background"));
}

fn index_of(dialog: &TemplateDialog, id: &str) -> usize {
    dialog
        .templates()
        .iter()
        .position(|t| t.id == id)
        .unwrap_or_else(|| panic!("no starter called {id}"))
}

#[test]
fn opening_lists_the_starters_with_the_first_chosen() {
    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(empty_folder());

    assert!(dialog.open);
    assert!(dialog.templates().len() >= 5);
    assert_eq!(dialog.selected().unwrap().id, dialog.templates()[0].id);
}

#[test]
fn text_slots_start_with_the_templates_own_wording() {
    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(empty_folder());
    dialog.select(index_of(&dialog, "meme"));

    let fills = dialog.fills();
    assert_eq!(
        fills.get("top"),
        Some(&bettercut_editor_core::SlotFill::Text(
            "WHEN THE CODE COMPILES".to_owned()
        ))
    );
    assert!(!fills.contains_key("clip"), "media slots start empty");
}

#[test]
fn choosing_another_template_starts_its_slots_afresh() {
    let (mut editor, _events) = Editor::new_project("Dialog");
    let clip = video(&mut editor, "a");
    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(empty_folder());
    dialog.select(index_of(&dialog, "quick-intro"));
    dialog.set_media("main", Some(clip));

    dialog.select(index_of(&dialog, "outro"));
    dialog.select(index_of(&dialog, "quick-intro"));

    assert!(
        !dialog.fills().contains_key("main"),
        "a clip chosen for one template leaked into another"
    );
}

#[test]
fn applying_places_the_template_at_the_end_and_reports_empty_slots() {
    let (mut editor, _events) = Editor::new_project("Dialog");
    let existing = video(&mut editor, "existing");
    editor.place_media(existing).unwrap(); // 0–20 s
    let clip = video(&mut editor, "a");

    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(empty_folder());
    dialog.select(index_of(&dialog, "quick-intro"));
    dialog.set_media("main", Some(clip));
    dialog.set_words("title", "Launch day");

    let message = dialog.apply(&mut editor).expect("applies");

    assert!(message.starts_with("Added Quick Intro"), "{message}");
    assert!(
        message.contains("left empty: Music"),
        "the report names the slot as the user saw it: {message}"
    );
    assert!(
        !dialog.open,
        "the window closes once the template is placed"
    );
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(20));

    let sequence = editor.active_sequence().unwrap();
    let placed = &sequence.video_tracks[0].clips()[1];
    assert_eq!(placed.timeline.start, TimelineTime::from_seconds(20));
    assert_eq!(sequence.text_tracks[0].clips()[0].text, "Launch day");
}

#[test]
fn a_refusal_keeps_the_window_open() {
    let (mut editor, _events) = Editor::new_project("Dialog");
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(60),
    );
    song.audio_codec = Some("mp3".to_owned());
    let song = editor.import_media(song);
    let mut dialog = TemplateDialog::default();
    dialog.open_with_folder(empty_folder());
    dialog.select(index_of(&dialog, "quick-intro"));
    dialog.set_media("main", Some(song));

    let err = dialog
        .apply(&mut editor)
        .expect_err("a song is not a picture");
    assert!(err.contains("no picture"), "{err}");
    assert!(dialog.open, "the user's choices are still there to fix");
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 0);
}

/// The window draws, with and without media, without panicking.
#[test]
fn the_window_draws() {
    for with_media in [false, true] {
        let (mut editor, _events) = Editor::new_project("Dialog");
        if with_media {
            video(&mut editor, "a");
        }
        let mut state = UiState::default();
        state.template_dialog.open_with_folder(empty_folder());

        let ctx = egui::Context::default();
        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                bettercut_ui::template_dialog::show(ui.ctx(), &mut editor, &mut state);
            });
            output.textures_delta.clear();
        }
        assert!(state.template_dialog.open);
    }
}
