//! Every inspector tab fits the inspector. A tab whose content is wider than
//! the panel lets the panel spill left, and the preview — drawn after it —
//! paints over its left edge: the Colours tab's tab bar and filter row were
//! cut off that way until a screenshot showed it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::panels::InspectorTab;
use egui::{Pos2, RawInput, Rect, vec2};

/// The narrowest the inspector is drawn (its panel's `min_size`).
const NARROWEST: f32 = 300.0;

fn width_of(editor: &mut Editor, state: &mut UiState, room: f32) -> f32 {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 3000.0))),
        ..Default::default()
    };
    let mut used = 0.0;
    let mut output = ctx.run_ui(input, |ui| {
        let response = ui.scope(|ui| {
            ui.set_max_width(room);
            bettercut_ui::panels::inspector(ui, editor, state);
        });
        used = response.response.rect.width();
    });
    output.textures_delta.clear();
    used
}

#[test]
fn every_tab_fits_the_narrowest_inspector() {
    let (mut editor, _events) = Editor::new_project("Widths");
    // A long name, as cameras give: no spaces to wrap at.
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a-very-long-file-name-from-a-camera-that-names-things-badly-0001.mp4",
        MediaTime::from_seconds(20),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.audio_codec = Some("aac".to_owned());
    asset.audio_channels = Some(2);
    asset.audio_sample_rate = Some(48_000);
    let media = editor.import_media(asset);
    let clips = editor.place_media(media).unwrap();
    let mut state = UiState::default();
    state.select_only(clips[0]);

    let mut too_wide = Vec::new();
    for tab in InspectorTab::ALL {
        state.inspector_tab = tab;
        // Twice: some controls size themselves from the first frame.
        width_of(&mut editor, &mut state, NARROWEST);
        let used = width_of(&mut editor, &mut state, NARROWEST);
        eprintln!("{tab:?}: {used:.0} of {NARROWEST}");
        if used > NARROWEST + 0.5 {
            too_wide.push(format!("{tab:?} {used:.0}"));
        }
    }
    assert!(
        too_wide.is_empty(),
        "wider than the inspector: {too_wide:?}"
    );
}

/// The other things the inspector shows: a title, a sound on its own, and the
/// whole sequence when nothing is chosen.
#[test]
fn titles_sound_and_the_sequence_fit_too() {
    let (mut editor, _events) = Editor::new_project("Widths");
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.m4a",
        MediaTime::from_seconds(20),
    );
    song.audio_codec = Some("aac".to_owned());
    song.audio_channels = Some(2);
    song.audio_sample_rate = Some(48_000);
    let song = editor.import_media(song);
    let sound = editor.place_media(song).unwrap()[0];
    let title = editor.add_text("A title").unwrap();

    let mut too_wide = Vec::new();
    for (what, chosen) in [
        ("title", Some(title)),
        ("sound", Some(sound)),
        ("sequence", None),
    ] {
        let mut state = UiState::default();
        if let Some(clip) = chosen {
            state.select_only(clip);
        }
        for tab in InspectorTab::ALL {
            state.inspector_tab = tab;
            width_of(&mut editor, &mut state, NARROWEST);
            let used = width_of(&mut editor, &mut state, NARROWEST);
            if used > NARROWEST + 0.5 {
                too_wide.push(format!("{what} {tab:?} {used:.0}"));
            }
        }
    }
    assert!(
        too_wide.is_empty(),
        "wider than the inspector: {too_wide:?}"
    );
}

/// The media panel at its narrowest (its `min_size`), with a long name in it.
#[test]
fn the_media_panel_fits_at_its_narrowest() {
    let (mut editor, _events) = Editor::new_project("Widths");
    let mut long = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a-very-long-file-name-from-a-camera-that-names-things-badly-0001.mp4",
        MediaTime::from_seconds(20),
    );
    long.width = 1920;
    long.height = 1080;
    editor.import_media(long);
    let mut state = UiState::default();
    let narrowest = 160.0;
    let ctx = egui::Context::default();
    let mut used = 0.0;
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 3000.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let response = ui.scope(|ui| {
                ui.set_max_width(narrowest);
                bettercut_ui::panels::media_browser(ui, &mut editor, &mut state);
            });
            used = response.response.rect.width();
        });
        output.textures_delta.clear();
    }
    assert!(
        used <= narrowest + 0.5,
        "the media panel is {used:.0} wide at {narrowest}"
    );
}
