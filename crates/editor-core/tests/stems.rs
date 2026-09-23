//! A stem's copy plays one sound lane (`stems::isolate_sound_lane`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TrackId;
use bettercut_editor_core::stems::isolate_sound_lane;

#[test]
fn only_the_kept_lane_is_heard_whatever_its_switches_said() {
    let (mut editor, _events) = Editor::new_project("Stems");
    editor.add_audio_track("A2").unwrap();
    let (mut project, sequence) = editor.export_copy(None).unwrap();
    let (a1, a2) = {
        let s = project.sequence(sequence).unwrap();
        (s.audio_tracks[0].id, s.audio_tracks[1].id)
    };
    // The kept lane muted, the other soloed: the stem ignores both.
    {
        let s = project.sequence_mut(sequence).unwrap();
        s.audio_tracks[1].enabled = false;
        s.audio_tracks[0].solo = true;
    }
    isolate_sound_lane(&mut project, sequence, a2).unwrap();
    let s = project.sequence(sequence).unwrap();
    assert!(s.audio_tracks.iter().find(|t| t.id == a2).unwrap().enabled);
    assert!(!s.audio_tracks.iter().find(|t| t.id == a1).unwrap().enabled);
    assert!(s.audio_tracks.iter().all(|t| !t.solo));

    assert!(isolate_sound_lane(&mut project, sequence, TrackId::new()).is_err());
}
