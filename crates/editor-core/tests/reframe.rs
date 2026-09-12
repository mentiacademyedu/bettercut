//! Re-framing a whole sequence for a new canvas shape (§36).
//!
//! The case this exists for: an edit cut in landscape, switched to vertical for
//! Shorts. §36 says resizing must not touch the source media, and it doesn't —
//! but every clip in the sequence is suddenly a different shape from the frame,
//! and telling each one by hand is the work an editor should absorb.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::Resolution;
use bettercut_editor_core::{Editor, Movement};

/// A sequence of `resolution` with one landscape clip on it.
fn landscape_in(width: u32, height: u32) -> (Editor, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Reframe");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/wide.mp4",
        MediaTime::from_seconds(10),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();

    editor
        .set_sequence_format(
            Resolution::new(width, height),
            bettercut_editor_core::foundation::FrameRate::FPS_30,
        )
        .unwrap();
    (editor, placed[0])
}

fn scale_of(editor: &Editor, clip: bettercut_editor_core::foundation::ClipId) -> (f32, f32) {
    let clip = editor.video_clip(clip).unwrap();
    (clip.transform.scale.x, clip.transform.scale.y)
}

/// 16:9 footage in a 9:16 frame has to be scaled by (16/9)/(9/16) — about
/// 3.16 — to cover it.
#[test]
fn landscape_footage_fills_a_vertical_frame() {
    let (mut editor, clip) = landscape_in(1080, 1920);
    assert_eq!(scale_of(&editor, clip), (1.0, 1.0), "starts fitted");

    let (rescaled, animated) = editor.reframe_clips(true).unwrap();

    assert_eq!((rescaled, animated), (1, 0));
    let (x, y) = scale_of(&editor, clip);
    let wanted = (16.0 / 9.0) / (9.0 / 16.0);
    assert!((x - wanted).abs() < 0.01, "scaled by {x}, wanted {wanted}");
    assert_eq!(x, y, "the picture was stretched rather than scaled");
}

#[test]
fn fitting_puts_it_back() {
    let (mut editor, clip) = landscape_in(1080, 1920);
    editor.reframe_clips(true).unwrap();

    let (rescaled, _) = editor.reframe_clips(false).unwrap();

    assert_eq!(rescaled, 1);
    assert_eq!(scale_of(&editor, clip), (1.0, 1.0));
}

/// Nothing to do is not an edit. A history full of steps that changed nothing
/// makes undo useless.
#[test]
fn clips_already_framed_that_way_are_left_alone() {
    let (mut editor, _clip) = landscape_in(1920, 1080);
    let before = editor.undo_depth();

    // Same shape as the frame, so filling it is what it already does.
    let (rescaled, animated) = editor.reframe_clips(true).unwrap();

    assert_eq!((rescaled, animated), (0, 0));
    assert_eq!(
        editor.undo_depth(),
        before,
        "an empty step went into history"
    );
}

/// A movement is framing the user wrote by hand. Flattening it to one number
/// would throw away work that cannot be guessed back.
#[test]
fn an_animated_clip_keeps_its_movement() {
    let (mut editor, clip) = landscape_in(1080, 1920);
    editor.set_movement(clip, Movement::ZoomIn).unwrap();
    let keyframes = editor.video_clip(clip).unwrap().keyframes.clone();

    let (rescaled, animated) = editor.reframe_clips(true).unwrap();

    assert_eq!((rescaled, animated), (0, 1), "the movement was overwritten");
    assert_eq!(
        editor.video_clip(clip).unwrap().keyframes,
        keyframes,
        "the keyframes changed"
    );
}

/// Every clip in one step: switching a whole edit to vertical is one decision.
#[test]
fn a_whole_sequence_reframes_as_one_undo_step() {
    let (mut editor, first) = landscape_in(1080, 1920);
    let media = editor.video_clip(first).unwrap().media_id;
    editor.set_playhead(TimelineTime::from_seconds(10));
    editor.place_media(media).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(20));
    editor.place_media(media).unwrap();

    let before = editor.undo_depth();
    let (rescaled, _) = editor.reframe_clips(true).unwrap();
    assert_eq!(rescaled, 3);
    assert_eq!(editor.undo_depth(), before + 1);

    editor.undo().unwrap();
    assert_eq!(
        scale_of(&editor, first),
        (1.0, 1.0),
        "undo put back only some of it"
    );
}
