//! From a frame on the timeline back to the file it came from
//! (`bettercut_editor_core::match_frame`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// One shot on V1: eight seconds of a minute-long file, taken from twenty
/// seconds in, with its sound on A1.
fn one_shot() -> (Editor, [ClipId; 2]) {
    let (mut editor, _events) = Editor::new_project("Match frame");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/take-3.mp4",
        MediaTime::from_seconds(60),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor
        .place_media_range(
            media,
            Some((MediaTime::from_seconds(20), MediaTime::from_seconds(28))),
        )
        .unwrap();
    (editor, [placed[0], placed[1]])
}

#[test]
fn the_playhead_finds_the_frame_it_is_looking_at() {
    let (mut editor, [picture, _sound]) = one_shot();
    editor.set_playhead(seconds(3));

    let found = editor.match_frame(&[]).expect("a frame");
    assert_eq!(
        found.clip, picture,
        "the picture is what is being looked at"
    );
    assert_eq!(
        found.at,
        MediaTime::from_seconds(23),
        "three seconds into a shot that starts twenty seconds into the file"
    );
    assert_eq!(found.source.start, MediaTime::from_seconds(20));
    assert_eq!(found.source.end, MediaTime::from_seconds(28));
}

/// Speed and direction are part of the answer: the instant returned is the
/// frame actually on screen, not the one the timeline arithmetic alone says.
#[test]
fn speed_and_reversal_are_followed() {
    let (mut editor, [picture, _sound]) = one_shot();
    editor.set_playhead(seconds(2));
    let straight = editor.match_frame(&[]).expect("a frame").at;
    assert_eq!(straight, MediaTime::from_seconds(22));

    editor
        .set_clip_speed(
            picture,
            bettercut_editor_core::foundation::Rational::new(2, 1).expect("a speed"),
            false,
        )
        .expect("faster");
    let faster = editor.match_frame(&[]).expect("a frame").at;
    assert_eq!(
        faster,
        MediaTime::from_seconds(24),
        "at twice the speed, two seconds in is four seconds of file"
    );
}

/// A selected clip the playhead is inside is the user saying which one they
/// mean, so it wins over whatever is topmost.
#[test]
fn a_selected_clip_wins_over_the_topmost_one() {
    let (mut editor, [picture, sound]) = one_shot();
    editor.set_playhead(seconds(3));

    assert_eq!(editor.match_frame(&[]).expect("a frame").clip, picture);
    assert_eq!(
        editor.match_frame(&[sound]).expect("a frame").clip,
        sound,
        "the selected sound clip should be the one matched"
    );
    // Two selected clips are no answer to "which one", so the topmost wins.
    assert_eq!(
        editor.match_frame(&[picture, sound]).expect("a frame").clip,
        picture
    );
}

/// A hidden lane is not being looked at, so it is not what gets matched.
#[test]
fn a_hidden_lane_is_not_matched() {
    let (mut editor, [picture, sound]) = one_shot();
    editor.set_playhead(seconds(3));
    let lane = editor.track_of(picture).unwrap();
    editor
        .set_track_flag(lane, bettercut_editor_core::TrackFlag::Enabled, false)
        .unwrap();

    assert_eq!(
        editor.match_frame(&[]).expect("a frame").clip,
        sound,
        "with the picture hidden, the sound is what is there"
    );
}

#[test]
fn nothing_under_the_playhead_is_no_match() {
    let (mut editor, _clips) = one_shot();
    editor.set_playhead(seconds(30));
    assert!(editor.match_frame(&[]).is_none());
}

/// A colour clip was made, not filmed: there is no file to open.
#[test]
fn a_made_picture_has_no_file_to_match() {
    let (mut editor, _events) = Editor::new_project("Colour");
    let clip = editor
        .add_colour_clip(bettercut_editor_core::media::Generated::solid([
            200, 30, 30,
        ]))
        .expect("a colour");
    editor.set_playhead(seconds(1));

    assert!(editor.match_frame(&[]).is_none());
    assert!(editor.match_frame_of(clip).is_none());
}

/// Asked about a clip the playhead is nowhere near, the answer is its
/// in-point — the start of what it plays.
#[test]
fn a_clip_away_from_the_playhead_answers_with_its_in_point() {
    let (mut editor, [picture, _sound]) = one_shot();
    editor.set_playhead(seconds(40));

    let found = editor.match_frame_of(picture).expect("a frame");
    assert_eq!(found.at, MediaTime::from_seconds(20));
}
