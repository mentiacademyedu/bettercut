//! The reshaped copies an export writes alongside the main file
//! (`bettercut_editor_core::reshape`).
//!
//! What matters: the copy is the new shape and its untouched shots fill it,
//! shots someone framed by hand keep that framing, and the edit on screen does
//! not change at all — not its shape, not a clip, not the undo list.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, FrameRate, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Crop, Resolution, crop_to_aspect};
use bettercut_editor_core::{ClipProperty, Editor, SHAPES, Shape};

fn shape(label: &str) -> Shape {
    SHAPES.into_iter().find(|s| s.label == label).unwrap()
}

/// A 1920×1080 sequence with a landscape clip placed on it, then `extra`
/// more of the same after it.
fn landscape_edit(extra: usize) -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Shapes");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/wide.mp4",
        MediaTime::from_seconds(10),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = editor.import_media(asset);
    editor
        .set_sequence_format(Resolution::HD_1080, FrameRate::FPS_30)
        .unwrap();
    let clips = (0..=extra)
        .map(|_| editor.place_media(media).unwrap()[0])
        .collect();
    (editor, clips)
}

fn crop_in(project: &bettercut_editor_core::project_format::Project, clip: ClipId) -> Crop {
    project
        .active()
        .unwrap()
        .video_tracks
        .iter()
        .find_map(|t| t.get(clip))
        .unwrap()
        .crop
}

/// The vertical copy is vertical at the same detail, and a landscape shot is
/// cropped to fill it rather than sitting between bars.
#[test]
fn a_vertical_copy_is_vertical_and_filled() {
    let (editor, clips) = landscape_edit(0);

    let (copy, sequence) = editor.export_copy(Some(shape("9:16"))).unwrap();

    assert_eq!(
        copy.sequence(sequence).unwrap().resolution,
        Resolution::new(1080, 1920),
        "the copy lost detail or kept its shape"
    );
    assert_eq!(
        crop_in(&copy, clips[0]),
        crop_to_aspect(16.0 / 9.0, 1080.0 / 1920.0),
        "the landscape shot was not cropped to the vertical frame"
    );
}

/// The whole point of a copy: the edit the user is looking at is exactly as it
/// was, with nothing added to undo.
#[test]
fn the_edit_itself_is_not_changed() {
    let (editor, clips) = landscape_edit(0);
    let depth = editor.undo_depth();
    let dirty = editor.is_dirty();

    let _ = editor.export_copy(Some(shape("1:1"))).unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.resolution, Resolution::HD_1080);
    assert!(editor.video_clip(clips[0]).unwrap().crop.is_none());
    assert_eq!(editor.undo_depth(), depth);
    assert_eq!(editor.is_dirty(), dirty);
}

/// A shot framed by hand — shrunk, or cropped — was a decision, so the copy
/// keeps it rather than guessing what the same decision would be at another
/// shape.
#[test]
fn clips_framed_by_hand_keep_their_framing() {
    let (mut editor, clips) = landscape_edit(2);
    editor
        .set_clip_property(clips[1], ClipProperty::Scale { x: 0.5, y: 0.5 }, false)
        .unwrap();
    let by_hand = Crop {
        left: 0.1,
        ..Crop::NONE
    };
    editor
        .set_clip_property(clips[2], ClipProperty::Crop(by_hand), false)
        .unwrap();

    let (copy, _) = editor.export_copy(Some(shape("9:16"))).unwrap();

    assert!(
        !crop_in(&copy, clips[0]).is_none(),
        "the untouched shot was not cropped"
    );
    assert!(
        crop_in(&copy, clips[1]).is_none(),
        "a shrunk shot was cropped"
    );
    assert_eq!(
        crop_in(&copy, clips[2]),
        by_hand,
        "a hand crop was replaced"
    );
}

/// The copy counts as framed exactly what the sequence panel's hint counts —
/// so after reshaping the copy's sequence, nothing is left showing bars.
#[test]
fn the_copy_leaves_no_untouched_shot_showing_bars() {
    let (editor, _) = landscape_edit(2);

    for target in SHAPES
        .into_iter()
        .filter(|s| !s.matches(Resolution::HD_1080))
    {
        let (copy, _) = editor.export_copy(Some(target)).unwrap();
        let (reopened, _events) = Editor::from_project(copy);
        assert_eq!(
            reopened.clips_showing_bars(),
            0,
            "{} copy still shows bars",
            target.label
        );
    }
}

/// No shape is the project as it stands, for the main file.
#[test]
fn no_shape_is_the_edit_as_it_is() {
    let (editor, clips) = landscape_edit(0);

    let (copy, sequence) = editor.export_copy(None).unwrap();

    assert_eq!(
        copy.sequence(sequence).unwrap().resolution,
        Resolution::HD_1080
    );
    assert!(crop_in(&copy, clips[0]).is_none());
}

/// Every shape keeps the short edge and comes out even (§36), and exactly one
/// shape claims each common size.
#[test]
fn shapes_keep_the_short_edge_and_stay_even() {
    for target in SHAPES {
        for start in [
            Resolution::HD_1080,
            Resolution::new(1080, 1920),
            Resolution::new(3840, 2160),
            Resolution::new(999, 501),
        ] {
            let shaped = target.applied_to(start);
            assert!(
                shaped.width.is_multiple_of(2) && shaped.height.is_multiple_of(2),
                "{} of {start:?} gave {shaped:?}",
                target.label
            );
            assert!(target.matches(shaped) || start.width.min(start.height) % 2 == 1);
        }
        assert!(
            !target.file_suffix.contains(':'),
            "{} cannot name a file",
            target.label
        );
    }
    assert_eq!(
        shape("9:16").applied_to(Resolution::HD_1080),
        Resolution::new(1080, 1920)
    );
    for size in [
        Resolution::HD_1080,
        Resolution::new(1080, 1920),
        Resolution::new(1080, 1080),
    ] {
        assert_eq!(
            SHAPES.iter().filter(|s| s.matches(size)).count(),
            1,
            "{size:?}"
        );
    }
}
