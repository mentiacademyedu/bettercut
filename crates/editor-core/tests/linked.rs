//! Linked clips (§12).
//!
//! A video file is a picture clip on a video track and a sound clip on an audio
//! track. What makes them one *thing* is a shared link, and every edit that
//! would otherwise pull them apart has to respect it: move, trim, split, delete.
//! Unlinking is how the user says they want them apart on purpose.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, TrimEdge};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A ten-second video with sound, placed at the start: picture on V1, sound on
/// A1, linked.
fn editor_with_a_linked_pair() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Linked");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);

    let placed = editor.place_media(media).unwrap();
    assert_eq!(placed.len(), 2, "setup: a video with sound is two clips");
    (editor, placed[0], placed[1])
}

fn range_of(editor: &Editor, clip: ClipId) -> (TimelineTime, TimelineTime) {
    let sequence = editor.active_sequence().unwrap();
    sequence
        .video_tracks
        .iter()
        .find_map(|t| t.get(clip).map(|c| (c.timeline.start, c.timeline.end)))
        .or_else(|| {
            sequence
                .audio_tracks
                .iter()
                .find_map(|t| t.get(clip).map(|c| (c.timeline.start, c.timeline.end)))
        })
        .expect("clip is on the timeline")
}

#[test]
fn placing_a_video_links_its_two_halves() {
    let (editor, picture, sound) = editor_with_a_linked_pair();

    assert!(editor.link_of(picture).is_some());
    assert_eq!(editor.link_of(picture), editor.link_of(sound));
    let mut linked = editor.linked_with(picture);
    linked.sort();
    let mut expected = vec![picture, sound];
    expected.sort();
    assert_eq!(linked, expected);
}

/// Dragging the picture drags the sound. Without this, the first move of any
/// imported video puts its sound out of sync.
#[test]
fn moving_the_picture_moves_the_sound() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    let track = editor.track_of(picture).unwrap();

    editor.move_clip(track, track, picture, seconds(5)).unwrap();

    assert_eq!(range_of(&editor, picture), (seconds(5), seconds(15)));
    assert_eq!(
        range_of(&editor, sound),
        (seconds(5), seconds(15)),
        "the sound stayed behind"
    );
}

/// The link is symmetric: selecting the audio clip is just as likely.
#[test]
fn moving_the_sound_moves_the_picture() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    let track = editor.track_of(sound).unwrap();

    editor.move_clip(track, track, sound, seconds(3)).unwrap();
    assert_eq!(range_of(&editor, picture).0, seconds(3));
}

/// One drag, one undo (§79).
#[test]
fn a_linked_move_is_a_single_undo_step() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    let track = editor.track_of(picture).unwrap();
    editor.move_clip(track, track, picture, seconds(5)).unwrap();

    editor.undo().unwrap();

    assert_eq!(range_of(&editor, picture).0, TimelineTime::ZERO);
    assert_eq!(
        range_of(&editor, sound).0,
        TimelineTime::ZERO,
        "one undo left the sound where it had been moved to"
    );
}

/// All or nothing. If the sound cannot go where the picture is going, neither
/// does — a half-applied move is the out-of-sync state this exists to prevent.
#[test]
fn a_move_the_partner_cannot_make_is_refused_entirely() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();

    // Music on A1 from 20 s, where the sound would land.
    let mut music = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/m.mp3",
        MediaTime::from_seconds(30),
    );
    music.audio_codec = Some("mp3".to_owned());
    let music = editor.import_media(music);
    let audio_track = editor.track_of(sound).unwrap();
    let music_clip = bettercut_editor_core::timeline::AudioClip::new(
        music,
        seconds(20),
        bettercut_editor_core::timeline::SourceRange::new(
            MediaTime::ZERO,
            MediaTime::from_seconds(30),
        )
        .unwrap(),
    )
    .unwrap();
    editor
        .add_clip(
            audio_track,
            bettercut_editor_core::ClipPayload::Audio(Box::new(music_clip)),
        )
        .unwrap();

    let video_track = editor.track_of(picture).unwrap();
    assert!(
        editor
            .move_clip(video_track, video_track, picture, seconds(20))
            .is_err(),
        "the picture moved although its sound could not follow"
    );
    assert_eq!(
        range_of(&editor, picture).0,
        TimelineTime::ZERO,
        "picture moved"
    );
    assert_eq!(
        range_of(&editor, sound).0,
        TimelineTime::ZERO,
        "sound moved"
    );
}

