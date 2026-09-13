//! The Delete key, driven through the real shortcut handler.
//!
//! Two bugs lived here. Deleting a video's picture left its sound playing under
//! whatever came next (§12), and pressing Delete on a selected title reported it
//! as already gone — the lookup that found a clip's track had never been told
//! about text tracks (§26). Both are reached only through the key, so that is
//! what these press.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};

/// Press one key for one frame and let the shortcut handler see it.
///
/// The held modifiers go in as a `ModifiersChanged` event ahead of the key: egui
/// takes `i.modifiers` from that, not from the key event's own field, and
/// without it Shift+Delete arrives as a plain Delete. `timeline_interaction.rs`
/// learned the same thing for Ctrl+click.
fn press(editor: &mut Editor, state: &mut UiState, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![
            Event::ModifiersChanged(modifiers),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::shortcuts::handle(ui.ctx(), editor, state, None);
    });
    output.textures_delta.clear();
}

fn linked_pair() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Delete");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

#[test]
fn deleting_the_picture_deletes_its_sound() {
    let (mut editor, picture, _) = linked_pair();
    let mut state = UiState::default();
    state.selected_clips.insert(picture);

    press(&mut editor, &mut state, Key::Delete, Modifiers::NONE);

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 0, "the picture survived");
    assert_eq!(
        sequence.audio_tracks[0].len(),
        0,
        "the sound was left playing on its own"
    );

    // One undo for the pair (§79).
    editor.undo().unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 1);
    assert_eq!(sequence.audio_tracks[0].len(), 1);
}

/// Unlinked on purpose, a picture deletes alone.
#[test]
fn an_unlinked_picture_deletes_alone() {
    let (mut editor, picture, _) = linked_pair();
    editor.unlink(picture).unwrap();
    let mut state = UiState::default();
    state.selected_clips.insert(picture);

    press(&mut editor, &mut state, Key::Delete, Modifiers::NONE);

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 0);
    assert_eq!(
        sequence.audio_tracks[0].len(),
        1,
        "the unlinked sound went too"
    );
}

/// §26: a title is selectable, so it has to be deletable with the same key.
#[test]
fn delete_removes_a_selected_title() {
    let (mut editor, _rx) = Editor::new_project("Delete a title");
    let title = editor.add_text("Hello").unwrap();
    let mut state = UiState::default();
    state.selected_clips.insert(title);

    press(&mut editor, &mut state, Key::Delete, Modifiers::NONE);

    assert!(
        editor.text_clip(title).is_none(),
        "Delete left the title on the timeline"
    );
    assert!(
        state.status.as_ref().is_none_or(|s| !s.is_error),
        "Delete reported an error: {:?}",
        state.status.as_ref().map(|s| &s.text)
    );
}

/// An adjustment is selectable too, so the same key has to remove it — through
/// its own command, since a media clip's removal does not know its lane.
#[test]
fn delete_removes_a_selected_adjustment() {
    let (mut editor, _rx) = Editor::new_project("Delete an adjustment");
    let adjustment = editor.add_adjustment().unwrap();
    let mut state = UiState::default();
    state.selected_clips.insert(adjustment);

    press(&mut editor, &mut state, Key::Delete, Modifiers::NONE);

    assert!(
        editor.adjustment_clip(adjustment).is_none(),
        "Delete left the adjustment on the timeline"
    );
    assert!(
        state.status.as_ref().is_none_or(|s| !s.is_error),
        "Delete reported an error: {:?}",
        state.status.as_ref().map(|s| &s.text)
    );
}

/// Ripple delete of a linked pair closes the gap on *both* tracks, in one undo.
#[test]
fn ripple_deleting_a_linked_pair_closes_both_tracks() {
    let (mut editor, _rx) = Editor::new_project("Ripple");
    let mut place = || {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            "C:/media/a.mp4",
            MediaTime::from_seconds(10),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap()
    };
    let first = place();
    let _second = place();

    let mut state = UiState::default();
    state.selected_clips.insert(first[0]);
    press(&mut editor, &mut state, Key::Delete, Modifiers::SHIFT);

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 1);
    assert_eq!(sequence.audio_tracks[0].len(), 1);
    assert_eq!(
        sequence.video_tracks[0].clips()[0].timeline.start,
        bettercut_editor_core::foundation::TimelineTime::ZERO,
        "the picture gap was not closed"
    );
    assert_eq!(
        sequence.audio_tracks[0].clips()[0].timeline.start,
        bettercut_editor_core::foundation::TimelineTime::ZERO,
        "the sound gap was not closed"
    );

    editor.undo().unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        (
            sequence.video_tracks[0].len(),
            sequence.audio_tracks[0].len()
        ),
        (2, 2),
        "one undo did not restore the whole ripple"
    );
}
