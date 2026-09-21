//! Applying a hand-drawn speed curve (`Editor::apply_speed_factors`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError, SpeedCurve};

/// A ten-second shot with sound on the first lane.
fn shot() -> (Editor, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Curve");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/run.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

fn speeds(editor: &Editor) -> Vec<f64> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|clip| clip.speed.as_f64())
        .collect()
}

#[test]
fn a_curve_cuts_the_clip_into_pieces_that_follow_it() {
    let (mut editor, clip) = shot();
    let depth = editor.undo_depth();
    // Slow at the ends, fast through the middle.
    let curve = SpeedCurve::new([(0.0, 0.5), (0.5, 4.0), (1.0, 0.5)]);

    let pieces = editor
        .apply_speed_factors(clip, &curve.factors(5), "Speed Curve")
        .unwrap();

    assert_eq!(pieces.len(), 5);
    let after = speeds(&editor);
    assert_eq!(after.len(), 5);
    assert!(after[2] > after[0], "{after:?}");
    assert!(after[2] > after[4], "{after:?}");
    // Read at the middle of each piece, so the ends sit between the slowest
    // point of the curve and the rush in the middle — but well under it.
    assert!(after[0] < after[1] && after[4] < after[3], "{after:?}");
    assert!(after[0] < 2.0 && after[4] < 2.0, "{after:?}");
    // Its sound was cut and re-timed with it (§12).
    assert_eq!(
        editor.active_sequence().unwrap().audio_tracks[0]
            .clips()
            .len(),
        5
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(speeds(&editor).len(), 1);
}

/// The curve is relative, like the presets: a clip already at 2× keeps its
/// character rather than snapping back around normal speed.
#[test]
fn a_curve_multiplies_the_speed_the_clip_already_had() {
    let (mut editor, clip) = shot();
    editor
        .set_clip_speed(
            clip,
            bettercut_editor_core::foundation::Rational::new(2, 1).unwrap(),
            false,
        )
        .unwrap();
    let curve = SpeedCurve::new([(0.0, 1.0), (1.0, 1.0)]);
    editor
        .apply_speed_factors(clip, &curve.factors(3), "Speed Curve")
        .unwrap();
    for speed in speeds(&editor) {
        assert!((speed - 2.0).abs() < 1e-6, "{speed}");
    }
}

#[test]
fn a_curve_needs_at_least_two_pieces() {
    let (mut editor, clip) = shot();
    assert!(matches!(
        editor.apply_speed_factors(clip, &[], "Speed Curve"),
        Err(EditorError::TooShortToRamp { .. })
    ));
}

/// A clip too short to give every piece a frame is refused, and nothing about
/// it changes.
#[test]
fn a_clip_too_short_for_the_pieces_is_refused() {
    let (mut editor, clip) = shot();
    let track = editor.track_of(clip).unwrap();
    editor
        .trim_clip(
            track,
            clip,
            bettercut_editor_core::TrimEdge::End,
            TimelineTime::from_ticks(3 * 32_000),
        )
        .unwrap();
    let curve = SpeedCurve::default();
    let before = speeds(&editor);
    assert!(matches!(
        editor.apply_speed_factors(clip, &curve.factors(16), "Speed Curve"),
        Err(EditorError::TooShortToRamp { .. })
    ));
    assert_eq!(speeds(&editor), before);
}
