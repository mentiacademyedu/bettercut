//! Insert and overwrite from the browser
//! (`bettercut_editor_core::three_point`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::three_point::DropKind;
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Two ten-second shots butted together on V1 and A1, and a third file in the
/// browser that has not been used yet.
fn assembled() -> (Editor, MediaId) {
    let (mut editor, _events) = Editor::new_project("Three point");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/on-timeline.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    editor.place_media(media).unwrap();

    let mut spare = MediaAsset::new(
        MediaKind::Video,
        "C:/media/b-roll.mp4",
        MediaTime::from_seconds(30),
    );
    spare.audio_codec = Some("aac".to_owned());
    let spare = editor.import_media(spare);
    (editor, spare)
}

type Spans = Vec<(i64, i64)>;

fn lanes(editor: &Editor) -> (Spans, Spans) {
    let sequence = editor.active_sequence().unwrap();
    let secs = |t: TimelineTime| t.ticks() / 960_000;
    let of = |clips: &[(i64, i64)]| clips.to_vec();
    let video = sequence.video_tracks[0]
        .clips()
        .iter()
        .map(|c| (secs(c.timeline.start), secs(c.timeline.end)))
        .collect::<Vec<_>>();
    let audio = sequence.audio_tracks[0]
        .clips()
        .iter()
        .map(|c| (secs(c.timeline.start), secs(c.timeline.end)))
        .collect::<Vec<_>>();
    (of(&video), of(&audio))
}

fn part(from: i64, to: i64) -> Option<(MediaTime, MediaTime)> {
    Some((MediaTime::from_seconds(from), MediaTime::from_seconds(to)))
}

/// Overwrite: the marked part lands at the playhead, what was under it goes,
/// and nothing after it moves.
#[test]
fn an_overwrite_replaces_what_is_under_it() {
    let (mut editor, spare) = assembled();
    assert_eq!(lanes(&editor).0, vec![(0, 10), (10, 20)]);

    let placed = editor
        .place_media_at(spare, part(5, 9), seconds(8), DropKind::Overwrite)
        .expect("placed");

    assert_eq!(placed.len(), 2, "the picture and its sound");
    let (video, audio) = lanes(&editor);
    // The first shot is cut at 8, the new four seconds sit 8-12, and the
    // second shot carries on from 12 — still ending where it always did.
    assert_eq!(video, vec![(0, 8), (8, 12), (12, 20)]);
    assert_eq!(audio, video, "the sound fell out of step");
    assert_eq!(editor.undo_label().as_deref(), Some("Overwrite"));
}

/// Insert: everything from there on moves along by the length of the shot.
#[test]
fn an_insert_pushes_everything_along() {
    let (mut editor, spare) = assembled();

    editor
        .place_media_at(spare, part(0, 4), seconds(8), DropKind::Insert)
        .expect("placed");

    let (video, audio) = lanes(&editor);
    assert_eq!(video, vec![(0, 8), (8, 12), (12, 14), (14, 24)]);
    assert_eq!(audio, video);
    assert_eq!(editor.undo_label().as_deref(), Some("Insert"));
}

/// The whole file when nothing is marked.
#[test]
fn with_nothing_marked_the_whole_file_lands() {
    let (mut editor, spare) = assembled();

    editor
        .place_media_at(spare, None, seconds(20), DropKind::Overwrite)
        .expect("placed");

    let (video, _audio) = lanes(&editor);
    assert_eq!(
        video.last().copied(),
        Some((20, 50)),
        "thirty seconds of it"
    );
}

/// The shot lands on the targeted lanes, not always the first ones (§10).
#[test]
fn it_lands_on_the_targeted_lanes() {
    let (mut editor, spare) = assembled();
    editor.add_video_track("V2").unwrap();
    let second = editor.active_sequence().unwrap().video_tracks[1].id;
    editor.set_target_track(second, true).unwrap();

    let placed = editor
        .place_media_at(spare, part(0, 3), seconds(4), DropKind::Overwrite)
        .expect("placed");

    assert_eq!(editor.track_of(placed[0]), Some(second));
    assert_eq!(
        lanes(&editor).0,
        vec![(0, 10), (10, 20)],
        "the first lane should be untouched"
    );
}

