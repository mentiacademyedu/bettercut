//! Where things are (`Editor::clips_using`, `clips_under_playhead`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};

#[test]
fn a_file_s_clips_are_found_and_so_is_what_the_playhead_is_over() {
    let (mut editor, _events) = Editor::new_project("Where");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let a = editor.import_media(asset);
    let b = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/b.mp4",
        MediaTime::from_seconds(4),
    ));
    let first = editor.place_media(a).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(4));
    editor.place_media(b).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(8));
    let again = editor.place_media(a).unwrap();

    let uses = editor.clips_using(a);
    assert_eq!(
        uses.len(),
        4,
        "two placements, picture and sound each: {uses:?}"
    );
    assert!(uses.windows(2).all(|p| p[0].1 <= p[1].1), "earliest first");
    assert!(uses.iter().any(|(c, _)| *c == first[0]));
    assert!(uses.iter().any(|(c, _)| *c == again[0]));
    assert_eq!(editor.clips_using(b).len(), 1, "b has no sound");

    editor.set_playhead(TimelineTime::from_seconds(1));
    let under = editor.clips_under_playhead();
    assert_eq!(under.len(), 2, "the first placement's picture and sound");
}
