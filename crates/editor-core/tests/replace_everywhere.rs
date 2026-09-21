//! Replacing every use of a file (`Editor::replace_media_everywhere`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn video(editor: &mut Editor, name: &str, seconds: i64, sound: bool) -> MediaId {
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        format!("C:/media/{name}.mp4"),
        MediaTime::from_seconds(seconds),
    );
    if sound {
        asset.audio_codec = Some("aac".to_owned());
    }
    editor.import_media(asset)
}

/// A placeholder used twice, then the real take in its place in both, with
/// its sound following — one undo step, and one undo puts both back.
#[test]
fn every_use_takes_the_new_file_in_one_step() {
    let (mut editor, _events) = Editor::new_project("Swap");
    let placeholder = video(&mut editor, "placeholder", 6, true);
    let other = video(&mut editor, "other", 6, false);
    let real = video(&mut editor, "real", 6, true);
    let first = editor.place_media(placeholder).unwrap()[0];
    editor.place_media(other).unwrap();
    let second = editor.place_media(placeholder).unwrap()[0];
    let depth = editor.undo_depth();

    let (changed, refused) = editor.replace_media_everywhere(placeholder, real).unwrap();
    assert_eq!((changed, refused), (2, 0));
    assert_eq!(editor.undo_depth(), depth + 1);
    for clip in [first, second] {
        assert_eq!(editor.video_clip(clip).unwrap().media_id, real);
        let sound = editor
            .linked_with(clip)
            .into_iter()
            .find(|c| editor.audio_clip(*c).is_some())
            .expect("the sound did not follow");
        assert_eq!(editor.audio_clip(sound).unwrap().media_id, real);
    }
    assert!(
        !editor.media_is_used(placeholder),
        "a use of the old file was missed"
    );

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(first).unwrap().media_id, placeholder);
    assert_eq!(editor.video_clip(second).unwrap().media_id, placeholder);
}

/// A file too short for a clip's cut is refused for that clip and counted;
/// the others still change.
#[test]
fn a_clip_the_new_file_cannot_fill_is_counted_not_forced() {
    let (mut editor, _events) = Editor::new_project("Swap");
    let long = video(&mut editor, "long", 10, false);
    let short = video(&mut editor, "short", 3, false);
    let whole = editor.place_media(long).unwrap()[0];
    let trimmed = editor.place_media(long).unwrap()[0];
    let track = editor.track_of(trimmed).unwrap();
    let end = editor.video_clip(trimmed).unwrap().timeline.start
        + bettercut_editor_core::foundation::TimelineTime::from_seconds(2);
    editor
        .trim_clip(
            track,
            trimmed,
            bettercut_editor_core::command::TrimEdge::End,
            end,
        )
        .unwrap();

    let (changed, refused) = editor.replace_media_everywhere(long, short).unwrap();
    assert_eq!((changed, refused), (1, 1));
    assert_eq!(editor.video_clip(whole).unwrap().media_id, long);
    assert_eq!(editor.video_clip(trimmed).unwrap().media_id, short);
}
