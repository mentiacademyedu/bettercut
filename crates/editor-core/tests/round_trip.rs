//! Everything the editor can put in a project, saved and opened again.
//!
//! One test rather than one per feature, because the failure it guards against
//! is *silent*: a field added to the model but not to the file format loads as
//! its default, and the user's work is quietly gone. The project is built
//! through the editor's own commands, so anything a user can do is covered by
//! adding it here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, Rational, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Motion, MotionKind, TextAnimation, TransitionKind};
use bettercut_editor_core::{ClipProperty, Editor, Movement, TextProperty};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A project using every feature that has somewhere to be stored.
fn elaborate_project(path: &std::path::Path) {
    let (mut editor, _events) = Editor::new_project("Everything");

    let mut video = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(30),
    );
    video.audio_codec = Some("aac".to_owned());
    let video = editor.import_media(video);
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/p.jpg",
        MediaTime::ZERO,
    ));

    let placed = editor.place_media(video).unwrap();
    let (picture, sound) = (placed[0], placed[1]);
    editor.place_media(photo).unwrap();

    // Clip properties, a look, and a movement.
    editor
        .set_clip_property(picture, ClipProperty::Opacity(0.8), false)
        .unwrap();
    editor
        .set_color_adjust(
            Some(picture),
            bettercut_editor_core::timeline::ColorAdjust {
                brightness: 1.1,
                contrast: 0.9,
                saturation: 0.7,
            },
        )
        .unwrap();
    editor.set_movement(picture, Movement::ZoomIn).unwrap();
    editor
        .set_clip_speed(picture, Rational::new(3, 2).unwrap(), false)
        .unwrap();

    // Sound: gain, fades, and the track's own volume and pan.
    editor
        .set_clip_property(sound, ClipProperty::Gain(0.6), false)
        .unwrap();
    editor
        .set_clip_fades(sound, TimelineTime::from_millis(400), seconds(1), false)
        .unwrap();
    let audio_track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .set_track_mix(audio_track, 0.75, -0.3, false)
        .unwrap();

    // A volume envelope on the sound (§24 on §20a.4's clip-gain stage).
    editor
        .set_gain_envelope(
            sound,
            &[(seconds(0), 1.0), (seconds(4), 0.25), (seconds(8), 1.0)],
            false,
        )
        .unwrap();

    // A transition on the cut between the video and the photo.
    editor
        .set_transition(picture, TransitionKind::FadeThroughBlack)
        .unwrap();

    // A title with an entrance and an exit.
    editor.set_playhead(seconds(2));
    let title = editor.add_text("Hello").unwrap();
    editor
        .set_text_property(
            title,
            TextProperty::Animation(TextAnimation {
                intro: Some(Motion::new(MotionKind::Pop, TimelineTime::from_millis(500))),
                outro: Some(Motion::new(
                    MotionKind::Typewriter,
                    TimelineTime::from_millis(700),
                )),
            }),
            false,
        )
        .unwrap();

    // Markers, a whole-video volume, and a held frame.
    editor.add_markers(&[seconds(3), seconds(9)]).unwrap();
    editor
        .set_sequence_value(ClipProperty::Gain(0.5), false)
        .unwrap();
    editor.set_playhead(seconds(5));
    editor.freeze_frame(picture, seconds(2)).unwrap();

    editor.save_as(path).unwrap();
}

#[test]
fn every_feature_survives_a_save_and_an_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("everything.vproj");
    elaborate_project(&path);

    let saved = {
        let (editor, _events) = Editor::open(&path).unwrap();
        editor.project().clone()
    };
    // Saving what was loaded and loading it again must give the same project:
    // anything that did not survive the first trip is already gone, so this
    // compares the file against itself as well as against the editor.
    let again = {
        let (mut editor, _events) = Editor::open(&path).unwrap();
        let second = dir.path().join("again.vproj");
        editor.save_as(&second).unwrap();
        let (editor, _events) = Editor::open(&second).unwrap();
        editor.project().clone()
    };
    assert_eq!(saved.active(), again.active(), "a second trip changed it");

    let sequence = saved.active().expect("a sequence");

    // Clip look, speed, transition, and the held frame.
    let clips = sequence.video_tracks[0].clips();
    let first = &clips[0];
    assert_eq!(first.opacity, 0.8);
    assert_eq!(first.color.saturation, 0.7);
    assert_eq!(first.speed, Rational::new(3, 2).unwrap());
    assert!(
        first
            .keyframes
            .is_animated(bettercut_editor_core::timeline::AnimatedParameter::ScaleX),
        "the movement's keyframes were lost"
    );
    assert_eq!(
        clips.iter().filter(|c| c.frozen).count(),
        1,
        "the hold was lost"
    );
    assert!(
        clips.iter().any(|c| c.transition_out.is_some()),
        "the transition was lost"
    );

    // Sound, and the track it is on. The freeze split the sound in two, and a
    // fade belongs to the edge it is on (§25's rule for transitions): the fade
    // in stayed with the first half, the fade out with the last.
    let sounds = sequence.audio_tracks[0].clips();
    assert_eq!(sounds[0].gain, 0.6);
    assert_eq!(sounds[0].fade_in, TimelineTime::from_millis(400));
    assert_eq!(sounds[0].fade_out, TimelineTime::ZERO);
    assert_eq!(sounds.last().unwrap().fade_out, seconds(1));
    assert_eq!(sequence.audio_tracks[0].gain, 0.75);
    assert!(
        sounds[0]
            .keyframes
            .is_animated(bettercut_editor_core::timeline::AnimatedParameter::Gain),
        "the volume envelope was lost"
    );
    assert_eq!(
        sounds[0].gain_at(TimelineTime::from_seconds(4)),
        0.25,
        "the envelope came back at the wrong level"
    );
    assert_eq!(sequence.audio_tracks[0].pan, -0.3);

    // The title and its animation.
    let title = &sequence.text_tracks[0].clips()[0];
    assert_eq!(title.text, "Hello");
    assert_eq!(
        title.animation.intro.map(|m| m.kind),
        Some(MotionKind::Pop),
        "the title's entrance was lost"
    );
    assert_eq!(
        title.animation.outro.map(|m| m.duration),
        Some(TimelineTime::from_millis(700))
    );

    // Sequence-level state.
    assert_eq!(sequence.markers.len(), 2, "markers were lost");
    assert_eq!(sequence.master_volume, 0.5);
}
