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
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
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
