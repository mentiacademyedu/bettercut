//! Speed ramps through the editor (`bettercut_editor_core::speed_ramp`).
//!
//! A ramp is a clip cut into pieces, each at its own speed. What it must not
//! do is what a careless version would: lose or repeat material at a cut,
//! leave gaps between the pieces, re-time the picture and not its sound, or
//! take several undos to take back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, Rational, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, EditorError, SpeedRamp};

/// A ten-second video with sound, placed at the start: picture on V1, sound on
/// A1, linked — and a second clip after it on V1 to be pushed around.
fn editor_with_a_linked_pair() -> (Editor, ClipId, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Ramp");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    assert_eq!(placed.len(), 2, "setup: a video with sound is two clips");

    let other = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/b.mp4",
        MediaTime::from_seconds(10),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let after = VideoClip::new(
        other,
        TimelineTime::from_seconds(10),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap(),
    )
    .unwrap();
    let after_id = after.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(after)))
        .unwrap();
    (editor, placed[0], placed[1], after_id)
}

/// `(start, end, source start, source end, speed)` of every clip on a video
/// track, in timeline order.
fn video_lane(editor: &Editor, lane: usize) -> Vec<(i64, i64, i64, i64, Rational)> {
    editor.active_sequence().unwrap().video_tracks[lane]
        .clips()
        .iter()
        .map(|c| {
            (
                c.timeline.start.ticks(),
                c.timeline.end.ticks(),
                c.source.start.ticks(),
                c.source.end.ticks(),
                c.speed,
            )
        })
        .collect()
}

fn audio_lane(editor: &Editor, lane: usize) -> Vec<(i64, i64, i64, i64, Rational)> {
    editor.active_sequence().unwrap().audio_tracks[lane]
        .clips()
        .iter()
        .map(|c| {
            (
                c.timeline.start.ticks(),
                c.timeline.end.ticks(),
                c.source.start.ticks(),
                c.source.end.ticks(),
                c.speed,
            )
        })
        .collect()
}

/// The pieces play every frame of the original, once, in order: each picks up
/// the material exactly where the one before left off, and together they end
/// where the clip ended.
#[test]
fn a_ramp_plays_the_same_material_in_back_to_back_pieces() {
    let (mut editor, picture, _, _) = editor_with_a_linked_pair();
    let before = video_lane(&editor, 0);

    let pieces = editor
        .apply_speed_ramp(picture, SpeedRamp::Montage)
        .unwrap();

    assert_eq!(pieces.len(), SpeedRamp::Montage.factors().len());
    let lane = video_lane(&editor, 0);
    let ramp = &lane[..pieces.len()];
    assert_eq!(ramp[0].0, before[0].0, "the ramp moved its start");
    assert_eq!(
        ramp[0].2, before[0].2,
        "the ramp skipped its first material"
    );
    assert_eq!(
        ramp.last().unwrap().3,
        before[0].3,
        "the ramp lost the end of the material"
    );
    for pair in ramp.windows(2) {
        assert_eq!(pair[0].1, pair[1].0, "a gap or overlap between pieces");
        assert_eq!(pair[0].3, pair[1].2, "material lost or repeated at a cut");
    }
}

/// Each piece at the speed the curve gives it, and each piece's length is its
/// material at that speed — a piece that kept its old length would freeze or
/// cut off.
#[test]
fn each_piece_plays_at_the_curves_speed() {
    let (mut editor, picture, _, _) = editor_with_a_linked_pair();

    let pieces = editor.apply_speed_ramp(picture, SpeedRamp::Bullet).unwrap();

    let lane = video_lane(&editor, 0);
    for ((start, end, from, to, speed), factor) in
        lane[..pieces.len()].iter().zip(SpeedRamp::Bullet.factors())
    {
        assert_eq!(speed, factor);
        let expected = bettercut_editor_core::timeline::timeline_ticks_for(
            MediaTime::from_ticks(to - from),
            *speed,
        );
        assert_eq!(end - start, expected, "a piece's length ignores its speed");
    }
}

/// Relative to the speed the clip already had: a ramp on a 2× clip is twice
/// as fast throughout, not dragged back to normal speed.
#[test]
fn a_ramp_is_relative_to_the_clips_speed() {
    let (mut editor, picture, _, _) = editor_with_a_linked_pair();
    editor
        .set_clip_speed(picture, Rational::new(2, 1).unwrap(), false)
        .unwrap();

    let pieces = editor
        .apply_speed_ramp(picture, SpeedRamp::FlashIn)
        .unwrap();

    let lane = video_lane(&editor, 0);
    let speeds: Vec<Rational> = lane[..pieces.len()].iter().map(|c| c.4).collect();
    let expected: Vec<Rational> = SpeedRamp::FlashIn
        .factors()
        .iter()
        .map(|f| Rational::new(f.num() * 2, f.den()).unwrap())
        .collect();
    assert_eq!(speeds, expected);
}

/// §12: the sound is cut at the same instants and re-timed to the same
/// speeds, so it is still under its picture at every piece.
#[test]
fn the_sound_ramps_with_its_picture() {
    let (mut editor, picture, _, _) = editor_with_a_linked_pair();

    let pieces = editor.apply_speed_ramp(picture, SpeedRamp::Hero).unwrap();

    let video = video_lane(&editor, 0);
    let audio = audio_lane(&editor, 0);
    assert_eq!(
        audio.len(),
        pieces.len(),
        "the sound was not cut into pieces"
    );
    assert_eq!(
        &video[..pieces.len()],
        &audio[..],
        "picture and sound drifted apart"
    );
    for piece in &pieces {
        assert_eq!(
            editor.linked_with(*piece).len(),
            2,
            "a piece lost its sound partner"
        );
    }
}

