//! Every change to a title can be written to the autosave journal and read
//! back. `TextProperty` was internally tagged, which serde cannot do for a
//! variant holding a bare string, number or flag: changing a title's words
//! failed to journal ("autosave failed once"), so a crash lost the edit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, SequenceId, TrackId};
use bettercut_editor_core::{Command, TextProperty};

#[test]
fn every_title_change_survives_the_journal() {
    let properties = vec![
        TextProperty::Content("New words".to_owned()),
        TextProperty::Style(Box::default()),
        TextProperty::Position { x: 0.1, y: -0.2 },
        TextProperty::Scale { x: 1.5, y: 1.5 },
        TextProperty::Rotation(12.0),
        TextProperty::Opacity(0.5),
        TextProperty::Animation(Default::default()),
        TextProperty::MotionBlur(true),
        TextProperty::Shape(None),
        TextProperty::Counter(None),
        TextProperty::Highlight(Some(bettercut_editor_core::text::Rgba::opaque(255, 200, 0))),
        TextProperty::Highlight(None),
    ];
    for property in properties {
        let command = Command::SetTextProperty {
            sequence: SequenceId::new(),
            track: TrackId::new(),
            clip: ClipId::new(),
            property: property.clone(),
        };
        let written = serde_json::to_string(&command)
            .unwrap_or_else(|err| panic!("{property:?} could not be journalled: {err}"));
        let read: Command = serde_json::from_str(&written)
            .unwrap_or_else(|err| panic!("{property:?} could not be read back: {err}"));
        assert_eq!(read, command, "{written}");
    }
}
