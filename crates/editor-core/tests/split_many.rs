//! Cutting one clip at several instants at once (§79: one intention, one undo).
//!
//! What scene detection hands the editor. The rules worth holding: every cut
//! lands, the sound is cut with the picture (§12), and the whole lot is one
//! step of history.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A ten-second video with its sound, on V1 and A1 from zero.
fn editor_with_a_shot() -> (Editor, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Scenes");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/reel.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0])
}

fn video_count(editor: &Editor) -> usize {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .len()
}

fn audio_count(editor: &Editor) -> usize {
    editor.active_sequence().unwrap().audio_tracks[0]
        .clips()
        .len()
}

#[test]
fn every_cut_lands_and_the_sound_comes_too() {
    let (mut editor, clip) = editor_with_a_shot();

    let cuts = editor
        .split_clip_at(clip, &[seconds(2), seconds(5), seconds(8)])
        .unwrap();

    assert_eq!(cuts, 3);
    assert_eq!(video_count(&editor), 4, "three cuts make four clips");
    assert_eq!(
        audio_count(&editor),
        4,
        "§12: the sound was left in one piece under four pieces of picture"
    );

    let starts: Vec<TimelineTime> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.timeline.start)
        .collect();
    assert_eq!(
        starts,
        vec![TimelineTime::ZERO, seconds(2), seconds(5), seconds(8)]
    );
}

/// The point of doing them together: one press of undo puts the clip back.
#[test]
fn all_the_cuts_are_one_undo_step() {
    let (mut editor, clip) = editor_with_a_shot();
    let before = editor.undo_depth();

    editor
        .split_clip_at(clip, &[seconds(2), seconds(5), seconds(8)])
        .unwrap();
    assert_eq!(editor.undo_depth(), before + 1, "one intention, one step");

    editor.undo().unwrap();
    assert_eq!(video_count(&editor), 1, "undo left the clip in pieces");
    assert_eq!(audio_count(&editor), 1);
}

/// Detection reports what is in the *file*; where the clip is on the timeline
/// is the editor's business. An instant outside the clip is simply not a cut.
#[test]
fn instants_outside_the_clip_are_ignored() {
    let (mut editor, clip) = editor_with_a_shot();

    let cuts = editor
        .split_clip_at(
            clip,
            &[
                TimelineTime::ZERO, // its own start edge
                seconds(4),         // inside
                seconds(10),        // its own end edge
                seconds(30),        // past the end of the timeline
                TimelineTime::from_seconds(-5),
            ],
        )
        .unwrap();

    assert_eq!(cuts, 1, "only the instant inside the clip is a cut");
    assert_eq!(video_count(&editor), 2);
}

/// The same instant twice is one cut — including when the repeat is not next
/// to the original, which is what detection over two passes would produce.
#[test]
fn the_same_instant_twice_is_one_cut() {
    let (mut editor, clip) = editor_with_a_shot();
    let cuts = editor
        .split_clip_at(clip, &[seconds(3), seconds(7), seconds(3)])
        .unwrap();

    assert_eq!(cuts, 2, "a repeated instant made a zero-length clip");
    assert_eq!(video_count(&editor), 3);
    // The step in the history has to describe what actually happened: an undo
    // button offering to undo a cut that was never made is a lie about the
    // project.
    assert_eq!(
        editor.undo_label().as_deref(),
        Some("Split into 3 Clips"),
        "the history counted a repeated instant as a piece"
    );
}

#[test]
fn nothing_to_cut_changes_nothing() {
    let (mut editor, clip) = editor_with_a_shot();
    let before = editor.undo_depth();

    assert_eq!(editor.split_clip_at(clip, &[]).unwrap(), 0);
    assert_eq!(editor.split_clip_at(clip, &[seconds(20)]).unwrap(), 0);
    assert_eq!(video_count(&editor), 1);
    assert_eq!(
        editor.undo_depth(),
        before,
        "an edit that cut nothing still went into the history"
    );
}
