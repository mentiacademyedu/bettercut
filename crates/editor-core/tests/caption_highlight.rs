//! Lighting each word of the captions (`Editor::set_caption_highlight`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_captions::CaptionSegment;
use bettercut_editor_core::Editor;
use bettercut_editor_core::command::TextProperty;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::text::Rgba;

fn captioned() -> Editor {
    let (mut editor, _events) = Editor::new_project("Karaoke");
    editor
        .replace_captions(
            vec![
                CaptionSegment::new(
                    TimelineTime::ZERO,
                    TimelineTime::from_seconds(2),
                    "hello there",
                ),
                CaptionSegment::new(
                    TimelineTime::from_seconds(2),
                    TimelineTime::from_seconds(4),
                    "general kenobi",
                ),
            ],
            "Captions",
        )
        .unwrap();
    editor
}

fn lane(editor: &Editor) -> Vec<(bettercut_editor_core::foundation::ClipId, Option<Rgba>)> {
    let sequence = editor.active_sequence().unwrap();
    sequence
        .text_tracks
        .iter()
        .find(|t| t.name == Editor::CAPTION_TRACK)
        .unwrap()
        .clips()
        .iter()
        .map(|c| (c.id, c.highlight))
        .collect()
}

fn highlights(editor: &Editor) -> Vec<Option<Rgba>> {
    lane(editor).into_iter().map(|(_, h)| h).collect()
}

#[test]
fn the_whole_lane_lights_up_in_one_step() {
    let mut editor = captioned();
    assert_eq!(editor.caption_highlight(), None);
    let depth = editor.undo_depth();

    assert_eq!(
        editor
            .set_caption_highlight(Some(Editor::DEFAULT_HIGHLIGHT))
            .unwrap(),
        2
    );
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(
        highlights(&editor),
        vec![Some(Editor::DEFAULT_HIGHLIGHT); 2]
    );
    assert_eq!(editor.caption_highlight(), Some(Editor::DEFAULT_HIGHLIGHT));
    assert_eq!(
        editor
            .set_caption_highlight(Some(Editor::DEFAULT_HIGHLIGHT))
            .unwrap(),
        0,
        "the same colour again was a step"
    );

    editor.undo().unwrap();
    assert_eq!(highlights(&editor), vec![None, None]);
}

/// Captions that disagree have no lane colour to show.
#[test]
fn a_mixed_lane_has_no_single_colour() {
    let mut editor = captioned();
    let first = lane(&editor)[0].0;
    editor
        .set_text_property(
            first,
            TextProperty::Highlight(Some(Rgba::opaque(0, 200, 255))),
            false,
        )
        .unwrap();
    assert_eq!(editor.caption_highlight(), None);
    assert_eq!(editor.set_caption_highlight(None).unwrap(), 1);
    assert_eq!(highlights(&editor), vec![None, None]);
}

#[test]
fn no_captions_is_nothing_to_light() {
    let (mut editor, _events) = Editor::new_project("Karaoke");
    assert_eq!(
        editor
            .set_caption_highlight(Some(Editor::DEFAULT_HIGHLIGHT))
            .unwrap(),
        0
    );
}
