//! Track colours (`Track::color_label`): any kind of lane takes one, as one
//! step, and none by default.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TrackId;
use bettercut_editor_core::timeline::ColorLabel;

fn lanes() -> (Editor, TrackId, TrackId, TrackId) {
    let (editor, _events) = Editor::new_project("Colours");
    let sequence = editor.active_sequence().unwrap();
    let lanes = (
        sequence.video_tracks[0].id,
        sequence.audio_tracks[0].id,
        sequence.text_tracks[0].id,
    );
    (editor, lanes.0, lanes.1, lanes.2)
}

#[test]
fn every_kind_of_lane_takes_a_colour_as_one_step() {
    let (mut editor, picture, sound, titles) = lanes();
    for lane in [picture, sound, titles] {
        assert_eq!(editor.track_colour(lane), ColorLabel::None);
    }
    let depth = editor.undo_depth();

    editor.set_track_colour(picture, ColorLabel::Green).unwrap();
    editor.set_track_colour(sound, ColorLabel::Blue).unwrap();
    editor.set_track_colour(titles, ColorLabel::Pink).unwrap();
    assert_eq!(editor.track_colour(picture), ColorLabel::Green);
    assert_eq!(editor.track_colour(sound), ColorLabel::Blue);
    assert_eq!(editor.track_colour(titles), ColorLabel::Pink);
    assert_eq!(editor.undo_depth(), depth + 3);

    editor.undo().unwrap();
    assert_eq!(editor.track_colour(titles), ColorLabel::None);
    assert_eq!(editor.track_colour(sound), ColorLabel::Blue);
}

#[test]
fn a_lane_that_is_not_there_is_refused() {
    let (mut editor, _, _, _) = lanes();
    assert!(
        editor
            .set_track_colour(TrackId::new(), ColorLabel::Red)
            .is_err()
    );
    assert_eq!(editor.track_colour(TrackId::new()), ColorLabel::None);
}
