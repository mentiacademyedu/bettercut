//! Every transition on a lane made one length
//! (`Editor::retime_every_transition`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, TransitionKind, VideoClip};

#[test]
fn every_transition_takes_the_length_and_keeps_its_kind() {
    let (mut editor, _events) = Editor::new_project("Retime");
    // Stills have endless handles: the room is never the limit here.
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let mut clips = Vec::new();
    for index in 0..3 {
        let clip = VideoClip::new(media, TimelineTime::from_seconds(index * 4), source).unwrap();
        clips.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    editor
        .set_transition(clips[0], TransitionKind::Crossfade)
        .unwrap();
    editor
        .set_transition(clips[1], TransitionKind::FadeThroughBlack)
        .unwrap();
    let depth = editor.undo_depth();

    let changed = editor
        .retime_every_transition(track, TimelineTime::from_seconds(1))
        .unwrap();
    assert_eq!(changed, 2);
    let first = editor.video_clip(clips[0]).unwrap().transition_out.unwrap();
    let second = editor.video_clip(clips[1]).unwrap().transition_out.unwrap();
    assert_eq!(first.duration, TimelineTime::from_seconds(1));
    assert_eq!(second.duration, TimelineTime::from_seconds(1));
    assert_eq!(
        second.kind,
        TransitionKind::FadeThroughBlack,
        "the kind changed"
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    assert_eq!(
        editor
            .retime_every_transition(track, TimelineTime::from_seconds(1))
            .unwrap(),
        0,
        "already that long"
    );
}

#[test]
fn mixed_transitions_take_turns_across_the_cuts() {
    let (mut editor, _events) = Editor::new_project("Mixed");
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let mut clips = Vec::new();
    for index in 0..4 {
        let clip = VideoClip::new(media, TimelineTime::from_seconds(index * 4), source).unwrap();
        clips.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    let depth = editor.undo_depth();
    assert_eq!(editor.mixed_transition_every_cut(track).unwrap(), (3, 0));
    let kinds: Vec<TransitionKind> = clips[..3]
        .iter()
        .map(|c| editor.video_clip(*c).unwrap().transition_out.unwrap().kind)
        .collect();
    assert_eq!(kinds, Editor::MIXED_TRANSITIONS[..3]);
    assert!(
        editor
            .video_clip(clips[3])
            .unwrap()
            .transition_out
            .is_none(),
        "the last is not a cut"
    );
    assert_eq!(editor.undo_depth(), depth + 1);
}
