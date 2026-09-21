//! Setting the sequence's watermark (`Editor::set_watermark`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::watermark::Watermark;

#[test]
fn a_watermark_is_set_clamped_undone_and_keeps_its_logo_in_the_project() {
    let (mut editor, _events) = Editor::new_project("Logo");
    let logo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/logo.png",
        MediaTime::ZERO,
    ));
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(3),
    );
    song.audio_codec = Some("mp3".to_owned());
    let song = editor.import_media(song);

    editor
        .set_watermark(
            Some(Watermark {
                size: 9.0,
                ..Watermark::new(logo)
            }),
            false,
        )
        .unwrap();
    let kept = editor.active_sequence().unwrap().watermark.unwrap();
    assert_eq!(kept.size, 0.5);
    // In use, so it cannot be removed from the media list.
    assert!(editor.media_is_used(logo));

    // A song is no logo.
    assert!(
        editor
            .set_watermark(Some(Watermark::new(song)), false)
            .is_err()
    );

    editor.undo().unwrap();
    assert!(editor.active_sequence().unwrap().watermark.is_none());
    assert!(!editor.media_is_used(logo));
}
