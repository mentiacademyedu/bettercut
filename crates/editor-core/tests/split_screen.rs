//! Split-screen layouts (`bettercut_editor_core::split_screen`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::split_screen::frame_in_cell;
use bettercut_editor_core::timeline::{
    AnimatedParameter, Interpolation, Keyframe, Resolution, fit_scale,
};
use bettercut_editor_core::{ClipPayload, Editor, EditorError, SplitLayout};

/// Where a framed picture lands, as `[left, top, width, height]` of the frame —
/// the same arithmetic the renderer and the preview handles use.
fn landed(source_aspect: f32, output_aspect: f32, layout_cell: [f32; 4]) -> [f32; 4] {
    let (crop, scale, [x, y]) = frame_in_cell(source_aspect, output_aspect, layout_cell);
    let shown_aspect = crop.applied_to(source_aspect);
    let (fit_x, fit_y) = fit_scale(shown_aspect, output_aspect);
    let (w, h) = (fit_x * scale, fit_y * scale);
    [0.5 + x - w / 2.0, 0.5 + y - h / 2.0, w, h]
}

/// **Every cell is filled exactly** — no bars inside it, nothing spilling into
/// the next — for landscape and vertical footage, in landscape and vertical
/// sequences, in every layout. And the cells cover the frame between them.
#[test]
fn every_picture_fills_its_cell_exactly() {
    for output in [16.0 / 9.0, 9.0 / 16.0, 1.0_f32] {
        for source in [16.0 / 9.0, 9.0 / 16.0, 4.0 / 3.0_f32] {
            for layout in SplitLayout::ALL {
                let mut area = 0.0;
                for cell in layout.cells() {
                    let got = landed(source, output, *cell);
                    for i in 0..4 {
                        assert!(
                            (got[i] - cell[i]).abs() < 1e-4,
                            "{layout:?}, source {source}, frame {output}: {got:?} is not {cell:?}"
                        );
                    }
                    area += cell[2] * cell[3];
                }
                assert!(
                    (area - 1.0).abs() < 1e-5,
                    "{layout:?} leaves part of the frame empty"
                );
            }
        }
    }
}

/// Two videos on two lanes of a landscape sequence.
fn two_lanes() -> (
    Editor,
    bettercut_editor_core::foundation::ClipId,
    bettercut_editor_core::foundation::ClipId,
) {
    let (mut editor, _events) = Editor::new_project("Split");
    editor
        .set_sequence_format(
            Resolution::HD_1080,
            bettercut_editor_core::foundation::FrameRate::FPS_30,
        )
        .unwrap();
    let mut clips = Vec::new();
    for name in ["low", "high"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(5),
        );
        asset.width = 1920;
        asset.height = 1080;
        let media = editor.import_media(asset);
        clips.push(media);
    }
    let low_track = editor.active_sequence().unwrap().video_tracks[0].id;
    let low = bettercut_editor_core::timeline::VideoClip::new(
        clips[0],
        TimelineTime::ZERO,
        bettercut_editor_core::timeline::SourceRange::new(
            MediaTime::ZERO,
            MediaTime::from_seconds(5),
        )
        .unwrap(),
    )
    .unwrap();
    let low_id = low.id;
    editor
        .add_clip(low_track, ClipPayload::Video(Box::new(low)))
        .unwrap();
    editor.add_video_track("V2").unwrap();
    let high_track = editor.active_sequence().unwrap().video_tracks[1].id;
    let high = bettercut_editor_core::timeline::VideoClip::new(
        clips[1],
        TimelineTime::ZERO,
        bettercut_editor_core::timeline::SourceRange::new(
            MediaTime::ZERO,
            MediaTime::from_seconds(5),
        )
        .unwrap(),
    )
    .unwrap();
    let high_id = high.id;
    editor
        .add_clip(high_track, ClipPayload::Video(Box::new(high)))
        .unwrap();
    (editor, low_id, high_id)
}

/// Side by side: the clip on the higher lane reads first, on the left; both
/// are cropped and halved; and one undo puts them back.
#[test]
fn the_higher_lane_goes_first_and_one_undo_restores() {
    let (mut editor, low, high) = two_lanes();
    let before = (
        editor.video_clip(low).unwrap().clone(),
        editor.video_clip(high).unwrap().clone(),
    );

    editor
        .apply_split_screen(&[low, high], SplitLayout::SideBySide)
        .unwrap();

    let (l, h) = (
        editor.video_clip(low).unwrap(),
        editor.video_clip(high).unwrap(),
    );
    assert!(
        h.transform.position.x < 0.0,
        "the higher lane is not on the left"
    );
    assert!(l.transform.position.x > 0.0);
    assert!(
        !h.crop.is_none() && !l.crop.is_none(),
        "landscape shots in half-width cells were not cropped"
    );
    assert_eq!(
        editor.undo_label().as_deref(),
        Some("Split Screen: Side by side")
    );

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(low).unwrap(), &before.0);
    assert_eq!(editor.video_clip(high).unwrap(), &before.1);
}

/// The wrong number of clips is refused, saying how many the layout takes.
#[test]
fn the_wrong_number_of_clips_is_refused() {
    let (mut editor, low, high) = two_lanes();
    let label = editor.undo_label();

    let result = editor.apply_split_screen(&[low, high], SplitLayout::Grid);

    assert!(
        matches!(
            result,
            Err(EditorError::SplitScreenCount {
                wanted: 4,
                given: 2
            })
        ),
        "{result:?}"
    );
    assert_eq!(editor.undo_label(), label);
}

/// A clip whose position or scale is animated would have the layout overridden
/// by its own keys, so it is refused rather than silently ignored.
#[test]
fn an_animated_clip_is_refused() {
    let (mut editor, low, high) = two_lanes();
    let track = editor.track_of(high).unwrap();
    let sequence = editor.active_sequence().unwrap().id;
    editor
        .dispatch(bettercut_editor_core::Command::SetKeyframe {
            sequence,
            track,
            clip: high,
            parameter: AnimatedParameter::ScaleX,
            key: Keyframe::new(MediaTime::ZERO, 1.2, Interpolation::Linear),
        })
        .unwrap();

    let result = editor.apply_split_screen(&[low, high], SplitLayout::SideBySide);

    assert!(
        matches!(result, Err(EditorError::AlreadyAnimated(_))),
        "{result:?}"
    );
    assert!(
        editor.video_clip(low).unwrap().crop.is_none(),
        "the other clip was changed anyway"
    );
}
