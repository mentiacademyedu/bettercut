//! Picture in picture (`bettercut_editor_core::pip`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::pip::{PIP_MARGIN, inset_frame};
use bettercut_editor_core::timeline::Crop;
use bettercut_editor_core::{ClipProperty, Editor, EditorError, PipCorner, PipSize};

const WIDE: f32 = 16.0 / 9.0;

/// Where an inset's edges land, in frame fractions from the top left.
fn edges(source_aspect: f32, output_aspect: f32, corner: PipCorner, size: PipSize) -> [f32; 4] {
    let (scale, [x, y]) = inset_frame(source_aspect, output_aspect, corner, size);
    let (fit_x, fit_y) = bettercut_editor_core::timeline::fit_scale(source_aspect, output_aspect);
    let (w, h) = (fit_x * scale, fit_y * scale);
    [
        0.5 + x - w / 2.0,
        0.5 + y - h / 2.0,
        0.5 + x + w / 2.0,
        0.5 + y + h / 2.0,
    ]
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// Each corner sits the same number of pixels in from both edges it touches,
/// and the inset is the share of the frame its size says.
#[test]
fn an_inset_sits_one_margin_in_from_its_corner() {
    let margin_x = PIP_MARGIN / WIDE;
    for size in PipSize::ALL {
        let [left, top, right, bottom] = edges(WIDE, WIDE, PipCorner::TopLeft, size);
        assert!(near(left, margin_x) && near(top, PIP_MARGIN), "{size:?}");
        assert!(near(right - left, size.share()), "{size:?} width");

        let [_, _, right, bottom2] = edges(WIDE, WIDE, PipCorner::BottomRight, size);
        assert!(near(right, 1.0 - margin_x) && near(bottom2, 1.0 - PIP_MARGIN));
        assert!(
            near(bottom - top, size.share()),
            "a 16:9 inset in 16:9 keeps its shape"
        );
    }
    let [left, top, right, _] = edges(WIDE, WIDE, PipCorner::TopRight, PipSize::Small);
    assert!(near(right, 1.0 - margin_x) && near(top, PIP_MARGIN) && left > 0.5);
    let [left, _, _, bottom] = edges(WIDE, WIDE, PipCorner::BottomLeft, PipSize::Small);
    assert!(near(left, margin_x) && near(bottom, 1.0 - PIP_MARGIN));
}

/// A portrait shot in a landscape frame is sized by its height, so even the
/// large inset stays inside the frame.
#[test]
fn a_portrait_inset_fits_inside_a_landscape_frame() {
    let portrait = 9.0 / 16.0;
    for corner in PipCorner::ALL {
        let [left, top, right, bottom] = edges(portrait, WIDE, corner, PipSize::Large);
        assert!(
            left > 0.0 && top > 0.0 && right < 1.0 && bottom < 1.0,
            "{corner:?}"
        );
        assert!(near(bottom - top, PipSize::Large.share()));
    }
}

fn setup() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Pip");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/face.mp4",
        MediaTime::from_seconds(4),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

/// Applied as one undo step, recognised afterwards, and undone whole.
#[test]
fn an_inset_is_one_step_and_is_recognised() {
    let (mut editor, clip, _) = setup();
    editor
        .set_clip_property(
            clip,
            ClipProperty::Crop(Crop {
                left: 0.2,
                ..Crop::NONE
            }),
            false,
        )
        .unwrap();
    let before = editor.video_clip(clip).unwrap().clone();
    let depth = editor.undo_depth();

    editor
        .apply_picture_in_picture(clip, PipCorner::BottomLeft, PipSize::Large)
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);
    let video = editor.video_clip(clip).unwrap();
    assert!(video.crop.is_none(), "the crop would cut the inset");
    assert_eq!(
        editor.picture_in_picture_of(clip),
        Some((PipCorner::BottomLeft, PipSize::Large))
    );

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(clip).unwrap(), &before);
    assert_eq!(editor.picture_in_picture_of(clip), None);
}

/// Full Frame undoes the layout as an edit of its own.
#[test]
fn full_frame_puts_it_back() {
    let (mut editor, clip, _) = setup();
    editor
        .apply_picture_in_picture(clip, PipCorner::TopRight, PipSize::Small)
        .unwrap();
    editor.reset_to_full_frame(clip).unwrap();
    let t = editor.video_clip(clip).unwrap().transform;
    assert_eq!(
        (t.scale.x, t.scale.y, t.position.x, t.position.y),
        (1.0, 1.0, 0.0, 0.0)
    );
    assert_eq!(editor.picture_in_picture_of(clip), None);
    assert_eq!(editor.undo_label().as_deref(), Some("Full Frame"));
}

/// Refused for sound, and for a clip whose position is animated.
#[test]
fn sound_and_animated_clips_are_refused() {
    let (mut editor, clip, sound) = setup();
    assert!(matches!(
        editor.apply_picture_in_picture(sound, PipCorner::TopLeft, PipSize::Small),
        Err(EditorError::ClipKindMismatch)
    ));

    editor.set_playhead(bettercut_editor_core::foundation::TimelineTime::from_seconds(1));
    editor
        .toggle_keyframe(clip, ClipProperty::Position { x: 0.0, y: 0.0 })
        .unwrap();
    let before = editor.video_clip(clip).unwrap().clone();
    assert!(matches!(
        editor.apply_picture_in_picture(clip, PipCorner::TopLeft, PipSize::Small),
        Err(EditorError::AlreadyAnimated(_))
    ));
    assert_eq!(editor.video_clip(clip).unwrap(), &before);
}
