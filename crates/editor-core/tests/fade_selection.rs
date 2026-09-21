//! Fading the selection in one click (`Editor::fade_clips`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::fade_selection::FadeEnds;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::MotionKind;

fn millis(n: i64) -> TimelineTime {
    TimelineTime::from_millis(n)
}

/// Two shots with sound: 4 s and 1 s.
fn shots() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Fades");
    let mut place = |length| {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{length}.mp4"),
            MediaTime::from_seconds(length),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap()[0]
    };
    let long = place(4);
    let short = place(1);
    (editor, long, short)
}

fn sound_of(editor: &Editor, picture: ClipId) -> ClipId {
    editor
        .linked_with(picture)
        .into_iter()
        .find(|c| *c != picture)
        .unwrap()
}

#[test]
fn both_ends_fade_with_their_sound_in_one_step() {
    let (mut editor, long, short) = shots();
    let depth = editor.undo_depth();

    let changed = editor
        .fade_clips(&[long, short], FadeEnds::Both, millis(1000))
        .unwrap();
    assert_eq!(changed, 4, "two pictures and two sounds");
    assert_eq!(editor.undo_depth(), depth + 1);

    let motion = editor.video_clip(long).unwrap().motion;
    assert_eq!(motion.intro.unwrap().kind, MotionKind::Fade);
    assert_eq!(motion.intro.unwrap().duration, millis(1000));
    assert_eq!(motion.outro.unwrap().duration, millis(1000));
    let sound = editor.audio_clip(sound_of(&editor, long)).unwrap();
    assert_eq!(
        (sound.fade_in, sound.fade_out),
        (millis(1000), millis(1000))
    );

    // A one-second clip can only fade half a second each way.
    let short_motion = editor.video_clip(short).unwrap().motion;
    assert_eq!(short_motion.intro.unwrap().duration, millis(500));
    let short_sound = editor.audio_clip(sound_of(&editor, short)).unwrap();
    assert_eq!(short_sound.fade_out, millis(500));

    editor.undo().unwrap();
    assert!(editor.video_clip(long).unwrap().motion.intro.is_none());
}

#[test]
fn one_end_leaves_the_other_and_remove_takes_both_off() {
    let (mut editor, long, _) = shots();
    editor
        .fade_clips(&[long], FadeEnds::Out, millis(2000))
        .unwrap();
    let motion = editor.video_clip(long).unwrap().motion;
    assert!(motion.intro.is_none());
    assert_eq!(motion.outro.unwrap().duration, millis(2000));

    editor
        .fade_clips(&[long], FadeEnds::In, millis(500))
        .unwrap();
    let motion = editor.video_clip(long).unwrap().motion;
    assert_eq!(motion.intro.unwrap().duration, millis(500));
    assert_eq!(
        motion.outro.unwrap().duration,
        millis(2000),
        "the out fade went"
    );

    assert_eq!(
        editor
            .fade_clips(&[long], FadeEnds::In, millis(500))
            .unwrap(),
        0,
        "the same fade again changed something"
    );

    editor
        .fade_clips(&[long], FadeEnds::Neither, TimelineTime::ZERO)
        .unwrap();
    let motion = editor.video_clip(long).unwrap().motion;
    assert!(motion.intro.is_none() && motion.outro.is_none());
    let sound = editor.audio_clip(sound_of(&editor, long)).unwrap();
    assert_eq!(
        (sound.fade_in, sound.fade_out),
        (TimelineTime::ZERO, TimelineTime::ZERO)
    );
}
