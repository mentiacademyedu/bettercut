//! Grouped clips: selected and moved as one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A four-second video with sound at 0, a second one at 4, and a title at 1.
fn edit() -> (Editor, [ClipId; 2], [ClipId; 2], ClipId) {
    let (mut editor, _events) = Editor::new_project("Groups");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let first = editor.place_media(media).unwrap();
    let second = editor.place_media(media).unwrap();
    editor.set_playhead(seconds(1));
    let title = editor.add_text("Over the first shot").unwrap();
    (editor, [first[0], first[1]], [second[0], second[1]], title)
}

fn start(editor: &Editor, clip: ClipId) -> i64 {
    editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline
        .start
        .ticks()
        / 960_000
}

/// A picture and a title grouped move together when either is moved — the
/// picture's sound too — in one undo step.
#[test]
fn a_grouped_title_moves_with_its_picture() {
    let (mut editor, first, second, title) = edit();
    // Out of the way of the second shot, so the first can move right.
    let track = editor.track_of(second[0]).unwrap();
    editor
        .move_clip(track, track, second[0], seconds(20))
        .unwrap();

    assert_eq!(editor.group_clips(&[first[0], title]).unwrap(), 2);
    assert_eq!(editor.group_of(title).unwrap().len(), 2);
    assert!(
        editor.moves_with(title).contains(&first[1]),
        "the sound is left behind"
    );

    let depth = editor.undo_depth();
    let track = editor.track_of(first[0]).unwrap();
    editor
        .move_clip(track, track, first[0], seconds(3))
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(start(&editor, first[0]), 3);
    assert_eq!(start(&editor, first[1]), 3);
    assert_eq!(start(&editor, title), 4, "the title did not come along");

    // Dragging the title brings the picture back with it.
    let text_track = editor.track_of(title).unwrap();
    editor
        .move_clip(text_track, text_track, title, seconds(1))
        .unwrap();
    assert_eq!(start(&editor, first[0]), 0);

    editor.undo().unwrap();
    assert_eq!(start(&editor, first[0]), 3);
    assert_eq!(start(&editor, title), 4);
}

/// Two grouped clips on one track move together without running into each
/// other, whichever way they go.
#[test]
fn clips_on_one_track_move_as_a_block() {
    let (mut editor, first, second, _) = edit();
    editor.group_clips(&[first[0], second[0]]).unwrap();
    let track = editor.track_of(first[0]).unwrap();

    editor
        .move_clip(track, track, first[0], seconds(2))
        .unwrap();
    assert_eq!(
        (start(&editor, first[0]), start(&editor, second[0])),
        (2, 6)
    );
    editor
        .move_clip(track, track, second[0], seconds(4))
        .unwrap();
    assert_eq!(
        (start(&editor, first[0]), start(&editor, second[0])),
        (0, 4)
    );
}

/// A picture and its own sound are one thing, not a group; a clip already in
/// a group brings its group into a new one; ungrouping is one step.
#[test]
fn grouping_rules() {
    let (mut editor, first, second, title) = edit();
    let depth = editor.undo_depth();
    assert!(matches!(
        editor.group_clips(&[first[0], first[1]]),
        Err(EditorError::NothingToGroup)
    ));
    assert_eq!(editor.undo_depth(), depth);

    editor.group_clips(&[first[0], title]).unwrap();
    editor.group_clips(&[title, second[0]]).unwrap();
    let group = editor.group_of(second[0]).unwrap();
    assert!(group.contains(&first[0]) && group.contains(&title));
    assert_eq!(
        editor.active_sequence().unwrap().groups.len(),
        1,
        "a clip ended up in two groups"
    );

    assert_eq!(editor.ungroup_clips(&[title]).unwrap(), 1);
    assert!(editor.group_of(first[0]).is_none());
    assert_eq!(editor.ungroup_clips(&[title]).unwrap(), 0);
    editor.undo().unwrap();
    assert!(editor.group_of(first[0]).is_some());
}

/// A deleted member leaves the group; a group of one is no group.
#[test]
fn a_deleted_member_leaves_its_group() {
    let (mut editor, first, second, _) = edit();
    editor.group_clips(&[first[0], second[0]]).unwrap();
    let track = editor.track_of(second[0]).unwrap();
    editor.remove_clip(track, second[0]).unwrap();
    assert!(editor.group_of(first[0]).is_none());
    assert_eq!(editor.moves_with(first[0]), editor.linked_with(first[0]));
}

/// Groups are saved, and a project from before them loads with none.
#[test]
fn groups_are_saved_and_old_projects_have_none() {
    let (mut editor, first, _, title) = edit();
    editor.group_clips(&[first[0], title]).unwrap();
    let mut json = serde_json::to_value(editor.project()).unwrap();
    let restored: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json.clone()).unwrap();
    assert_eq!(restored.sequences[0].groups.len(), 1);

    json["sequences"][0]
        .as_object_mut()
        .unwrap()
        .remove("groups");
    let old: bettercut_editor_core::project_format::Project = serde_json::from_value(json).unwrap();
    assert!(old.sequences[0].groups.is_empty());
}

/// Splitting a grouped clip keeps both halves in the group, so the pieces
/// still move with the rest; undoing the split puts the whole clip back in.
#[test]
fn a_split_clip_stays_in_its_group() {
    let (mut editor, first, second, title) = edit();
    editor.group_clips(&[first[0], title]).unwrap();
    // Out of the way, so the group can move right.
    let track = editor.track_of(second[0]).unwrap();
    editor
        .move_clip(track, track, second[0], seconds(20))
        .unwrap();

    editor.set_playhead(seconds(2));
    editor.split_at_playhead(&[first[0]]).unwrap();
    let pieces = editor.active_sequence().unwrap().video_tracks[0].clips()[..2]
        .iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let group = editor
        .group_of(title)
        .expect("the split broke the group up");
    assert!(
        pieces.iter().all(|piece| group.contains(piece)),
        "a half left the group: {group:?} vs {pieces:?}"
    );

    // Moving the title carries both halves.
    let text_track = editor.track_of(title).unwrap();
    editor
        .move_clip(text_track, text_track, title, seconds(4))
        .unwrap();
    assert_eq!(start(&editor, pieces[0]), 3);
    assert_eq!(start(&editor, pieces[1]), 5);

    editor.undo().unwrap();
    editor.undo().unwrap();
    let whole = editor.group_of(title).expect("undo lost the group");
    assert!(
        whole.contains(&first[0]),
        "undoing the split did not restore the clip to its group"
    );
    assert_eq!(whole.len(), 2);
}