#[test]
fn trimming_the_picture_trims_the_sound() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    let track = editor.track_of(picture).unwrap();

    editor
        .trim_clip(track, picture, TrimEdge::End, seconds(6))
        .unwrap();
    assert_eq!(
        range_of(&editor, sound).1,
        seconds(6),
        "the sound's end did not follow"
    );

    editor
        .trim_clip(track, picture, TrimEdge::Start, seconds(2))
        .unwrap();
    assert_eq!(
        range_of(&editor, sound).0,
        seconds(2),
        "the sound's start did not follow"
    );

    editor.undo().unwrap();
    assert_eq!(range_of(&editor, sound).0, TimelineTime::ZERO);
    assert_eq!(range_of(&editor, picture).0, TimelineTime::ZERO);
}

/// A split cuts both halves, and — the subtle part — pairs them up again. A
/// split clones the clip, so left and right would otherwise share one link and
/// four clips would move together where two pairs were meant.
#[test]
fn splitting_a_linked_pair_gives_two_linked_pairs() {
    let (mut editor, picture, _) = editor_with_a_linked_pair();
    editor.set_playhead(seconds(4));

    let cut = editor.split_at_playhead(&[picture]).unwrap();
    assert_eq!(cut, 2, "selecting the picture should cut its sound too");

    let sequence = editor.active_sequence().unwrap();
    let video = sequence.video_tracks[0].clips();
    let audio = sequence.audio_tracks[0].clips();
    assert_eq!((video.len(), audio.len()), (2, 2));

    // Left with left, right with right, and the two pairs distinct.
    assert!(video[0].link.is_some());
    assert_eq!(
        video[0].link, audio[0].link,
        "the left halves are not paired"
    );
    assert_eq!(
        video[1].link, audio[1].link,
        "the right halves are not paired"
    );
    assert_ne!(
        video[0].link, video[1].link,
        "the two halves still share one link — moving either would move all four"
    );
}

/// And the consequence that matters: after the split, moving the right half
/// moves only the right half's sound.
#[test]
fn after_a_split_each_half_moves_with_its_own_sound() {
    let (mut editor, picture, _) = editor_with_a_linked_pair();
    editor.set_playhead(seconds(4));
    editor.split_at_playhead(&[picture]).unwrap();

    let sequence = editor.active_sequence().unwrap();
    let right_picture = sequence.video_tracks[0].clips()[1].id;
    let left_sound = sequence.audio_tracks[0].clips()[0].id;
    let right_sound = sequence.audio_tracks[0].clips()[1].id;
    let track = editor.track_of(right_picture).unwrap();

    editor
        .move_clip(track, track, right_picture, seconds(20))
        .unwrap();

    assert_eq!(range_of(&editor, right_sound).0, seconds(20));
    assert_eq!(
        range_of(&editor, left_sound).0,
        TimelineTime::ZERO,
        "the left half's sound moved with the right half"
    );
}

/// Undoing the split puts back one pair with its original link.
#[test]
fn undoing_a_split_restores_the_original_link() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    let original = editor.link_of(picture);
    editor.set_playhead(seconds(4));
    editor.split_at_playhead(&[picture]).unwrap();

    editor.undo().unwrap();

    assert_eq!(editor.link_of(picture), original);
    assert_eq!(editor.link_of(sound), original);
}

/// Unlinking is how the user says they want the two apart on purpose — to lay
/// a different sound under the picture, or move a line of dialogue.
#[test]
fn an_unlinked_pair_moves_independently() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    editor.unlink(picture).unwrap();

    assert_eq!(editor.link_of(picture), None);
    assert_eq!(editor.link_of(sound), None, "only one side was unlinked");

    let track = editor.track_of(picture).unwrap();
    editor.move_clip(track, track, picture, seconds(5)).unwrap();
    assert_eq!(
        range_of(&editor, sound).0,
        TimelineTime::ZERO,
        "the sound followed after being unlinked"
    );
}

#[test]
fn unlinking_is_undoable() {
    let (mut editor, picture, sound) = editor_with_a_linked_pair();
    let original = editor.link_of(picture);
    editor.unlink(sound).unwrap();

    editor.undo().unwrap();

    assert_eq!(editor.link_of(picture), original);
    assert_eq!(editor.link_of(sound), original);
}

/// Unlinking something that is not linked is refused rather than recorded as an
/// undo step that undoes nothing.
#[test]
fn unlinking_an_unlinked_clip_is_refused() {
    let (mut editor, picture, _) = editor_with_a_linked_pair();
    editor.unlink(picture).unwrap();
    assert!(editor.unlink(picture).is_err());
}

/// §38.2: links survive a save and load, or a project reopened tomorrow has
/// every video's sound quietly detached.
#[test]
fn links_survive_a_save_and_load() {
    let (editor, picture, sound) = editor_with_a_linked_pair();
    let json = serde_json::to_string(editor.project()).unwrap();
    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).unwrap();

    let sequence = &loaded.sequences[0];
    let video_link = sequence.video_tracks[0].get(picture).unwrap().link;
    let audio_link = sequence.audio_tracks[0].get(sound).unwrap().link;
    assert!(video_link.is_some());
    assert_eq!(video_link, audio_link);
}
