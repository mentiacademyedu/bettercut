//! One transition on every cut (`Editor::transition_every_cut`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, TransitionKind};

/// Three 3 s shots back to back on V1, each cut from the middle of a 10 s
/// file so there is footage either side of every cut.
fn montage() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Montage");
    let mut ids = Vec::new();
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(10),
        ));
        let track = editor.active_sequence().unwrap().video_tracks[0].id;
        let clip = bettercut_editor_core::timeline::VideoClip::new(
            media,
            TimelineTime::from_seconds(3 * i as i64),
            SourceRange::new(MediaTime::from_seconds(3), MediaTime::from_seconds(6)).unwrap(),
        )
        .unwrap();
        ids.push(clip.id);
        editor
            .add_clip(
                track,
                bettercut_editor_core::ClipPayload::Video(Box::new(clip)),
            )
            .unwrap();
    }
    (editor, ids)
}

#[test]
fn every_cut_takes_the_transition_in_one_step() {
    let (mut editor, clips) = montage();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let depth = editor.undo_depth();

    let (applied, skipped) = editor
        .transition_every_cut(track, TransitionKind::Crossfade)
        .unwrap();
    assert_eq!((applied, skipped), (2, 0), "two cuts between three shots");
    assert_eq!(editor.undo_depth(), depth + 1);
    for clip in &clips[..2] {
        let transition = editor.video_clip(*clip).unwrap().transition_out.unwrap();
        assert_eq!(transition.kind, TransitionKind::Crossfade);
    }
    assert!(
        editor
            .video_clip(clips[2])
            .unwrap()
            .transition_out
            .is_none(),
        "the last shot has no cut after it"
    );

    assert_eq!(editor.remove_every_transition(track).unwrap(), 2);
    assert!(
        clips
            .iter()
            .all(|c| editor.video_clip(*c).unwrap().transition_out.is_none())
    );
    editor.undo().unwrap();
    assert!(
        editor
            .video_clip(clips[0])
            .unwrap()
            .transition_out
            .is_some()
    );
}

/// A cut with no footage to spare overlaps its clips to take a crossfade,
/// and one that needs no footage lands there as it is.
#[test]
fn a_cut_without_room_overlaps_to_make_it() {
    let (mut editor, _events) = Editor::new_project("Tight");
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    for (i, name) in ["a", "b"].iter().enumerate() {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(3),
        ));
        // The whole file: nothing before its start or after its end.
        let clip = bettercut_editor_core::timeline::VideoClip::new(
            media,
            TimelineTime::from_seconds(3 * i as i64),
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(3)).unwrap(),
        )
        .unwrap();
        editor
            .add_clip(
                track,
                bettercut_editor_core::ClipPayload::Video(Box::new(clip)),
            )
            .unwrap();
    }
    // No footage either side: the clips overlap to make room, as CapCut
    // does, and the edit gets shorter by about the transition.
    let length = editor.active_sequence().unwrap().duration();
    assert_eq!(
        editor
            .transition_every_cut(track, TransitionKind::Crossfade)
            .unwrap(),
        (1, 0)
    );
    assert!(editor.active_sequence().unwrap().duration() < length);
    assert_eq!(
        editor
            .transition_every_cut(track, TransitionKind::FadeThroughBlack)
            .unwrap(),
        (1, 0)
    );
}
