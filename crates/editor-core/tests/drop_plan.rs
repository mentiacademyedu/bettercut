//! Where a file dragged onto the timeline goes: never over something already
//! there (`Editor::drop_plan`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::three_point::DropKind;

fn s(seconds: f64) -> TimelineTime {
    TimelineTime::from_millis((seconds * 1000.0) as i64)
}

fn shot(editor: &mut Editor, name: &str, seconds: i64, sound: bool) -> MediaId {
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

/// Three 3 s shots with sound, back to back: 0–3, 3–6, 6–9.
fn three_shots() -> (Editor, MediaId) {
    let (mut editor, _events) = Editor::new_project("Drop");
    let mut first = None;
    for name in ["a", "b", "c"] {
        let media = shot(&mut editor, name, 3, true);
        first.get_or_insert(media);
        editor.place_media(media).unwrap();
    }
    (editor, first.unwrap())
}

#[test]
fn inside_a_clip_it_goes_in_at_the_nearer_edge() {
    let (editor, media) = three_shots();
    assert_eq!(
        editor.drop_plan(media, s(1.0)).unwrap(),
        (s(0.0), DropKind::Insert)
    );
    assert_eq!(
        editor.drop_plan(media, s(2.0)).unwrap(),
        (s(3.0), DropKind::Insert)
    );
    assert_eq!(
        editor.drop_plan(media, s(7.0)).unwrap(),
        (s(6.0), DropKind::Insert)
    );
}

#[test]
fn after_the_end_it_goes_down_where_it_was_dropped() {
    let (editor, media) = three_shots();
    assert_eq!(
        editor.drop_plan(media, s(12.0)).unwrap(),
        (s(12.0), DropKind::Overwrite)
    );
    // Right at a cut: nothing is covered from there on, so nothing moves...
    assert_eq!(
        editor.drop_plan(media, s(9.0)).unwrap(),
        (s(9.0), DropKind::Overwrite)
    );
}

#[test]
fn a_gap_takes_it_only_when_it_fits_on_every_lane() {
    let (mut editor, _) = three_shots();
    // Take the middle shot and its sound out, leaving a 3 s hole at 3–6.
    let middle = editor.active_sequence().unwrap().video_tracks[0].clips()[1].id;
    for clip in editor.linked_with(middle) {
        let track = editor.track_of(clip).unwrap();
        editor.remove_clip(track, clip).unwrap();
    }
    let short = shot(&mut editor, "short", 2, true);
    let long = shot(&mut editor, "long", 4, true);
    assert_eq!(
        editor.drop_plan(short, s(3.5)).unwrap(),
        (s(3.5), DropKind::Overwrite)
    );
    assert_eq!(
        editor.drop_plan(long, s(3.0)).unwrap(),
        (s(3.0), DropKind::Insert),
        "4 s does not fit in a 3 s gap"
    );

    // Music under the gap: the picture lane is free, the sound lane is not,
    // so a shot with sound pushes in rather than cutting the music.
    let song = editor.import_media({
        let mut asset = MediaAsset::new(
            MediaKind::Audio,
            "C:/media/song.mp3",
            MediaTime::from_seconds(10),
        );
        asset.audio_codec = Some("mp3".to_owned());
        asset
    });
    let sound_lane = editor.active_sequence().unwrap().audio_tracks[0].id;
    let music = bettercut_editor_core::timeline::AudioClip::new(
        song,
        s(3.0),
        bettercut_editor_core::timeline::SourceRange::new(
            MediaTime::ZERO,
            MediaTime::from_seconds(3),
        )
        .unwrap(),
    )
    .unwrap();
    editor
        .add_clip(
            sound_lane,
            bettercut_editor_core::ClipPayload::Audio(Box::new(music)),
        )
        .unwrap();
    assert_eq!(
        editor.drop_plan(short, s(3.5)).unwrap(),
        (s(3.5), DropKind::Insert)
    );
    // A silent shot only needs the picture lane.
    let silent = shot(&mut editor, "silent", 2, false);
    assert_eq!(
        editor.drop_plan(silent, s(3.5)).unwrap(),
        (s(3.5), DropKind::Overwrite)
    );
}
