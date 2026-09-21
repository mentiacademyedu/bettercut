//! Crossfading one sound clip into the next.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::SourceRange;
use bettercut_editor_core::{ClipPayload, Editor, EditorError};

/// Two four-second stretches of a twenty-second song, back to back on A1: the
/// first from 2 s into the file, the second from 10 s.
fn two_sounds() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Crossfade");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(20),
    ));
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    let mut place = |start: i64, from: i64| {
        let clip = bettercut_editor_core::timeline::AudioClip::new(
            media,
            TimelineTime::from_seconds(start),
            SourceRange::new(
                MediaTime::from_seconds(from),
                MediaTime::from_seconds(from + 4),
            )
            .unwrap(),
        )
        .unwrap();
        let id = clip.id;
        editor
            .add_clip(track, ClipPayload::Audio(Box::new(clip)))
            .unwrap();
        id
    };
    let first = place(0, 2);
    let second = place(4, 10);
    (editor, first, second)
}

#[test]
fn a_crossfade_is_set_undone_and_held_to_the_material() {
    let (mut editor, first, _) = two_sounds();
    let applied = editor
        .set_audio_crossfade(first, TimelineTime::from_millis(800), false)
        .unwrap();
    assert_eq!(applied, TimelineTime::from_millis(800));
    assert_eq!(editor.audio_clip(first).unwrap().crossfade_out, applied);
    assert_eq!(editor.undo_label().as_deref(), Some("Change Crossfade"));

    // Far past the ceiling: held to it.
    let held = editor
        .set_audio_crossfade(first, TimelineTime::from_seconds(30), false)
        .unwrap();
    assert_eq!(held, bettercut_editor_core::timeline::MAX_CROSSFADE);

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(
        editor.audio_clip(first).unwrap().crossfade_out,
        TimelineTime::ZERO
    );
}

/// The last clip has nothing to fade into; nor does one before a gap.
#[test]
fn only_a_cut_can_be_crossfaded() {
    let (mut editor, first, second) = two_sounds();
    assert_eq!(editor.next_touching_sound(first), Some(second));
    assert!(matches!(
        editor.set_audio_crossfade(second, TimelineTime::from_millis(500), false),
        Err(EditorError::NoRoomForTransition)
    ));
    let track = editor.track_of(second).unwrap();
    editor
        .move_clip(track, track, second, TimelineTime::from_seconds(5))
        .unwrap();
    assert_eq!(editor.next_touching_sound(first), None);
}

/// Splitting the outgoing clip leaves the crossfade on the half whose end it
/// is on, not a second one at the split.
#[test]
fn a_split_keeps_the_crossfade_on_the_end_it_belongs_to() {
    let (mut editor, first, _) = two_sounds();
    editor
        .set_audio_crossfade(first, TimelineTime::from_millis(600), false)
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(2));
    editor.split_at_playhead(&[first]).unwrap();
    let clips = editor.active_sequence().unwrap().audio_tracks[0]
        .clips()
        .to_vec();
    assert_eq!(
        clips[0].crossfade_out,
        TimelineTime::ZERO,
        "a crossfade appeared at the split"
    );
    assert_eq!(clips[1].crossfade_out, TimelineTime::from_millis(600));
}

/// An outgoing clip that stops close to the end of its file has little to
/// fade out with: the crossfade is held to twice what is left after it.
#[test]
fn a_crossfade_is_held_to_what_is_left_of_the_file() {
    let (mut editor, _events) = Editor::new_project("Near the end");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(20),
    ));
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    // Ends 200 ms before the end of the file.
    let near_end = bettercut_editor_core::timeline::AudioClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(
            MediaTime::from_millis(15_800),
            MediaTime::from_millis(19_800),
        )
        .unwrap(),
    )
    .unwrap();
    let first = near_end.id;
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(near_end)))
        .unwrap();
    let next = bettercut_editor_core::timeline::AudioClip::new(
        media,
        TimelineTime::from_seconds(4),
        SourceRange::new(MediaTime::from_seconds(5), MediaTime::from_seconds(9)).unwrap(),
    )
    .unwrap();
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(next)))
        .unwrap();

    let applied = editor
        .set_audio_crossfade(first, TimelineTime::from_seconds(1), false)
        .unwrap();
    assert_eq!(applied, TimelineTime::from_millis(400));
}
