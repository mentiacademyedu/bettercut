//! Sequence resolution and frame rate (§8, §9, §36).
//!
//! The rate is the risky half. Positions are absolute ticks, so changing it
//! must not move a single clip — if it ever did, every cut in a project would
//! shift the moment someone corrected the sequence format.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Resolution, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Command, Editor};

fn editor_with_clip() -> (Editor, TimelineTime, TimelineTime) {
    let (mut editor, _rx) = Editor::new_project("Format");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(600),
    ));
    let source =
        SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(14)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::from_seconds(3), source).unwrap();
    let (start, end) = (clip.timeline.start, clip.timeline.end);
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    (editor, start, end)
}

#[test]
fn setting_the_format_changes_both_together() {
    let (mut editor, _, _) = editor_with_clip();

    editor
        .set_sequence_format(Resolution::VERTICAL_1080, FrameRate::PAL_25)
        .expect("valid format");

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.resolution, Resolution::VERTICAL_1080);
    assert_eq!(sequence.frame_rate, FrameRate::PAL_25);
    assert_eq!(sequence.ticks_per_frame(), 38_400, "25 fps per §9's table");
}

/// The whole point: a rate change must not shift anything.
#[test]
fn changing_the_rate_does_not_move_clips() {
    let (mut editor, start, end) = editor_with_clip();

    for rate in FrameRate::SUPPORTED {
        editor
            .set_sequence_format(Resolution::HD_1080, rate)
            .expect("every supported rate is settable");

        let sequence = editor.active_sequence().unwrap();
        let clip = sequence.video_tracks[0].clips().first().expect("the clip");
        assert_eq!(
            (clip.timeline.start, clip.timeline.end),
            (start, end),
            "clip moved when the sequence changed to {rate} fps"
        );
    }
}

/// One user decision, one undo step — and undo restores both halves.
#[test]
fn the_format_change_undoes_as_one_step() {
    let (mut editor, _, _) = editor_with_clip();
    let before = {
        let s = editor.active_sequence().unwrap();
        (s.resolution, s.frame_rate)
    };

    editor
        .set_sequence_format(Resolution::new(3840, 2160), FrameRate::FPS_60)
        .expect("valid");
    editor.undo().expect("undo");

    let after = {
        let s = editor.active_sequence().unwrap();
        (s.resolution, s.frame_rate)
    };
    assert_eq!(
        after, before,
        "undo did not restore both resolution and rate"
    );
}

/// §9: a rate that does not divide 960,000 exactly is refused, not rounded.
/// 100 fps divides exactly (9,600); 7 fps does not.
#[test]
fn an_unrepresentable_rate_is_refused_and_changes_nothing() {
    let (mut editor, _, _) = editor_with_clip();
    let before = {
        let s = editor.active_sequence().unwrap();
        (s.resolution, s.frame_rate)
    };

    let seven = FrameRate::new(7, 1).expect("a valid rational");
    assert!(
        bettercut_editor_core::foundation::ticks_per_frame(seven).is_none(),
        "7 fps should not divide the timebase exactly"
    );

    // The fixture already added a clip, so there is undo history either way.
    // What matters is that the *failed* command did not add to it.
    let undo_before = editor.undo_label();

    let err = editor.set_sequence_format(Resolution::HD_1080, seven);
    assert!(err.is_err(), "an unrepresentable rate was accepted");

    let after = {
        let s = editor.active_sequence().unwrap();
        (s.resolution, s.frame_rate)
    };
    assert_eq!(
        after, before,
        "a rejected change still mutated the sequence"
    );
    assert_eq!(
        editor.undo_label(),
        undo_before,
        "a failed command was pushed onto the undo history"
    );
}

#[test]
fn a_zero_dimension_is_refused() {
    let (mut editor, _, _) = editor_with_clip();
    assert!(
        editor
            .set_sequence_format(Resolution::new(1920, 0), FrameRate::FPS_30)
            .is_err()
    );
}

// ---- adopting the format from an import ---------------------------------

fn video(width: u32, height: u32, rate: FrameRate) -> MediaAsset {
    MediaAsset::new(
        MediaKind::Video,
        "C:/media/clip.mp4",
        MediaTime::from_seconds(30),
    )
    .with_video(width, height, rate)
}

/// §8: the first video imported into an empty sequence sets the format, so 25
/// fps footage does not land on a 30 fps grid.
#[test]
fn the_first_import_sets_the_format() {
    let (mut editor, _rx) = Editor::new_project("Adopt");
    assert_eq!(
        editor.active_sequence().unwrap().frame_rate,
        FrameRate::FPS_30,
        "a new project starts at 30"
    );

    let id = editor.import_media(video(1080, 1920, FrameRate::PAL_25));
    let adopted = editor.adopt_format_from(id).expect("should adopt");

    assert_eq!(adopted, (Resolution::VERTICAL_1080, FrameRate::PAL_25));
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.frame_rate, FrameRate::PAL_25);
    assert_eq!(sequence.resolution, Resolution::VERTICAL_1080);
}

/// But not once the user has started editing — by then the format is theirs.
#[test]
fn a_later_import_does_not_change_the_format() {
    let (mut editor, start, _) = editor_with_clip();
    let before = editor.active_sequence().unwrap().frame_rate;

    let id = editor.import_media(video(1080, 1920, FrameRate::PAL_25));
    assert!(
        editor.adopt_format_from(id).is_none(),
        "the format was changed under a timeline that already had clips"
    );
    assert_eq!(editor.active_sequence().unwrap().frame_rate, before);

    let clip = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .first()
        .expect("clip");
    assert_eq!(clip.timeline.start, start);
}

#[test]
fn audio_and_stills_do_not_set_the_format() {
    let (mut editor, _rx) = Editor::new_project("Adopt");
    let audio = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/track.wav",
        MediaTime::from_seconds(30),
    ));
    assert!(editor.adopt_format_from(audio).is_none());

    let still = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/logo.png",
        MediaTime::ZERO,
    ));
    assert!(editor.adopt_format_from(still).is_none());

    assert_eq!(
        editor.active_sequence().unwrap().frame_rate,
        FrameRate::FPS_30
    );
}

/// A camera reporting a rate the timebase cannot represent must not drag the
/// sequence onto a drifting grid — better an honest 30 than an inexact 7.
#[test]
fn an_unrepresentable_source_rate_is_not_adopted() {
    let (mut editor, _rx) = Editor::new_project("Adopt");
    let seven = FrameRate::new(7, 1).expect("valid rational");
    let id = editor.import_media(video(1920, 1080, seven));

    assert!(editor.adopt_format_from(id).is_none());
    assert_eq!(
        editor.active_sequence().unwrap().frame_rate,
        FrameRate::FPS_30
    );
}

/// §38.2: the journal replays commands after a crash, so this one has to
/// survive a round trip through JSON.
#[test]
fn the_command_round_trips_through_json() {
    let (editor, _, _) = editor_with_clip();
    let command = Command::SetSequenceFormat {
        sequence: editor.active_sequence().unwrap().id,
        resolution: Resolution::VERTICAL_1080.into(),
        frame_rate: FrameRate::NTSC_29_97,
    };

    let json = serde_json::to_string(&command).expect("serialize");
    assert!(
        json.contains("set_sequence_format"),
        "unexpected tag: {json}"
    );
    assert_eq!(
        serde_json::from_str::<Command>(&json).expect("deserialize"),
        command
    );
}
