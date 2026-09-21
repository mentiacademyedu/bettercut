//! Loop a clip N times (`Editor::loop_clip`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Transition, TransitionKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A 3-second shot with sound, then a 2-second shot after it.
fn edit() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Loops");
    let place = |editor: &mut Editor, name: &str, length: i64| {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(length),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap()[0]
    };
    let shot = place(&mut editor, "loop", 3);
    let after = place(&mut editor, "after", 2);
    (editor, shot, after)
}

fn starts(editor: &Editor) -> Vec<TimelineTime> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.timeline.start)
        .collect()
}

#[test]
fn a_shot_plays_again_with_its_sound_and_the_rest_moves_along() {
    let (mut editor, shot, after) = edit();
    let depth = editor.undo_depth();

    let copies = editor.loop_clip(shot, 3).unwrap();
    assert_eq!(copies.len(), 2);
    assert_eq!(
        starts(&editor),
        vec![seconds(0), seconds(3), seconds(6), seconds(9)]
    );
    let later = editor.video_clip(after).unwrap();
    assert_eq!(later.timeline.start, seconds(9));

    // Same footage; each copy with its own sound, tied to it.
    for copy in &copies {
        let video = editor.video_clip(*copy).unwrap();
        assert_eq!(video.source, editor.video_clip(shot).unwrap().source);
        let partner = editor
            .linked_with(*copy)
            .into_iter()
            .find(|c| c != copy)
            .expect("a copy without its sound");
        let audio = editor.audio_clip(partner).unwrap();
        assert_eq!(audio.timeline, video.timeline);
    }
    assert_eq!(
        editor.active_sequence().unwrap().audio_tracks[0]
            .clips()
            .len(),
        4
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(starts(&editor), vec![seconds(0), seconds(3)]);
}

#[test]
fn a_transition_moves_to_the_end_of_the_loop() {
    let (mut editor, shot, _) = edit();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let sequence = editor.active_sequence().unwrap().id;
    editor
        .dispatch(bettercut_editor_core::Command::SetTransition {
            sequence,
            track,
            clip: shot,
            transition: Some(Transition::new(
                TransitionKind::FadeThroughBlack,
                TimelineTime::from_millis(500),
            )),
        })
        .unwrap();

    let copies = editor.loop_clip(shot, 2).unwrap();
    assert!(editor.video_clip(shot).unwrap().transition_out.is_none());
    assert!(
        editor
            .video_clip(copies[0])
            .unwrap()
            .transition_out
            .is_some()
    );
}

#[test]
fn music_loops_on_its_own_and_counts_are_checked() {
    let (mut editor, _events) = Editor::new_project("Loops");
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/beat.wav",
        MediaTime::from_seconds(4),
    );
    song.audio_codec = Some("pcm".to_owned());
    let media = editor.import_media(song);
    let music = editor.place_media(media).unwrap()[0];

    let copies = editor.loop_clip(music, 4).unwrap();
    assert_eq!(copies.len(), 3);
    let clips = editor.active_sequence().unwrap().audio_tracks[0].clips();
    assert_eq!(clips.len(), 4);
    assert_eq!(clips[3].timeline.start, seconds(12));

    for times in [0, 1, 21] {
        assert!(matches!(
            editor.loop_clip(music, times),
            Err(EditorError::LoopCountOutOfRange)
        ));
    }
}
