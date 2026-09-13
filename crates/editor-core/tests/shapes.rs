//! Shapes on the title lane (`Editor::add_shape`, `TextProperty::Shape`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::text::{Shape, ShapeKind};
use bettercut_editor_core::{Editor, TextProperty};

/// A shape goes on the title lane at the playhead in one step, and one undo
/// takes it away.
#[test]
fn a_shape_is_added_at_the_playhead() {
    let (mut editor, _events) = Editor::new_project("Shapes");
    editor.set_playhead(TimelineTime::from_seconds(2));
    let depth = editor.undo_depth();

    let clip = editor.add_shape(ShapeKind::Ellipse).unwrap();

    let title = editor.text_clip(clip).unwrap();
    assert_eq!(title.timeline.start, TimelineTime::from_seconds(2));
    assert_eq!(
        title.shape.as_ref().map(|s| s.kind),
        Some(ShapeKind::Ellipse)
    );
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert!(editor.text_clip(clip).is_none());
}

/// A shape's look is changed as one property, undone as one.
#[test]
fn a_shape_is_edited_and_undone_as_a_whole() {
    let (mut editor, _events) = Editor::new_project("Shapes");
    let clip = editor.add_shape(ShapeKind::Rectangle).unwrap();
    let wider = Shape {
        width: 900.0,
        ..Shape::new(ShapeKind::RoundedRectangle)
    };

    editor
        .set_text_property(clip, TextProperty::Shape(Some(wider.clone())), false)
        .unwrap();
    assert_eq!(editor.text_clip(clip).unwrap().shape, Some(wider));

    editor.undo().unwrap();
    assert_eq!(
        editor.text_clip(clip).unwrap().shape,
        Some(Shape::new(ShapeKind::Rectangle))
    );
}
