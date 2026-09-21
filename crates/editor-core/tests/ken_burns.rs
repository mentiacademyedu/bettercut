//! Movement on stills: how far it travels, and a montage of them at once
//! (`Editor::set_movement_at`, `Editor::set_movement_on`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, Movement, MovementStrength};

/// `count` photos, one after another.
fn photos(count: usize) -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Montage");
    let clips = (0..count)
        .map(|n| {
            let media = editor.import_media(MediaAsset::new(
                MediaKind::Image,
                format!("C:/media/{n}.jpg"),
                MediaTime::ZERO,
            ));
            editor.place_media(media).unwrap()[0]
        })
        .collect();
    (editor, clips)
}

/// The scale a movement ends on, which is how far it travelled.
fn ends_at(editor: &Editor, clip: ClipId) -> f32 {
    let keys = editor
        .video_clip(clip)
        .unwrap()
        .keyframes
        .track(AnimatedParameter::ScaleX)
        .expect("a scale animation")
        .keys()
        .to_vec();
    keys.last().expect("a last key").value
}

#[test]
fn a_gentler_zoom_travels_less_and_a_stronger_one_more() {
    let (mut editor, clips) = photos(3);
    let mut travel = Vec::new();

    for (clip, strength) in clips.iter().zip(MovementStrength::ALL) {
        editor
            .set_movement_at(*clip, Movement::ZoomIn, strength)
            .expect("a zoom");
        travel.push(ends_at(&editor, *clip));
    }

    assert!(
        travel[0] < travel[1] && travel[1] < travel[2],
        "gentle, normal and strong should differ: {travel:?}"
    );
    // A gentle zoom still zooms, and a strong one is not a jump.
    assert!(travel[0] > 1.0, "{:?}", travel[0]);
    assert!(travel[2] < 1.5, "{:?}", travel[2]);
}

/// A pan is drawn exactly as enlarged as its slide needs, at any strength —
/// or an edge of the picture would come into frame.
#[test]
fn a_pan_is_enlarged_by_what_its_slide_needs() {
    for strength in MovementStrength::ALL {
        let (mut editor, clips) = photos(1);
        editor
            .set_movement_at(clips[0], Movement::PanLeft, strength)
            .expect("a pan");

        let clip = editor.video_clip(clips[0]).unwrap();
        let scale = clip
            .keyframes
            .track(AnimatedParameter::ScaleX)
            .expect("a scale")
            .keys()[0]
            .value;
        let slide = clip
            .keyframes
            .track(AnimatedParameter::PositionX)
            .expect("a slide")
            .keys()
            .to_vec();
        let travel = (slide[0].value - slide[1].value).abs();

        // The picture is `scale` wide; what hangs off each side is half the
        // extra, and the slide uses all of it.
        assert!(
            (travel - (scale - 1.0)).abs() < 1e-4,
            "{}: slid {travel} with {scale} to work with",
            strength.label()
        );
    }
}

#[test]
fn a_montage_can_be_given_one_movement_at_once() {
    let (mut editor, clips) = photos(4);

    let given = editor
        .set_movement_on(&clips, Movement::ZoomIn, MovementStrength::Normal, false)
        .expect("movements");

    assert_eq!(given, 4);
    for clip in &clips {
        assert_eq!(editor.movement_of(*clip), Some(Movement::ZoomIn));
    }
}

/// Alternating is what keeps a montage from reading as a conveyor belt.
#[test]
fn alternating_turns_every_other_one_the_other_way() {
    let (mut editor, clips) = photos(4);

    editor
        .set_movement_on(&clips, Movement::PanLeft, MovementStrength::Gentle, true)
        .expect("movements");

    let moves: Vec<Option<Movement>> = clips.iter().map(|c| editor.movement_of(*c)).collect();
    assert_eq!(
        moves,
        vec![
            Some(Movement::PanLeft),
            Some(Movement::PanRight),
            Some(Movement::PanLeft),
            Some(Movement::PanRight),
        ]
    );
}

#[test]
fn every_movement_has_an_opposite() {
    for movement in Movement::ALL {
        let back = movement.reversed().reversed();
        assert_eq!(back, movement, "{}", movement.label());
        if movement != Movement::None {
            assert_ne!(movement.reversed(), movement, "{}", movement.label());
        }
    }
}

/// A clip that will not take the movement is skipped rather than stopping the
/// rest — one shake in a montage of twenty.
#[test]
fn a_clip_that_refuses_does_not_stop_the_others() {
    let (mut editor, clips) = photos(3);
    editor
        .set_shake(
            clips[1],
            Some(bettercut_editor_core::timeline::ShakeStrength::Gentle),
        )
        .expect("a shake");

    let given = editor
        .set_movement_on(&clips, Movement::PanLeft, MovementStrength::Normal, false)
        .expect("movements");

    assert_eq!(given, 2, "the shaken one should have been skipped");
    assert_eq!(editor.movement_of(clips[0]), Some(Movement::PanLeft));
    assert_eq!(editor.movement_of(clips[2]), Some(Movement::PanLeft));
}
