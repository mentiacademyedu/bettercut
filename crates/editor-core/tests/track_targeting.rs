//! Where a new clip lands, and which lanes ride along with a ripple edit
//! (`bettercut_editor_core::sync_lock`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::TrackKind;
use bettercut_editor_core::{Editor, EditorError, TrackFlag};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn secs(t: TimelineTime) -> i64 {
    t.ticks() / 960_000
}

/// An editor with one four-second video file imported, and a second lane of
/// each kind added.
fn editor_with_two_lanes() -> (Editor, bettercut_editor_core::foundation::MediaId) {
    let (mut editor, _events) = Editor::new_project("Targeting");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.add_video_track("V2").unwrap();
    editor.add_audio_track("A2").unwrap();
    (editor, media)
}

fn video_tracks(editor: &Editor) -> Vec<TrackId> {
    editor
        .active_sequence()
        .unwrap()
        .video_tracks
        .iter()
        .map(|t| t.id)
        .collect()
}

fn audio_tracks(editor: &Editor) -> Vec<TrackId> {
    editor
        .active_sequence()
        .unwrap()
        .audio_tracks
        .iter()
        .map(|t| t.id)
        .collect()
}

fn start_of(editor: &Editor, clip: ClipId) -> TimelineTime {
    editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline
        .start
}

/// A lane's clips, as whole seconds.
fn spans(editor: &Editor, track: TrackId) -> Vec<(i64, i64)> {
    editor
        .active_sequence()
        .unwrap()
        .clip_spans()
        .filter(|span| span.track == track)
        .map(|span| (secs(span.timeline.start), secs(span.timeline.end)))
        .collect()
}

// ---- targeting ----------------------------------------------------------

/// With nothing targeted an import still lands on the first lane of its kind:
/// exactly what every project did before the flag existed.
#[test]
fn without_a_target_an_import_lands_on_the_first_lane() {
    let (mut editor, media) = editor_with_two_lanes();
    let placed = editor.place_media(media).unwrap();
    let video = video_tracks(&editor);
    let audio = audio_tracks(&editor);
    assert_eq!(editor.track_of(placed[0]), Some(video[0]));
    assert_eq!(editor.track_of(placed[1]), Some(audio[0]));
    assert_eq!(editor.target_track(TrackKind::Video), Some(video[0]));
}

#[test]
fn an_import_lands_on_the_targeted_lanes() {
    let (mut editor, media) = editor_with_two_lanes();
    let video = video_tracks(&editor);
    let audio = audio_tracks(&editor);
    editor.set_target_track(video[1], true).unwrap();
    editor.set_target_track(audio[1], true).unwrap();

    let placed = editor.place_media(media).unwrap();
    assert_eq!(editor.track_of(placed[0]), Some(video[1]));
    assert_eq!(editor.track_of(placed[1]), Some(audio[1]));
}

/// The picture and its sound start together even when the targeted lanes are
/// different lengths — the pair is still placed at the later of the two ends.
#[test]
fn a_targeted_pair_still_starts_together() {
    let (mut editor, media) = editor_with_two_lanes();
    let video = video_tracks(&editor);
    let audio = audio_tracks(&editor);
    editor.place_media(media).unwrap();
    editor.set_target_track(video[1], true).unwrap();
    editor.set_target_track(audio[1], true).unwrap();

    let placed = editor.place_media(media).unwrap();
    let picture = start_of(&editor, placed[0]);
    let sound = start_of(&editor, placed[1]);
    assert_eq!(picture, sound);
    assert_eq!(secs(picture), 0, "the second lanes are both empty");
}

/// One target per lane kind: targeting another takes the flag off the first,
/// and it is one undo step.
#[test]
fn targeting_a_lane_untargets_the_other() {
    let (mut editor, _media) = editor_with_two_lanes();
    let video = video_tracks(&editor);
    editor.set_target_track(video[0], true).unwrap();
    editor.set_target_track(video[1], true).unwrap();

    assert!(!editor.track_flag(video[0], TrackFlag::Targeted));
    assert!(editor.track_flag(video[1], TrackFlag::Targeted));
    assert_eq!(editor.undo_label().as_deref(), Some("Target Track"));

    editor.undo().unwrap();
    assert!(editor.track_flag(video[0], TrackFlag::Targeted));
    assert!(!editor.track_flag(video[1], TrackFlag::Targeted));
}

/// Targeting a picture lane says nothing about where sound goes.
#[test]
fn the_lanes_are_targeted_separately() {
    let (mut editor, media) = editor_with_two_lanes();
    let video = video_tracks(&editor);
    let audio = audio_tracks(&editor);
    editor.set_target_track(video[1], true).unwrap();

    let placed = editor.place_media(media).unwrap();
    assert_eq!(editor.track_of(placed[0]), Some(video[1]));
    assert_eq!(editor.track_of(placed[1]), Some(audio[0]));
}

#[test]
fn a_paste_lands_on_the_targeted_lane() {
    let (mut editor, media) = editor_with_two_lanes();
    let placed = editor.place_media(media).unwrap();
    let video = video_tracks(&editor);
    editor.copy_clips(&[placed[0]]);
    editor.set_target_track(video[1], true).unwrap();
    editor.set_playhead(seconds(10));
    editor.paste_at_playhead().unwrap();

    assert_eq!(spans(&editor, video[1]), vec![(10, 14)]);
}

