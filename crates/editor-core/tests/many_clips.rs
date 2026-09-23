//! One change on many clips (`Editor::set_clips_property`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AnimatedParameter, SourceRange, VideoClip};
use bettercut_editor_core::{ClipProperty, Editor};

fn three() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Many");
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap();
    let mut ids = Vec::new();
    for index in 0..3 {
        let clip = VideoClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(index * 2),
            source,
        )
        .unwrap();
        ids.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    (editor, ids)
}

#[test]
fn a_change_goes_on_every_clip_as_one_step() {
    let (mut editor, clips) = three();
    let depth = editor.undo_depth();
    assert_eq!(
        editor
            .set_clips_property(&clips, ClipProperty::Brightness(1.3), false)
            .unwrap(),
        3
    );
    for clip in &clips {
        assert!((editor.video_clip(*clip).unwrap().color.brightness - 1.3).abs() < 1e-6);
    }
    assert_eq!(editor.undo_depth(), depth + 1);

    // A drag: every frame continues the same step.
    editor
        .set_clips_property(&clips, ClipProperty::Brightness(1.1), false)
        .unwrap();
    editor
        .set_clips_property(&clips, ClipProperty::Brightness(1.2), true)
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 2, "a drag is one step");

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert!((editor.video_clip(clips[2]).unwrap().color.brightness - 1.0).abs() < 1e-6);
}

#[test]
fn a_keyframed_clip_is_passed_over() {
    let (mut editor, clips) = three();
    // Key the middle clip's opacity; its value lives in its keys now.
    editor.set_playhead(TimelineTime::from_millis(2_500));
    editor
        .toggle_keyframe(clips[1], ClipProperty::Opacity(1.0))
        .unwrap();
    assert!(
        editor
            .video_clip(clips[1])
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::Opacity)
    );
    assert!(editor.animates(clips[1], ClipProperty::Opacity(0.5)));
    assert_eq!(
        editor
            .set_clips_property(&clips, ClipProperty::Opacity(0.5), false)
            .unwrap(),
        2
    );
    assert!((editor.video_clip(clips[0]).unwrap().opacity - 0.5).abs() < 1e-6);
}
