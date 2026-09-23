//! Evening out loudness across clips (`Editor::match_clips_loudness`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, SourceRange};

fn two() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Even");
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap();
    let a = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
    let b = AudioClip::new(MediaId::new(), TimelineTime::from_seconds(2), source).unwrap();
    let (ia, ib) = (a.id, b.id);
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(a)))
        .unwrap();
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(b)))
        .unwrap();
    (editor, ia, ib)
}

#[test]
fn a_quiet_clip_comes_up_and_a_loud_one_down_as_one_step() {
    let (mut editor, quiet, loud) = two();
    let depth = editor.undo_depth();
    // Six dB apart, brought to the point between them.
    let changed = editor
        .match_clips_loudness(&[(quiet, -26.0), (loud, -20.0)], -23.0)
        .unwrap();
    assert_eq!(changed, 2);
    let up = editor.audio_clip(quiet).unwrap().gain;
    let down = editor.audio_clip(loud).unwrap().gain;
    assert!((up - 10f32.powf(3.0 / 20.0)).abs() < 1e-3, "{up}");
    assert!((down - 10f32.powf(-3.0 / 20.0)).abs() < 1e-3, "{down}");
    assert_eq!(editor.undo_depth(), depth + 1);

    // Already there: nothing to do, no step.
    assert_eq!(
        editor
            .match_clips_loudness(&[(quiet, -23.0), (loud, -23.0)], -23.0)
            .unwrap(),
        0
    );
    editor.undo().unwrap();
    assert!((editor.audio_clip(quiet).unwrap().gain - 1.0).abs() < 1e-6);
}
