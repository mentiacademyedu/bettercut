//! Captions through the editor (§27, Milestone 10).
//!
//! The formats are tested in the captions crate. What matters here is what
//! happens to the timeline: that an import is one undo step, that it lands on a
//! lane of its own, that re-importing replaces rather than piles up, and that
//! what goes out is what came in.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use bettercut_editor_core::foundation::{TimelineTime, TrackId};
use bettercut_editor_core::{Editor, TextProperty};

fn editor() -> Editor {
    let (editor, _rx) = Editor::new_project("Captions");
    editor
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("bettercut-caption-editor-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// Three cues, a second apart.
fn write_srt(name: &str) -> PathBuf {
    let path = temp(name);
    std::fs::write(
        &path,
        "1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n\
         2\n00:00:03,000 --> 00:00:04,000\nsecond\n\n\
         3\n00:00:05,000 --> 00:00:06,000\nthird\n",
    )
    .expect("write");
    path
}

fn caption_track(editor: &Editor) -> Option<TrackId> {
    editor
        .active_sequence()?
        .text_tracks
        .iter()
        .find(|t| t.name == Editor::CAPTION_TRACK)
        .map(|t| t.id)
}

#[test]
fn importing_puts_every_cue_on_the_timeline() {
    let mut editor = editor();
    let count = editor.import_captions(&write_srt("basic.srt")).unwrap();
    assert_eq!(count, 3);

    let track = caption_track(&editor).expect("no caption lane");
    let track = editor
        .active_sequence()
        .unwrap()
        .text_track(track)
        .expect("lane");

    assert_eq!(track.len(), 3);
    assert_eq!(track.clips()[0].text, "first");
    assert_eq!(
        track.clips()[0].timeline.start,
        TimelineTime::from_seconds(1)
    );
    assert_eq!(track.clips()[2].timeline.end, TimelineTime::from_seconds(6));
}

/// Captions get a lane of their own. Dropped among someone's titles they would
/// either collide with them or scatter into the gaps between them.
#[test]
fn captions_land_on_a_lane_of_their_own() {
    let mut editor = editor();
    let title = editor.add_text("A title").unwrap();
    let title_track = editor.active_sequence().unwrap().text_tracks[0].id;

    editor.import_captions(&write_srt("own-lane.srt")).unwrap();

    let captions = caption_track(&editor).expect("no caption lane");
    assert_ne!(captions, title_track, "captions landed on the title lane");

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.text_track(title_track).unwrap().len(),
        1,
        "the title lane was disturbed"
    );
    assert!(editor.text_clip(title).is_some(), "the title was lost");
}

/// "Undo the import" is the only thing anyone means after a bad file — not
/// "undo one of the forty captions".
#[test]
fn an_import_is_a_single_undo_step() {
    let mut editor = editor();
    editor.import_captions(&write_srt("undo.srt")).unwrap();

    editor.undo().unwrap();

    let sequence = editor.active_sequence().unwrap();
    let captions: usize = sequence.text_tracks.iter().map(|t| t.len()).sum();
    assert_eq!(captions, 0, "one undo did not take the whole import back");

    editor.redo().unwrap();
    let sequence = editor.active_sequence().unwrap();
    let captions: usize = sequence.text_tracks.iter().map(|t| t.len()).sum();
    assert_eq!(captions, 3, "redo did not put the import back");
}

/// Importing a corrected file over an old one is the common case. Appending
/// would leave both, overlapping, and the model would refuse half of them.
#[test]
fn re_importing_replaces_rather_than_piling_up() {
    let mut editor = editor();
    editor
        .import_captions(&write_srt("first-pass.srt"))
        .unwrap();

    let corrected = temp("second-pass.srt");
    std::fs::write(&corrected, "1\n00:00:01,000 --> 00:00:02,000\ncorrected\n").expect("write");
    let count = editor.import_captions(&corrected).unwrap();

    assert_eq!(count, 1);
    let track = caption_track(&editor).unwrap();
    let track = editor.active_sequence().unwrap().text_track(track).unwrap();
    assert_eq!(track.len(), 1, "the old captions were left behind");
    assert_eq!(track.clips()[0].text, "corrected");
}

/// And that replacement is undoable in one step too, back to the old captions.
#[test]
fn undoing_a_re_import_restores_the_previous_captions() {
    let mut editor = editor();
    editor.import_captions(&write_srt("before.srt")).unwrap();

    let corrected = temp("after.srt");
    std::fs::write(&corrected, "1\n00:00:01,000 --> 00:00:02,000\nnew\n").expect("write");
    editor.import_captions(&corrected).unwrap();

    editor.undo().unwrap();

    let track = caption_track(&editor).unwrap();
    let track = editor.active_sequence().unwrap().text_track(track).unwrap();
    assert_eq!(track.len(), 3, "undo did not restore the old captions");
    assert_eq!(track.clips()[0].text, "first");
}

/// Imported captions are styled as captions, not as titles: readable over
/// footage that changes underneath them, and low in the frame.
#[test]
fn imported_captions_are_styled_and_placed_as_captions() {
    let mut editor = editor();
    editor.import_captions(&write_srt("styled.srt")).unwrap();

    let track = caption_track(&editor).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let clip = &sequence.text_track(track).unwrap().clips()[0];

    assert!(
        clip.style.background.is_some(),
        "a caption with no panel behind it is unreadable over moving footage"
    );
    assert!(clip.style.wrap_width.is_some(), "long lines would run off");
    assert!(
        clip.transform.position.y > 0.2,
        "captions belong low in the frame, not across the middle"
    );
}

#[test]
fn a_file_with_nothing_in_it_is_refused() {
    let mut editor = editor();
    let empty = temp("empty.srt");
    std::fs::write(&empty, "").expect("write");

    assert!(editor.import_captions(&empty).is_err());
    let sequence = editor.active_sequence().unwrap();
    let captions: usize = sequence.text_tracks.iter().map(|t| t.len()).sum();
    assert_eq!(captions, 0, "a failed import left something behind");
}

/// Overlapping cues are common in files converted between frame rates, and §8's
/// tracks refuse overlap. The reconciliation happens before the model sees
/// them, so an import never half-succeeds.
#[test]
fn overlapping_cues_do_not_break_the_import() {
    let mut editor = editor();
    let overlapping = temp("overlapping.srt");
    std::fs::write(
        &overlapping,
        "1\n00:00:01,000 --> 00:00:05,000\nlong\n\n\
         2\n00:00:03,000 --> 00:00:07,000\noverlapping\n\n\
         3\n00:00:02,000 --> 00:00:02,500\nnested\n",
    )
    .expect("write");

    let count = editor.import_captions(&overlapping).unwrap();
    assert!(count > 0);

    let track = caption_track(&editor).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let clips = sequence.text_track(track).unwrap().clips();
    for pair in clips.windows(2) {
        assert!(
            pair[0].timeline.end <= pair[1].timeline.start,
            "the model accepted overlapping captions"
        );
    }
}

#[test]
fn exporting_writes_what_is_on_the_timeline() {
    let mut editor = editor();
    editor.import_captions(&write_srt("export.srt")).unwrap();

    let out = temp("written.srt");
    assert_eq!(editor.export_captions(&out).unwrap(), 3);

    let text = std::fs::read_to_string(&out).expect("read");
    assert!(text.contains("00:00:01,000 --> 00:00:02,000"));
    assert!(text.contains("first"));
    assert!(text.contains("third"));
}

/// The extension picks the format, and both round trip through the timeline.
#[test]
fn a_round_trip_through_the_timeline_preserves_the_captions() {
    for name in ["round.srt", "round.vtt"] {
        let mut editor = editor();
        editor.import_captions(&write_srt("source.srt")).unwrap();

        let out = temp(name);
        editor.export_captions(&out).unwrap();

        let mut reloaded = self::editor();
        reloaded.import_captions(&out).unwrap();

        assert_eq!(
            reloaded.caption_segments(),
            editor.caption_segments(),
            "{name} did not survive the round trip"
        );
    }
}

/// Editing a caption on the timeline changes what is exported — otherwise the
/// export would be of the file, not of the edit.
#[test]
fn an_edited_caption_is_what_gets_exported() {
    let mut editor = editor();
    editor.import_captions(&write_srt("edited.srt")).unwrap();

    let track = caption_track(&editor).unwrap();
    let id = editor
        .active_sequence()
        .unwrap()
        .text_track(track)
        .unwrap()
        .clips()[0]
        .id;
    editor
        .set_text_property(id, TextProperty::Content("rewritten".into()), false)
        .unwrap();

    let segments = editor.caption_segments();
    assert_eq!(segments[0].text, "rewritten");
}

/// Someone who typed their subtitles as titles still means them as subtitles.
/// Refusing to export because the lane has the wrong name would be pedantry.
#[test]
fn titles_export_as_captions_when_there_is_no_caption_lane() {
    let mut editor = editor();
    editor.add_text("A title").unwrap();

    let segments = editor.caption_segments();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text, "A title");
}

#[test]
fn exporting_an_empty_timeline_is_refused() {
    let editor = editor();
    assert!(editor.export_captions(&temp("nothing.srt")).is_err());
}

/// A lane that can be created and not removed is a trap, and the caption import
/// creates one.
#[test]
fn a_caption_lane_can_be_removed_and_restored() {
    let mut editor = editor();
    editor.import_captions(&write_srt("removable.srt")).unwrap();
    let track = caption_track(&editor).expect("no caption lane");

    let sequence = editor.active_sequence().unwrap().id;
    editor
        .dispatch(bettercut_editor_core::Command::RemoveTrack { sequence, track })
        .unwrap();
    assert!(
        caption_track(&editor).is_none(),
        "the lane survived removal"
    );

    editor.undo().unwrap();
    let restored = caption_track(&editor).expect("the lane did not come back");
    assert_eq!(
        editor
            .active_sequence()
            .unwrap()
            .text_track(restored)
            .unwrap()
            .len(),
        3,
        "the lane came back empty"
    );
}