/// One undo puts the timeline back, cuts and all.
#[test]
fn one_undo_takes_the_edit_back() {
    for kind in [DropKind::Overwrite, DropKind::Insert] {
        let (mut editor, spare) = assembled();
        let before = lanes(&editor);

        editor
            .place_media_at(spare, part(2, 6), seconds(8), kind)
            .expect("placed");
        editor.undo().expect("undo");

        assert_eq!(lanes(&editor), before, "{}", kind.label());
    }
}

/// A picture and its sound arrive linked, as any placement does (§12).
#[test]
fn the_pair_arrives_linked() {
    let (mut editor, spare) = assembled();

    let placed = editor
        .place_media_at(spare, part(0, 5), seconds(25), DropKind::Overwrite)
        .expect("placed");

    let mut linked = editor.linked_with(placed[0]);
    linked.sort();
    let mut expected = placed.clone();
    expected.sort();
    assert_eq!(linked, expected);
}

/// A marked part the wrong way round is read as the stretch between the two
/// instants, and one past the end of the file is held inside it.
#[test]
fn an_awkward_part_is_read_sensibly() {
    let (mut editor, spare) = assembled();

    let placed = editor
        .place_media_at(spare, part(9, 4), seconds(25), DropKind::Overwrite)
        .expect("placed");
    let span = editor
        .active_sequence()
        .unwrap()
        .clip_span(placed[0])
        .unwrap()
        .timeline;
    assert_eq!(span.duration(), seconds(5));

    let placed = editor
        .place_media_at(spare, part(25, 900), seconds(40), DropKind::Overwrite)
        .expect("placed");
    let span = editor
        .active_sequence()
        .unwrap()
        .clip_span(placed[0])
        .unwrap()
        .timeline;
    assert_eq!(span.duration(), seconds(5), "held inside the file");
}

/// Nowhere to land is refused before anything is touched.
#[test]
fn a_sequence_with_no_lane_for_it_is_refused() {
    let (mut editor, _events) = Editor::new_project("Sound only");
    let sound = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/music.wav",
        MediaTime::from_seconds(60),
    ));
    let sequence = editor.active_sequence().unwrap().id;
    let lane = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .dispatch(bettercut_editor_core::Command::RemoveTrack {
            sequence,
            track: lane,
        })
        .expect("removed");

    let err = editor
        .place_media_at(sound, None, TimelineTime::ZERO, DropKind::Overwrite)
        .unwrap_err();
    assert!(matches!(err, EditorError::NoTrackForMedia), "{err}");
}

/// An overwrite into empty space is just a placement: nothing to clear, and
/// nothing else disturbed.
#[test]
fn an_overwrite_into_empty_space_disturbs_nothing() {
    let (mut editor, spare) = assembled();
    let before = lanes(&editor);

    editor
        .place_media_at(spare, part(0, 4), seconds(40), DropKind::Overwrite)
        .expect("placed");

    let (video, _) = lanes(&editor);
    assert_eq!(video[..before.0.len()], before.0[..]);
    assert_eq!(video.last().copied(), Some((40, 44)));
}

/// A clip that only part of is overwritten keeps the rest of itself.
#[test]
fn a_clip_overwritten_in_the_middle_keeps_both_ends() {
    let (mut editor, spare) = assembled();
    let clips: Vec<ClipId> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|clip| clip.id)
        .collect();

    editor
        .place_media_at(spare, part(0, 2), seconds(4), DropKind::Overwrite)
        .expect("placed");

    let (video, _) = lanes(&editor);
    assert_eq!(video, vec![(0, 4), (4, 6), (6, 10), (10, 20)]);
    assert!(
        editor
            .active_sequence()
            .unwrap()
            .clip_span(clips[1])
            .is_some(),
        "the second shot should be untouched"
    );
}
