//! Placing part of a file (`Editor::place_media_range`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn secs(n: i64) -> MediaTime {
    MediaTime::from_seconds(n)
}

/// A twenty-second shot with sound in the library.
fn library() -> (Editor, bettercut_editor_core::foundation::MediaId) {
    let (mut editor, _events) = Editor::new_project("Subclips");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/long.mp4",
        MediaTime::from_seconds(20),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    (editor, media)
}

fn span(editor: &Editor, clip: ClipId) -> (i64, i64, i64, i64) {
    let video = editor.video_clip(clip).unwrap();
    (
        video.timeline.start.ticks(),
        video.timeline.end.ticks(),
        video.source.start.ticks(),
        video.source.end.ticks(),
    )
}

#[test]
fn only_the_marked_part_is_placed() {
    let (mut editor, media) = library();

    let clips = editor
        .place_media_range(media, Some((secs(5), secs(9))))
        .unwrap();

    assert_eq!(clips.len(), 2, "the sound should come with it");
    let (start, end, from, to) = span(&editor, clips[0]);
    assert_eq!((from, to), (secs(5).ticks(), secs(9).ticks()));
    assert_eq!(end - start, TimelineTime::from_seconds(4).ticks());
    // Its sound plays the same part.
    let sound = editor.audio_clip(clips[1]).unwrap();
    assert_eq!(sound.source.start, secs(5));
    assert_eq!(sound.source.end, secs(9));
}

#[test]
fn a_part_marked_backwards_is_the_span_between_the_two() {
    let (mut editor, media) = library();
    let clips = editor
        .place_media_range(media, Some((secs(9), secs(5))))
        .unwrap();
    let (_, _, from, to) = span(&editor, clips[0]);
    assert_eq!((from, to), (secs(5).ticks(), secs(9).ticks()));
}

#[test]
fn a_part_reaching_past_the_end_is_held_inside_the_file() {
    let (mut editor, media) = library();
    let clips = editor
        .place_media_range(media, Some((secs(18), secs(40))))
        .unwrap();
    let (_, _, from, to) = span(&editor, clips[0]);
    assert_eq!((from, to), (secs(18).ticks(), secs(20).ticks()));
}

/// No marks: the whole file, exactly as it always was.
#[test]
fn placing_without_a_part_places_all_of_it() {
    let (mut editor, media) = library();
    let clips = editor.place_media_range(media, None).unwrap();
    let (_, _, from, to) = span(&editor, clips[0]);
    assert_eq!((from, to), (0, secs(20).ticks()));
}

/// A photo is one picture: there is no part of it to take.
#[test]
fn a_photo_ignores_a_marked_part() {
    let (mut editor, _events) = Editor::new_project("Subclips");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/beach.jpg",
        MediaTime::ZERO,
    ));
    let clips = editor
        .place_media_range(media, Some((secs(1), secs(2))))
        .unwrap();
    let (start, end, ..) = span(&editor, clips[0]);
    assert_eq!(
        end - start,
        editor.project().settings.photo_length.ticks(),
        "a photo should run for the project's photo length"
    );
}
