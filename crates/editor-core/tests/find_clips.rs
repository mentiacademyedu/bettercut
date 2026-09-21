//! Finding clips by name (`Editor::find_clips`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::find::matches_query;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn every_word_must_appear_in_any_case() {
    assert!(matches_query("Interview_Wide_02.mp4", "wide interview"));
    assert!(!matches_query("Interview_Wide_02.mp4", "wide close"));
    assert!(!matches_query("anything", "   "));
}

/// Files by their name or given name, titles by their words, clips by their
/// note — in time order.
#[test]
fn clips_are_found_by_file_title_and_note() {
    let (mut editor, _events) = Editor::new_project("Find");
    let drone = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/DJI_0042.mp4",
        MediaTime::from_seconds(4),
    ));
    let talk = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/C0007.mp4",
        MediaTime::from_seconds(4),
    ));
    let drone_clip = editor.place_media(drone).unwrap()[0];
    let talk_clip = editor.place_media(talk).unwrap()[0];
    editor.rename_media(talk, "Interview, wide").unwrap();
    editor
        .set_clip_note(drone_clip, "sunset over the bay")
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(1));
    let title = editor.add_text("Chapter one").unwrap();

    let by_file = editor.find_clips("dji");
    assert_eq!(by_file.len(), 1);
    assert_eq!(by_file[0].clip, drone_clip);

    let by_given_name = editor.find_clips("INTERVIEW");
    assert_eq!(by_given_name.len(), 1);
    assert_eq!(by_given_name[0].clip, talk_clip);
    assert_eq!(by_given_name[0].label, "Interview, wide");

    assert_eq!(editor.find_clips("bay sunset")[0].clip, drone_clip);
    let titles = editor.find_clips("chapter");
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].clip, title);

    assert!(editor.find_clips("nothing like this").is_empty());
}
