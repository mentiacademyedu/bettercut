//! The status bar offers Undo only beside a message an edit made.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn words(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 200.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::panels::status_bar(ui, editor, state)
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

#[test]
fn undo_is_offered_after_an_edit_and_not_after_a_toggle() {
    let (mut editor, _events) = Editor::new_project("Status");
    let mut state = UiState::default();
    words(&mut editor, &mut state);

    // An edit and its message in the same frame.
    editor
        .add_markers(&[TimelineTime::from_seconds(1)])
        .unwrap();
    state.info("Added 1 marker");
    assert!(
        words(&mut editor, &mut state).contains("Undo"),
        "no Undo after an edit"
    );

    // A message with no new step: no Undo.
    state.info("Snapping on");
    assert!(
        !words(&mut editor, &mut state).contains("Undo"),
        "Undo offered after a toggle"
    );
}

/// A newer version found by the day's check, and an assistant at work, are
/// said in the bar — and only while they are so.
#[test]
fn a_newer_version_and_an_assistant_are_shown() {
    let (mut editor, _events) = Editor::new_project("Status");
    let mut state = UiState::default();
    let quiet = words(&mut editor, &mut state);
    assert!(
        !quiet.contains("is out") && !quiet.contains("assistant connected"),
        "{quiet}"
    );

    *state.update_found.lock().unwrap() = Some(bettercut_ui::updates::Update {
        version: "9.9.9".to_owned(),
        url: "https://example.invalid/9.9.9".to_owned(),
        installer: None,
    });
    state.assistant_seen = Some(std::time::Instant::now());
    let busy = words(&mut editor, &mut state);
    assert!(busy.contains("bettercut 9.9.9 is out"), "{busy}");
    assert!(busy.contains("assistant connected"), "{busy}");
}

/// An update being fetched says how far it has got, and once fetched and
/// checked, offers the restart that installs it — which closes the window,
/// asking first about unsaved work.
#[test]
fn an_update_downloads_then_offers_a_restart() {
    use bettercut_ui::updates::{Download, Update};
    let (mut editor, _events) = Editor::new_project("Status");
    let mut state = UiState::default();
    *state.update_found.lock().unwrap() = Some(Update {
        version: "9.9.9".to_owned(),
        url: "https://example.invalid/9.9.9".to_owned(),
        installer: None,
    });

    *state.update_download.lock().unwrap() = Some(Download::Fetching {
        got: 25,
        of: 100,
    });
    let fetching = words(&mut editor, &mut state);
    assert!(fetching.contains("Downloading bettercut 9.9.9… 25%"), "{fetching}");

    *state.update_download.lock().unwrap() = Some(Download::Ready("C:/x/setup.exe".into()));
    let ready = words(&mut editor, &mut state);
    assert!(ready.contains("Restart to update to 9.9.9"), "{ready}");

    *state.update_download.lock().unwrap() = Some(Download::Failed("no network".to_owned()));
    let failed = words(&mut editor, &mut state);
    assert!(failed.contains("Update failed"), "{failed}");
}