/// Starting from the sound clip does the same, because the link is symmetric.
#[test]
fn ramping_the_sound_ramps_the_picture() {
    let (mut editor, _, sound, _) = editor_with_a_linked_pair();

    editor.apply_speed_ramp(sound, SpeedRamp::Montage).unwrap();

    let pieces = SpeedRamp::Montage.factors().len();
    assert_eq!(
        &video_lane(&editor, 0)[..pieces],
        &audio_lane(&editor, 0)[..]
    );
}

/// A clip after the ramp moves with the ramp's new end, as a speed change moves
/// it, rather than being overlapped or left behind a gap.
#[test]
fn the_rest_of_the_track_follows_the_ramp() {
    let (mut editor, picture, _, after) = editor_with_a_linked_pair();

    let pieces = editor.apply_speed_ramp(picture, SpeedRamp::Bullet).unwrap();

    let lane = video_lane(&editor, 0);
    let ramp_end = lane[pieces.len() - 1].1;
    let clip = editor.video_clip(after).unwrap();
    assert_eq!(clip.timeline.start.ticks(), ramp_end);
    // Bullet is slow on the whole, so it pushed the clip later.
    assert!(clip.timeline.start > TimelineTime::from_seconds(10));
}

/// §79: one undo takes the whole ramp back — every cut, every speed, the
/// pushed clip — and the undo list names it.
#[test]
fn one_undo_takes_the_whole_ramp_back() {
    let (mut editor, picture, _, _) = editor_with_a_linked_pair();
    let (video, audio) = (video_lane(&editor, 0), audio_lane(&editor, 0));

    editor.apply_speed_ramp(picture, SpeedRamp::Hero).unwrap();
    assert_eq!(editor.undo_label().as_deref(), Some("Hero Speed Ramp"));
    editor.undo().unwrap();

    assert_eq!(video_lane(&editor, 0), video);
    assert_eq!(audio_lane(&editor, 0), audio);
    assert!(
        editor.video_clip(picture).is_some(),
        "the original clip did not come back"
    );
}

/// A photo has no motion to ramp, and nothing is cut trying.
#[test]
fn a_photo_is_refused_untouched() {
    let (mut editor, _rx) = Editor::new_project("Ramp");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/p.png",
        MediaTime::ZERO,
    ));
    let picture = editor.place_media(media).unwrap()[0];
    let before = video_lane(&editor, 0);
    let undo = editor.undo_label();

    let result = editor.apply_speed_ramp(picture, SpeedRamp::Montage);

    assert!(
        matches!(result, Err(EditorError::NoMotionToRetime)),
        "{result:?}"
    );
    assert_eq!(video_lane(&editor, 0), before);
    assert_eq!(
        editor.undo_label(),
        undo,
        "a refused ramp left an undo step"
    );
}

/// Fewer frames than pieces: some piece would be empty, so it is refused
/// rather than quietly becoming a shorter ramp than the one asked for.
#[test]
fn a_clip_shorter_than_the_curve_is_refused_untouched() {
    let (mut editor, _rx) = Editor::new_project("Ramp");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let frames = editor.active_sequence().unwrap().ticks_per_frame();
    // Three frames long; every preset has more pieces than that.
    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(frames * 3)).unwrap(),
    )
    .unwrap();
    let id = clip.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    let before = video_lane(&editor, 0);
    let undo = editor.undo_label();

    let result = editor.apply_speed_ramp(id, SpeedRamp::Montage);

    assert!(
        matches!(result, Err(EditorError::TooShortToRamp { pieces: 5 })),
        "{result:?}"
    );
    assert_eq!(video_lane(&editor, 0), before);
    assert_eq!(
        editor.undo_label(),
        undo,
        "a refused ramp left an undo step"
    );
}

/// Every preset actually changes speed somewhere — a curve of all ones would
/// cut the clip up for nothing — and every one keeps inside the speed limits.
#[test]
fn every_preset_is_a_real_ramp_within_limits() {
    use bettercut_editor_core::timeline::{MAX_SPEED, MIN_SPEED};
    for ramp in SpeedRamp::ALL {
        let factors = ramp.factors();
        assert!(
            factors.len() >= 3,
            "{ramp:?} is too coarse to read as a ramp"
        );
        assert!(
            factors.iter().any(|f| *f != factors[0]),
            "{ramp:?} never changes speed"
        );
        for f in factors {
            assert!(
                f.as_f64() >= MIN_SPEED.as_f64() && f.as_f64() <= MAX_SPEED.as_f64(),
                "{ramp:?} leaves the speed limits"
            );
        }
        assert!(!ramp.label().is_empty() && !ramp.description().is_empty());
    }
}

/// A jump cut keeps the clip's own speed either side of one sudden fast
/// skip in the middle.
#[test]
fn a_jump_cut_skips_once_in_the_middle() {
    let factors = SpeedRamp::JumpCut.factors();
    let one = Rational::new(1, 1).unwrap();
    assert_eq!(factors.len(), 5);
    assert!(factors[2].num() > 4 * factors[2].den(), "not a fast skip");
    for (i, factor) in factors.iter().enumerate() {
        if i != 2 {
            assert_eq!(*factor, one, "piece {i}");
        }
    }
    assert!(SpeedRamp::ALL.contains(&SpeedRamp::JumpCut));
}
