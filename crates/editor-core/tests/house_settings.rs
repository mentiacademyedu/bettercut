//! The project's transition length and the machine's autosave interval.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, TransitionKind, VideoClip};
use bettercut_editor_core::{Command, Editor, SettingChange};

#[test]
fn a_new_transition_takes_the_project_length() {
    let (mut editor, _events) = Editor::new_project("House");
    assert_eq!(
        editor.project().settings.transition_length,
        TimelineTime::from_millis(500)
    );
    editor
        .dispatch(Command::ChangeSetting {
            change: SettingChange::TransitionLength(TimelineTime::from_seconds(1)),
        })
        .unwrap();
    // Held to the range: nine seconds is five.
    editor
        .dispatch(Command::ChangeSetting {
            change: SettingChange::TransitionLength(TimelineTime::from_seconds(9)),
        })
        .unwrap();
    assert_eq!(
        editor.project().settings.transition_length,
        TimelineTime::from_seconds(5)
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.project().settings.transition_length,
        TimelineTime::from_seconds(1)
    );

    // A still: its handles are endless, so a transition always has room and
    // this is a test about the length, not about the footage.
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let mut clips = Vec::new();
    for start in [0, 10] {
        let clip = VideoClip::new(media, TimelineTime::from_seconds(start), source).unwrap();
        clips.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    editor
        .set_transition(clips[0], TransitionKind::Crossfade)
        .unwrap();
    let transition = editor.video_clip(clips[0]).unwrap().transition_out.unwrap();
    assert_eq!(transition.duration, TimelineTime::from_seconds(1));
}

#[test]
fn the_autosave_interval_is_held_to_its_limits() {
    let (mut editor, _events) = Editor::new_project("Autosave");
    assert_eq!(editor.autosave_seconds(), 60);
    editor.set_autosave_seconds(120);
    assert_eq!(editor.autosave_seconds(), 120);
    editor.set_autosave_seconds(1);
    assert_eq!(editor.autosave_seconds(), 10);
    editor.set_autosave_seconds(100_000);
    assert_eq!(editor.autosave_seconds(), 600);
}
