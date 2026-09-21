//! A lower third in one click (`Editor::add_lower_third`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::lower_third::{LOWER_THIRD_ACCENT, LOWER_THIRD_DURATION};
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn a_bar_a_name_and_a_role_land_together_in_one_step() {
    let (mut editor, _events) = Editor::new_project("Titles");
    editor.set_playhead(TimelineTime::from_seconds(3));
    let depth = editor.undo_depth();

    let parts = editor
        .add_lower_third("  Ada Lovelace ", "Mathematician", LOWER_THIRD_ACCENT)
        .unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(editor.undo_depth(), depth + 1);

    let bar = editor.text_clip(parts[0]).unwrap();
    let name = editor.text_clip(parts[1]).unwrap();
    let role = editor.text_clip(parts[2]).unwrap();
    assert!(bar.shape.is_some(), "the bar is not a shape");
    assert_eq!(name.text, "Ada Lovelace");
    assert_eq!(role.text, "Mathematician");
    for part in [bar, name, role] {
        assert_eq!(part.timeline.start, TimelineTime::from_seconds(3));
        assert_eq!(part.timeline.duration(), LOWER_THIRD_DURATION);
        assert!(part.animation.intro.is_some() && part.animation.outro.is_some());
    }
    // Name and role line up on the left, the role beneath, the bar to the left.
    assert_eq!(name.transform.position.x, role.transform.position.x);
    assert!(role.transform.position.y > name.transform.position.y);
    assert!(bar.transform.position.x < name.transform.position.x);
    assert!(name.style.size > role.style.size);

    // Each part on a lane of its own.
    let lanes: std::collections::HashSet<_> = parts
        .iter()
        .map(|p| editor.active_sequence().unwrap().text_track_of(*p).unwrap())
        .collect();
    assert_eq!(lanes.len(), 3);

    editor.undo().unwrap();
    assert!(editor.text_clip(parts[1]).is_none());
}

#[test]
fn a_second_one_over_the_first_gets_lanes_of_its_own_and_a_role_is_optional() {
    let (mut editor, _events) = Editor::new_project("Titles");
    let first = editor
        .add_lower_third("One", "First", [0, 120, 255])
        .unwrap();
    let second = editor.add_lower_third("Two", "", [0, 120, 255]).unwrap();
    assert_eq!(second.len(), 2, "an empty role still made a clip");
    let sequence = editor.active_sequence().unwrap();
    for part in &second {
        let lane = sequence.text_track_of(*part).unwrap();
        assert!(
            first
                .iter()
                .all(|p| sequence.text_track_of(*p) != Some(lane))
        );
    }

    assert!(matches!(
        editor.add_lower_third("   ", "Role", [0, 0, 0]),
        Err(EditorError::LowerThirdNeedsName)
    ));
}
