//! A magnetic main track (`Editor::close_up_main_track`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Command, Editor, SettingChange};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three 2-second shots with sound, back to back from zero.
fn three_shots() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Magnet");
    let clips = ["a", "b", "c"]
        .into_iter()
        .map(|name| {
            let mut asset = MediaAsset::new(
                MediaKind::Video,
                format!("C:/media/{name}.mp4"),
                MediaTime::from_seconds(2),
            );
            asset.audio_codec = Some("aac".to_owned());
            let media = editor.import_media(asset);
            editor.place_media(media).unwrap()[0]
        })
        .collect();
    (editor, clips)
}

fn start(editor: &Editor, clip: ClipId) -> TimelineTime {
    editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline
        .start
}

fn sound(editor: &Editor, clip: ClipId) -> ClipId {
    editor
        .linked_with(clip)
        .into_iter()
        .find(|c| *c != clip)
        .unwrap()
}

#[test]
fn a_delete_closes_up_behind_it_in_the_same_undo_step() {
    let (mut editor, clips) = three_shots();
    editor
        .dispatch(Command::ChangeSetting {
            change: SettingChange::MagneticTimeline(true),
        })
        .unwrap();
    assert!(editor.is_magnetic());

    // Take the middle shot out, leaving a gap, then close up.
    let track = editor.main_track().unwrap();
    let sequence = editor.active_sequence().unwrap().id;
    let middle_sound = sound(&editor, clips[1]);
    let sound_track = editor.track_of(middle_sound).unwrap();
    editor
        .dispatch_group(
            "Delete Clip",
            vec![
                Command::RemoveClip {
                    sequence,
                    track,
                    clip: clips[1],
                },
                Command::RemoveClip {
                    sequence,
                    track: sound_track,
                    clip: middle_sound,
                },
            ],
        )
        .unwrap();
    let depth = editor.undo_depth();
    assert_eq!(editor.close_up_main_track().unwrap(), 1);
    assert_eq!(
        editor.undo_depth(),
        depth,
        "the close-up became a step of its own"
    );
    assert_eq!(start(&editor, clips[2]), seconds(2));
    assert_eq!(start(&editor, sound(&editor, clips[2])), seconds(2));

    // One undo brings the shot back and the last one back to where it was.
    editor.undo().unwrap();
    assert_eq!(start(&editor, clips[1]), seconds(2));
    assert_eq!(start(&editor, clips[2]), seconds(4));
    // And redo does both again.
    editor.redo().unwrap();
    assert_eq!(start(&editor, clips[2]), seconds(2));
    assert!(editor.video_clip(clips[1]).is_none());
}

#[test]
fn a_packed_track_is_left_alone_and_a_late_start_is_pulled_to_zero() {
    let (mut editor, clips) = three_shots();
    assert_eq!(editor.close_up_main_track().unwrap(), 0);

    let track = editor.main_track().unwrap();
    // Move everything along by pushing the first shot out to 10 s.
    editor
        .move_clip(track, track, clips[0], seconds(10))
        .unwrap();
    editor.close_up_main_track().unwrap();
    assert_eq!(start(&editor, clips[1]), seconds(0));
    assert_eq!(start(&editor, clips[2]), seconds(2));
    assert_eq!(start(&editor, clips[0]), seconds(4));
}
