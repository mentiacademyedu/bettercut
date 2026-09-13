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

// ---- §33's auto crop ------------------------------------------------------

/// The third answer to the same problem, and the one that costs no resolution:
/// take the frame's shape out of the source rather than enlarging the source
/// until the bars are gone.
mod auto_crop {
    use super::*;

    fn crop_of(
        editor: &Editor,
        clip: bettercut_editor_core::foundation::ClipId,
    ) -> bettercut_editor_core::timeline::Crop {
        editor.video_clip(clip).unwrap().crop
    }

    /// A wide shot in a vertical frame loses its sides, and what is left is the
    /// frame's shape — so it needs no scaling at all.
    #[test]
    fn a_landscape_clip_in_a_vertical_sequence_loses_its_sides() {
        let (mut editor, clip) = landscape_in(1080, 1920);
        let (cropped, kept) = editor.auto_crop_clips().unwrap();
        assert_eq!((cropped, kept), (1, 0));

        let crop = crop_of(&editor, clip);
        assert!(crop.left > 0.2 && crop.right > 0.2, "{crop:?}");
        assert_eq!(crop.top, 0.0, "it lost height as well");

        // And the scale is untouched: cropping is the alternative to scaling,
        // not something done alongside it.
        assert_eq!(scale_of(&editor, clip), (1.0, 1.0));
    }

    /// Nothing to do when the clip is already the frame's shape, and nothing in
    /// the history either.
    #[test]
    fn a_clip_already_the_right_shape_is_left_alone() {
        let (mut editor, clip) = landscape_in(1920, 1080);
        let before = editor.undo_depth();

        let (cropped, kept) = editor.auto_crop_clips().unwrap();
        assert_eq!((cropped, kept), (0, 0));
        assert!(crop_of(&editor, clip).is_none());
        assert_eq!(
            editor.undo_depth(),
            before,
            "a run that changed nothing still left a history entry"
        );
    }

    /// A crop the user set by hand is theirs. Replacing it with a centred one
    /// would throw away a watermark trimmed off an edge or a subject framed on
    /// purpose — work that cannot be guessed back.
    #[test]
    fn a_crop_the_user_set_is_not_replaced() {
        let (mut editor, clip) = landscape_in(1080, 1920);
        let theirs = bettercut_editor_core::timeline::Crop {
            left: 0.3,
            ..bettercut_editor_core::timeline::Crop::NONE
        };
        editor
            .set_clip_value(
                clip,
                bettercut_editor_core::ClipProperty::Crop(theirs),
                false,
            )
            .unwrap();

        let (cropped, kept) = editor.auto_crop_clips().unwrap();
        assert_eq!(
            (cropped, kept),
            (0, 1),
            "the user's own crop was counted as needing one"
        );
        assert_eq!(
            crop_of(&editor, clip),
            theirs,
            "the user's crop was overwritten"
        );
    }

    /// §79: one auto crop is one undo step, however many clips it touched.
    #[test]
    fn an_auto_crop_is_one_undo_step() {
        let (mut editor, clip) = landscape_in(1080, 1920);

        // A second shot on the same track, so there is more than one to undo.
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            "C:/media/wide2.mp4",
            MediaTime::from_seconds(10),
        );
        asset.width = 1920;
        asset.height = 1080;
        let media = editor.import_media(asset);
        let second = editor.place_media(media).unwrap()[0];

        let before = editor.undo_depth();
        let (cropped, _) = editor.auto_crop_clips().unwrap();
        assert_eq!(cropped, 2, "both shots should have been cropped");
        assert_eq!(editor.undo_depth(), before + 1);

        editor.undo().unwrap();
        assert!(crop_of(&editor, clip).is_none(), "undo left the first crop");
        assert!(
            crop_of(&editor, second).is_none(),
            "undo left the second crop"
        );
    }
}

// ---- the hint after a reshape ------------------------------------------------

/// What the sequence panel says after the shape changes, so its fill, crop and
/// fit buttons explain themselves. It has to count the real case, and it has to
/// stop counting once something has been done about it — a hint that stays
/// after the user acted on it is a nag.
mod bars_hint {
    use super::*;

    #[test]
    fn a_landscape_clip_in_a_vertical_sequence_shows_bars() {
        let (editor, _) = landscape_in(1080, 1920);
        assert_eq!(editor.clips_showing_bars(), 1);
    }

    #[test]
    fn a_clip_the_frames_shape_shows_none() {
        let (editor, _) = landscape_in(1920, 1080);
        assert_eq!(editor.clips_showing_bars(), 0);
    }

    /// Acting on it clears it — by cropping or by filling, the two answers that
    /// remove the bars.
    #[test]
    fn cropping_or_filling_to_the_frame_clears_it() {
        let (mut editor, _) = landscape_in(1080, 1920);
        editor.auto_crop_clips().unwrap();
        assert_eq!(
            editor.clips_showing_bars(),
            0,
            "still counted after cropping"
        );

        let (mut editor, _) = landscape_in(1080, 1920);
        editor.reframe_clips(true).unwrap();
        assert_eq!(
            editor.clips_showing_bars(),
            0,
            "still counted after filling"
        );
    }

    /// A shot shrunk into a corner shows bars because someone put it there. It
    /// is a picture-in-picture, not a clip that was never framed.
    #[test]
    fn a_clip_scaled_down_on_purpose_is_not_counted() {
        let (mut editor, clip) = landscape_in(1080, 1920);
        editor
            .set_clip_value(
                clip,
                bettercut_editor_core::ClipProperty::Scale { x: 0.4, y: 0.4 },
                false,
            )
            .unwrap();
        assert_eq!(editor.clips_showing_bars(), 0);
    }

    /// A crop set by hand is a decision too, even one that leaves bars.
    #[test]
    fn a_clip_cropped_by_hand_is_not_counted() {
        let (mut editor, clip) = landscape_in(1080, 1920);
        editor
            .set_clip_value(
                clip,
                bettercut_editor_core::ClipProperty::Crop(bettercut_editor_core::timeline::Crop {
                    top: 0.1,
                    ..bettercut_editor_core::timeline::Crop::NONE
                }),
                false,
            )
            .unwrap();
        assert_eq!(editor.clips_showing_bars(), 0);
    }

    /// A slow zoom is framing that moves, and its keys are the framing.
    #[test]
    fn a_clip_with_a_movement_is_not_counted() {
        let (mut editor, clip) = landscape_in(1080, 1920);
        editor.set_movement(clip, Movement::ZoomIn).unwrap();
        assert_eq!(editor.clips_showing_bars(), 0);
    }
}