/// A duplicated lane is never the target: one lane per kind carries it.
#[test]
fn a_duplicated_lane_is_not_the_target() {
    let (mut editor, _media) = editor_with_two_lanes();
    let video = video_tracks(&editor);
    editor.set_target_track(video[0], true).unwrap();
    editor
        .set_track_flag(video[0], TrackFlag::SyncLock, true)
        .unwrap();
    let copy = editor.duplicate_track(video[0]).unwrap();

    assert!(!editor.track_flag(copy, TrackFlag::Targeted));
    assert!(
        editor.track_flag(copy, TrackFlag::SyncLock),
        "the copy rides with the same edits the original does"
    );
    assert_eq!(editor.target_track(TrackKind::Video), Some(video[0]));
}

// ---- sync lock ----------------------------------------------------------

/// Two pictures butted together on V1, and one on V2 starting at eight
/// seconds — the shape of an overlay over a cut.
///
/// The V2 clip comes from a silent file, so it has no sound tied to it and
/// what happens to it is about the lane and nothing else.
fn two_lanes_of_pictures() -> (Editor, Vec<TrackId>, Vec<ClipId>) {
    let (mut editor, media) = editor_with_two_lanes();
    let video = video_tracks(&editor);
    let first = editor.place_media(media).unwrap()[0];
    let second = editor.place_media(media).unwrap()[0];

    let silent = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/b.mp4",
        MediaTime::from_seconds(4),
    ));
    editor.set_target_track(video[1], true).unwrap();
    let third = editor.place_media(silent).unwrap()[0];
    editor.set_target_track(video[1], false).unwrap();
    editor
        .move_clip(video[1], video[1], third, seconds(8))
        .unwrap();
    (editor, video, vec![first, second, third])
}

#[test]
fn a_synced_lane_rides_with_a_ripple_delete() {
    let (mut editor, video, clips) = two_lanes_of_pictures();
    editor
        .set_track_flag(video[1], TrackFlag::SyncLock, true)
        .unwrap();

    editor.ripple_delete(video[0], clips[0]).unwrap();
    assert_eq!(spans(&editor, video[0]), vec![(0, 4)]);
    assert_eq!(
        spans(&editor, video[1]),
        vec![(4, 8)],
        "the synced lane stayed where it was"
    );

    // And one undo puts both lanes back.
    editor.undo().unwrap();
    assert_eq!(spans(&editor, video[0]), vec![(0, 4), (4, 8)]);
    assert_eq!(spans(&editor, video[1]), vec![(8, 12)]);
}

#[test]
fn a_lane_without_sync_lock_stays_put() {
    let (mut editor, video, clips) = two_lanes_of_pictures();
    editor.ripple_delete(video[0], clips[0]).unwrap();
    assert_eq!(spans(&editor, video[0]), vec![(0, 4)]);
    assert_eq!(spans(&editor, video[1]), vec![(8, 12)]);
    assert_eq!(editor.undo_label().as_deref(), Some("Ripple Delete"));
}

/// A clip the edit runs through stops it, and nothing moves.
#[test]
fn a_clip_across_the_edit_blocks_the_ripple() {
    let (mut editor, video, clips) = two_lanes_of_pictures();
    let third = clips[2];
    let track = editor.track_of(third).unwrap();
    // Slide it back over the cut the ripple would make.
    editor.move_clip(track, track, third, seconds(2)).unwrap();
    editor
        .set_track_flag(video[1], TrackFlag::SyncLock, true)
        .unwrap();

    let before = (spans(&editor, video[0]), spans(&editor, video[1]));
    let err = editor.ripple_delete(video[0], clips[0]).unwrap_err();
    assert!(matches!(err, EditorError::SyncBlocked), "{err}");
    assert_eq!(
        (spans(&editor, video[0]), spans(&editor, video[1])),
        before,
        "a refused edit changed the timeline"
    );
}

/// A locked lane never rides: lock means leave it alone.
#[test]
fn a_locked_lane_does_not_ride() {
    let (mut editor, video, clips) = two_lanes_of_pictures();
    editor
        .set_track_flag(video[1], TrackFlag::SyncLock, true)
        .unwrap();
    editor
        .set_track_flag(video[1], TrackFlag::Locked, true)
        .unwrap();

    editor.ripple_delete(video[0], clips[0]).unwrap();
    assert_eq!(spans(&editor, video[1]), vec![(8, 12)]);
}

#[test]
fn closing_a_gap_takes_the_synced_lane_with_it() {
    let (mut editor, video, clips) = two_lanes_of_pictures();
    let second = clips[1];
    let track = editor.track_of(second).unwrap();
    editor.move_clip(track, track, second, seconds(6)).unwrap();
    editor
        .set_track_flag(video[1], TrackFlag::SyncLock, true)
        .unwrap();
    assert_eq!(spans(&editor, video[0]), vec![(0, 4), (6, 10)]);

    editor.close_gap(video[0], seconds(5)).unwrap();
    assert_eq!(spans(&editor, video[0]), vec![(0, 4), (4, 8)]);
    assert_eq!(
        spans(&editor, video[1]),
        vec![(6, 10)],
        "the synced lane did not come with the gap closing"
    );
}

/// The ripple trim the Q and W keys make rides too.
#[test]
fn a_ripple_trim_takes_the_synced_lane_with_it() {
    let (mut editor, video, clips) = two_lanes_of_pictures();
    editor
        .set_track_flag(video[1], TrackFlag::SyncLock, true)
        .unwrap();
    editor.set_playhead(seconds(1));
    let trimmed = editor
        .ripple_trim_to_playhead(&[clips[0]], bettercut_editor_core::TrimEdge::Start)
        .unwrap();

    assert_eq!(trimmed, 1);
    assert_eq!(spans(&editor, video[0]), vec![(0, 3), (3, 7)]);
    assert_eq!(spans(&editor, video[1]), vec![(7, 11)]);
}
