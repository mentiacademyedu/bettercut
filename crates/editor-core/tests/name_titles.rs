//! A lower third over each clip, reading its name (`Editor::title_each_clip`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn each_picture_gets_a_title_with_its_name_and_taken_stretches_are_skipped() {
    let (mut editor, _events) = Editor::new_project("Cards");
    let mut pictures = Vec::new();
    for name in ["Lisbon", "Porto"] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        ));
        pictures.push(editor.place_media(media).unwrap()[0]);
    }
    editor
        .set_clip_name(pictures[1], Some("Porto at dusk"))
        .unwrap();
    let depth = editor.undo_depth();

    assert_eq!(editor.title_each_clip(&pictures).unwrap(), 2);
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    let lane = &editor.active_sequence().unwrap().text_tracks[0];
    let texts: Vec<(String, _)> = lane
        .clips()
        .iter()
        .map(|c| (c.text.clone(), c.timeline))
        .collect();
    assert_eq!(texts.len(), 2);
    assert!(texts[0].0.contains("Lisbon"), "{:?}", texts[0].0);
    assert_eq!(texts[1].0, "Porto at dusk");
    assert_eq!(texts[1].1, editor.video_clip(pictures[1]).unwrap().timeline);

    assert_eq!(
        editor.title_each_clip(&pictures).unwrap(),
        0,
        "the lane is taken now"
    );
}
