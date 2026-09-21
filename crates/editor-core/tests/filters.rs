//! One-click filters (`bettercut_editor_core::filters`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipProperty;
use bettercut_editor_core::filters::Filter;
use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn two_clips() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Filters");
    for name in ["a", "b"] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(3),
        ));
        editor.place_media(media).unwrap();
    }
    let clips = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .to_vec();
    (editor, clips[0].id, clips[1].id)
}

/// A filter lands on every clip given, in one undo step, and is recognised
/// until a control is tuned by hand.
#[test]
fn a_filter_is_one_step_and_is_recognised() {
    let (mut editor, a, b) = two_clips();
    assert_eq!(editor.filter_of(a), Some(Filter::Original));
    let depth = editor.undo_depth();

    assert_eq!(
        editor.apply_filter(Filter::BlackAndWhite, [a, b]).unwrap(),
        2
    );
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.video_clip(a).unwrap().color.saturation, 0.0);
    assert_eq!(editor.filter_of(b), Some(Filter::BlackAndWhite));

    editor
        .set_clip_property(a, ClipProperty::Contrast(1.3), false)
        .unwrap();
    assert_eq!(
        editor.filter_of(a),
        None,
        "a tuned look still claims to be the filter"
    );

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(editor.filter_of(a), Some(Filter::Original));
}

/// Every filter replaces the last one completely: choosing Warm after VHS
/// leaves no VHS colour bleed behind, and Original puts everything back.
#[test]
fn a_filter_fully_replaces_another() {
    let (mut editor, a, _) = two_clips();
    editor.apply_filter(Filter::Vhs, [a]).unwrap();
    assert!(editor.video_clip(a).unwrap().rgb_split > 0.0);
    editor.apply_filter(Filter::Warm, [a]).unwrap();
    assert_eq!(editor.video_clip(a).unwrap().rgb_split, 0.0);
    assert_eq!(editor.filter_of(a), Some(Filter::Warm));
    editor.apply_filter(Filter::Original, [a]).unwrap();
    assert_eq!(editor.filter_of(a), Some(Filter::Original));
}

/// Each filter is recognisably itself: no two set the same look.
#[test]
fn every_filter_is_distinct() {
    for (i, one) in Filter::ALL.iter().enumerate() {
        for other in &Filter::ALL[i + 1..] {
            assert_ne!(
                one.properties(),
                other.properties(),
                "{one:?} and {other:?} look alike"
            );
        }
    }
}

/// The filter leaves what is not a look alone: framing and opacity stay.
#[test]
fn a_filter_leaves_framing_alone() {
    let (mut editor, a, _) = two_clips();
    editor
        .set_clip_property(a, ClipProperty::Opacity(0.5), false)
        .unwrap();
    editor.apply_filter(Filter::Cinematic, [a]).unwrap();
    assert_eq!(editor.video_clip(a).unwrap().opacity, 0.5);
}

/// Sepia and duotone are tones: set by the filter, recognised, and taken off
/// again by Original.
#[test]
fn sepia_and_duotone_set_a_tone_that_original_clears() {
    use bettercut_editor_core::filters::Filter;
    use bettercut_editor_core::timeline::curves::Tone;

    let (mut editor, _events) = bettercut_editor_core::Editor::new_project("Tones");
    let media = editor.import_media(bettercut_editor_core::media::MediaAsset::new(
        bettercut_editor_core::media::MediaKind::Video,
        "C:/media/old.mp4",
        bettercut_editor_core::foundation::MediaTime::from_seconds(3),
    ));
    let clip = editor.place_media(media).unwrap()[0];

    editor.apply_filter(Filter::Sepia, [clip]).unwrap();
    assert_eq!(editor.video_clip(clip).unwrap().curves.tone, Tone::Sepia);
    assert!(editor.video_clip(clip).unwrap().effective_lut().is_some());
    assert_eq!(editor.filter_of(clip), Some(Filter::Sepia));

    editor.apply_filter(Filter::Duotone, [clip]).unwrap();
    assert_eq!(editor.video_clip(clip).unwrap().curves.tone, Tone::DUOTONE);
    assert_eq!(editor.filter_of(clip), Some(Filter::Duotone));

    editor.apply_filter(Filter::Original, [clip]).unwrap();
    assert!(editor.video_clip(clip).unwrap().curves.tone.is_none());
    assert!(editor.video_clip(clip).unwrap().effective_lut().is_none());
}
