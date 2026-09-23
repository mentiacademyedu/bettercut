//! Find and replace in captions (`Editor::replace_in_captions`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;

#[test]
fn a_misheard_name_is_fixed_in_every_caption_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Names");
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nThanks jon\n\n2\n00:00:03,000 --> 00:00:04,000\nJon agreed\n\n3\n00:00:05,000 --> 00:00:06,000\nNobody else\n";
    let dir = std::env::temp_dir().join(format!("bettercut-replace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let depth = editor.undo_depth();

    assert_eq!(editor.replace_in_captions("jon", "John", false).unwrap(), 2);
    let texts: Vec<String> = editor
        .caption_segments()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["Thanks John", "John agreed", "Nobody else"]);
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    assert_eq!(editor.replace_in_captions("jon", "John", true).unwrap(), 0);

    use bettercut_editor_core::caption_replace::LetterCase;
    assert_eq!(editor.set_caption_case(LetterCase::Upper).unwrap(), 3);
    let texts: Vec<String> = editor
        .caption_segments()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["THANKS JOHN", "JOHN AGREED", "NOBODY ELSE"]);
}

#[test]
fn a_caption_joins_the_next_one_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Join");
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nWe went\n\n2\n00:00:02,000 --> 00:00:04,000\nto the sea\n";
    let dir = std::env::temp_dir().join(format!("bettercut-merge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let first = editor
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .find(|t| t.name == Editor::CAPTION_TRACK)
        .unwrap()
        .clips()[0]
        .id;
    let depth = editor.undo_depth();

    assert!(editor.merge_caption_with_next(first).unwrap());
    let segments = editor.caption_segments();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text, "We went to the sea");
    assert_eq!(
        segments[0].end,
        bettercut_editor_core::foundation::TimelineTime::from_seconds(4)
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    assert!(
        !editor.merge_caption_with_next(first).unwrap(),
        "nothing after it"
    );
}

#[test]
fn a_caption_splits_in_two_and_long_ones_split_together() {
    let (mut editor, _events) = Editor::new_project("Split");
    let srt = "1\n00:00:00,000 --> 00:00:04,000\nwe went down to the sea\n\n2\n00:00:05,000 --> 00:00:06,000\nshort\n";
    let dir = std::env::temp_dir().join(format!("bettercut-split-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let first = editor
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .find(|t| t.name == Editor::CAPTION_TRACK)
        .unwrap()
        .clips()[0]
        .id;
    let depth = editor.undo_depth();

    assert!(editor.split_caption(first).unwrap());
    let segments = editor.caption_segments();
    let texts: Vec<&str> = segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, ["we went down", "to the sea", "short"]);
    assert_eq!(segments[0].end, segments[1].start, "the halves meet");
    assert_eq!(
        segments[1].end,
        bettercut_editor_core::foundation::TimelineTime::from_seconds(4)
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    editor.undo().unwrap();
    assert_eq!(
        editor.split_long_captions(10).unwrap(),
        1,
        "only the long one"
    );
    assert_eq!(editor.caption_segments().len(), 3);
}

#[test]
fn the_caption_lane_moves_up_and_back_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Place");
    let srt = "1\n00:00:00,000 --> 00:00:01,000\nOne\n\n2\n00:00:02,000 --> 00:00:03,000\nTwo\n";
    let dir = std::env::temp_dir().join(format!("bettercut-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let heights = |editor: &Editor| -> Vec<f32> {
        editor
            .active_sequence()
            .unwrap()
            .text_tracks
            .iter()
            .find(|t| t.name == Editor::CAPTION_TRACK)
            .unwrap()
            .clips()
            .iter()
            .map(|c| c.transform.position.y)
            .collect()
    };
    let depth = editor.undo_depth();
    assert_eq!(editor.place_captions(-0.35).unwrap(), 2);
    assert_eq!(heights(&editor), [-0.35, -0.35]);
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.place_captions(-0.35).unwrap(), 0, "already there");
}

#[test]
fn empty_captions_go_and_the_rest_stay() {
    let (mut editor, _events) = Editor::new_project("Blanks");
    let srt = "1\n00:00:00,000 --> 00:00:01,000\nOne\n\n2\n00:00:02,000 --> 00:00:03,000\nTwo\n";
    let dir = std::env::temp_dir().join(format!("bettercut-blank-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    editor.replace_in_captions("Two", " ", true).unwrap();
    assert_eq!(editor.remove_empty_captions().unwrap(), 1);
    let texts: Vec<String> = editor
        .caption_segments()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["One"]);
    assert_eq!(editor.remove_empty_captions().unwrap(), 0);
}

#[test]
fn short_gaps_between_captions_close_and_long_ones_stay() {
    let (mut editor, _events) = Editor::new_project("Gaps");
    let srt = "1\n00:00:00,000 --> 00:00:01,000\nOne\n\n2\n00:00:01,200 --> 00:00:02,000\nTwo\n\n3\n00:00:05,000 --> 00:00:06,000\nThree\n";
    let dir = std::env::temp_dir().join(format!("bettercut-capgaps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    use bettercut_editor_core::foundation::TimelineTime;
    let before = editor.caption_segments();
    assert_eq!(
        editor
            .close_caption_gaps(TimelineTime::from_millis(500))
            .unwrap(),
        1
    );
    let after = editor.caption_segments();
    assert_eq!(after[0].end, before[1].start, "held until the next");
    assert_eq!(after[1].end, before[1].end, "a long gap stays open");
}
