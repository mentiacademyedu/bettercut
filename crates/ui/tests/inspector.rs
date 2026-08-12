//! Headless tests for the Inspector panel.
//!
//! The clip-property controls read the project immutably to draw and then
//! dispatch commands mutably, in one function. That is exactly the shape that
//! panics or fails to compile when rearranged carelessly, and nothing else
//! exercises it — the property *commands* are tested in editor-core, but not
//! the panel that drives them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn editor_with_clips() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Inspector");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();

    let video = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let video_id = video.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(video)))
        .unwrap();

    let audio = AudioClip::new(media, TimelineTime::ZERO, source).unwrap();
    let audio_id = audio.id;
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(audio)))
        .unwrap();

    (editor, video_id, audio_id)
}

/// Draw the inspector once. Returns nothing — the assertion is that it neither
/// panics nor leaves egui in a broken state.
fn draw(editor: &mut Editor, state: &mut UiState) {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 900.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::panels::inspector(ui, editor, state);
    });
    output.textures_delta.clear();
}

#[test]
fn the_inspector_draws_with_nothing_selected() {
    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_the_video_property_controls() {
    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_the_audio_property_controls() {
    let (mut editor, _, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(audio);
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_with_several_clips_selected() {
    let (mut editor, video, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    state.selected_clips.insert(audio);
    draw(&mut editor, &mut state);
}

/// A selection pointing at a clip that no longer exists is normal — delete
/// leaves the id behind for a frame — and must not panic.
#[test]
fn a_stale_selection_does_not_panic() {
    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(ClipId::new());
    draw(&mut editor, &mut state);
}

/// Drawing must never mutate the project by itself. Only user input does.
#[test]
fn drawing_changes_nothing() {
    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);

    let before = editor.project().clone();
    draw(&mut editor, &mut state);
    draw(&mut editor, &mut state);

    assert_eq!(
        editor.project(),
        &before,
        "drawing the inspector modified the project"
    );
}

/// A transformed clip shows the Reset button, which is a different code path
/// from the identity case.
#[test]
fn the_inspector_draws_a_transformed_clip() {
    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_clip_property(
            video,
            bettercut_editor_core::ClipProperty::Scale { x: 0.5, y: 0.5 },
            false,
        )
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.insert(video);
    draw(&mut editor, &mut state);
}
