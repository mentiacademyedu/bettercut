//! Shifting the caption lane (`Editor::shift_captions`): every caption by
//! the same amount, one step, never before the start.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

fn captioned() -> Editor {
    let (mut editor, _events) = Editor::new_project("Sync");
    let srt =
        "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n";
    let dir = std::env::temp_dir().join(format!("bettercut-shift-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("captions.srt");
    std::fs::write(&path, srt).unwrap();
    editor.import_captions(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    editor
}

fn starts(editor: &Editor) -> Vec<TimelineTime> {
    editor.caption_segments().iter().map(|s| s.start).collect()
}

#[test]
fn captions_move_together_and_never_before_the_start() {
    let mut editor = captioned();
    assert_eq!(
        starts(&editor),
        [TimelineTime::from_seconds(1), TimelineTime::from_seconds(3)]
    );
    let depth = editor.undo_depth();

    assert_eq!(
        editor
            .shift_captions(TimelineTime::from_millis(500))
            .unwrap(),
        2
    );
    assert_eq!(
        starts(&editor),
        [
            TimelineTime::from_millis(1_500),
            TimelineTime::from_millis(3_500)
        ]
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    // Asked to go back two seconds, it goes back the one and a half it can.
    assert_eq!(
        editor
            .shift_captions(TimelineTime::from_seconds(-2))
            .unwrap(),
        2
    );
    assert_eq!(
        starts(&editor),
        [TimelineTime::ZERO, TimelineTime::from_seconds(2)]
    );
    assert_eq!(editor.shift_captions(TimelineTime::ZERO).unwrap(), 0);

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(
        starts(&editor),
        [TimelineTime::from_seconds(1), TimelineTime::from_seconds(3)]
    );
}
