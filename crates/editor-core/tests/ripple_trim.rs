//! Ripple trim to the playhead: Q and W.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, TrimEdge};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three four-second videos with sound, butted together: 0–4, 4–8, 8–12.
fn three_clips() -> (Editor, Vec<[ClipId; 2]>) {
    let (mut editor, _events) = Editor::new_project("Ripple");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let pairs = (0..3)
        .map(|_| {
            let placed = editor.place_media(media).unwrap();
            [placed[0], placed[1]]
        })
        .collect();
    (editor, pairs)
}

type Spans = Vec<(i64, i64)>;

/// Each lane as whole seconds, and the source in-point of each picture.
fn lanes(editor: &Editor) -> (Spans, Spans, Vec<i64>) {
    let sequence = editor.active_sequence().unwrap();
    let secs = |t: i64| t / 960_000;
    (
        sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|c| (secs(c.timeline.start.ticks()), secs(c.timeline.end.ticks())))
            .collect(),
        sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| (secs(c.timeline.start.ticks()), secs(c.timeline.end.ticks())))
            .collect(),
        sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|c| secs(c.source.start.ticks()))
            .collect(),
    )
}

/// Q with nothing selected: the clip under the playhead loses its start, the
/// rest of the edit moves up, the sound with it, and the playhead lands on the
/// new join. One undo puts it all back.
#[test]
fn q_cuts_the_start_and_closes_the_gap() {
    let (mut editor, _) = three_clips();
    let before = lanes(&editor);
    editor.set_playhead(seconds(5));

    assert_eq!(
        editor
            .ripple_trim_to_playhead(&[], TrimEdge::Start)
            .unwrap(),
        1
    );
    let (pictures, sound, sources) = lanes(&editor);
    assert_eq!(pictures, vec![(0, 4), (4, 7), (7, 11)]);
    assert_eq!(sound, pictures, "the sound fell out of step");
    assert_eq!(
        sources,
        vec![0, 1, 0],
        "the trimmed clip should start 1 s into its file"
    );
    assert_eq!(
        editor.playhead(),
        seconds(4),
        "the playhead should sit on the join"
    );
    assert_eq!(editor.undo_label().as_deref(), Some("Ripple Trim"));

    editor.undo().unwrap();
    assert_eq!(lanes(&editor), before);
}

/// W on the selection: the end goes, the rest moves up, the playhead stays.
#[test]
fn w_cuts_the_end_of_the_selection() {
    let (mut editor, pairs) = three_clips();
    editor.set_playhead(seconds(5));

    // Both halves of the pair selected: still one clip trimmed.
    let trimmed = editor
        .ripple_trim_to_playhead(&pairs[1], TrimEdge::End)
        .unwrap();
    assert_eq!(trimmed, 1);
    let (pictures, sound, _) = lanes(&editor);
    assert_eq!(pictures, vec![(0, 4), (4, 5), (5, 9)]);
    assert_eq!(sound, pictures);
    assert_eq!(editor.playhead(), seconds(5));
}

/// On a cut, or with a selection the playhead is not inside, nothing happens
/// and no undo step is left behind.
#[test]
fn nothing_to_trim_is_no_edit() {
    let (mut editor, pairs) = three_clips();
    let depth = editor.undo_depth();

    editor.set_playhead(seconds(4));
    assert_eq!(
        editor
            .ripple_trim_to_playhead(&[], TrimEdge::Start)
            .unwrap(),
        0
    );
    // W on a cut would otherwise take the whole of the clip that starts there.
    assert_eq!(
        editor.ripple_trim_to_playhead(&[], TrimEdge::End).unwrap(),
        0
    );
    assert_eq!(editor.active_sequence().unwrap().video_tracks[0].len(), 3);
    editor.set_playhead(seconds(5));
    assert_eq!(
        editor
            .ripple_trim_to_playhead(&[pairs[0][0]], TrimEdge::End)
            .unwrap(),
        0
    );
    assert_eq!(editor.undo_depth(), depth);
}
