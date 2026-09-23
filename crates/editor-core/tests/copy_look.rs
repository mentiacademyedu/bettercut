//! Carrying a look from one clip to the rest of them.
//!
//! Setting up a shot — a grade, a mask, an entrance — is minutes of work, and
//! the next twenty clips want the same treatment. Doing it by hand twenty times
//! is the part of editing that makes people give up on an editor.
//!
//! The interesting parts are the boundaries. A look is *not* a framing, so the
//! crop the user set on one shot must not follow it onto another; and a
//! selection on the timeline carries the linked sound with it (§12), which has
//! no look at all and must not fail the paste.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{
    AudioClip, Backdrop, BlendMode, ChromaKey, ClipMotion, ColorAdjust, Mask, MaskShape, Motion,
    MotionKind, SourceRange, VideoClip,
};
use bettercut_editor_core::{ClipPayload, Editor};

/// Two pictures on one track, and a sound beneath them.
fn editor_with_clips() -> (Editor, ClipId, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Looks");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    let first = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let first_id = first.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(first)))
        .unwrap();

    let second = VideoClip::new(media, TimelineTime::from_seconds(4), source).unwrap();
    let second_id = second.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(second)))
        .unwrap();

    let sound = AudioClip::new(media, TimelineTime::ZERO, source).unwrap();
    let sound_id = sound.id;
    let audio_track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .add_clip(audio_track, ClipPayload::Audio(Box::new(sound)))
        .unwrap();

    (editor, first_id, second_id, sound_id)
}

fn clip(editor: &Editor, id: ClipId) -> &VideoClip {
    editor.video_clip(id).expect("a picture clip")
}

/// Dress the first clip up with one of everything.
fn make_up(editor: &mut Editor, id: ClipId) {
    editor
        .set_clip_value(
            id,
            bettercut_editor_core::ClipProperty::Blend(BlendMode::Screen),
            false,
        )
        .unwrap();
    editor
        .set_clip_value(
            id,
            bettercut_editor_core::ClipProperty::Mask(Some(Mask {
                shape: MaskShape::Ellipse,
                ..Mask::default()
            })),
            false,
        )
        .unwrap();
    editor
        .set_clip_value(
            id,
            bettercut_editor_core::ClipProperty::ChromaKey(Some(ChromaKey::default())),
            false,
        )
        .unwrap();
    editor
        .set_clip_value(
            id,
            bettercut_editor_core::ClipProperty::Motion(ClipMotion {
                intro: Some(Motion::new(MotionKind::Spin, TimelineTime::from_seconds(1))),
                outro: None,
            }),
            false,
        )
        .unwrap();
    editor
        .set_clip_value(
            id,
            bettercut_editor_core::ClipProperty::Brightness(0.4),
            false,
        )
        .unwrap();
    editor
        .set_clip_value(
            id,
            bettercut_editor_core::ClipProperty::MotionBlur(true),
            false,
        )
        .unwrap();
}

#[test]
fn a_look_carries_every_effect_onto_another_clip() {
    let (mut editor, first, second, _) = editor_with_clips();
    make_up(&mut editor, first);

    let look = editor.clip_look(first).expect("a look");
    assert_eq!(editor.paste_look(&look, [second]).unwrap(), 1);

    let (from, onto) = (clip(&editor, first), clip(&editor, second));
    assert_eq!(onto.blend, from.blend, "the blend mode did not carry");
    assert_eq!(onto.mask, from.mask, "the mask did not carry");
    assert_eq!(onto.chroma_key, from.chroma_key, "the key did not carry");
    assert_eq!(onto.motion, from.motion, "the animation did not carry");
    assert_eq!(
        onto.color.brightness, from.color.brightness,
        "the grade did not carry"
    );
}

/// A look is not a framing. The crop and the placement are decisions about the
/// individual shot — a wide landscape and a close-up do not want the same one,
/// and silently replacing it would be the paste destroying work rather than
/// saving it.
#[test]
fn pasting_a_look_leaves_the_framing_and_the_timing_alone() {
    let (mut editor, first, second, _) = editor_with_clips();
    make_up(&mut editor, first);
    editor
        .set_clip_value(
            first,
            bettercut_editor_core::ClipProperty::Scale { x: 2.0, y: 2.0 },
            false,
        )
        .unwrap();
    // §22's crop is framing in the same sense, and the one the doc comment
    // above has always named. It is on the source clip so that a paste has
    // something to wrongly carry across.
    editor
        .set_clip_value(
            first,
            bettercut_editor_core::ClipProperty::Crop(bettercut_editor_core::timeline::Crop {
                left: 0.2,
                right: 0.1,
                ..bettercut_editor_core::timeline::Crop::NONE
            }),
            false,
        )
        .unwrap();

    let was = clip(&editor, second).clone();
    let look = editor.clip_look(first).expect("a look");
    editor.paste_look(&look, [second]).unwrap();

    let now = clip(&editor, second);
    assert_eq!(now.transform, was.transform, "the framing was overwritten");
    assert_eq!(now.crop, was.crop, "the crop followed the look across");
    assert!(
        clip(&editor, first).crop.left > 0.0,
        "the source clip is not cropped, so the assertion above is vacuous"
    );
    assert_eq!(now.timeline, was.timeline, "the clip moved");
    assert_eq!(now.source, was.source, "the clip was re-trimmed");
    assert_eq!(now.speed, was.speed, "the speed changed");
}

/// §79: one paste is one undo step, however many properties and clips it
/// touched. Undoing it a property at a time would be unusable.
#[test]
fn a_paste_is_one_undo_step() {
    let (mut editor, first, second, _) = editor_with_clips();
    make_up(&mut editor, first);

    let before = clip(&editor, second).clone();
    let look = editor.clip_look(first).expect("a look");
    editor.paste_look(&look, [second]).unwrap();
    assert_ne!(clip(&editor, second).blend, before.blend);

    editor.undo().unwrap();
    let after = clip(&editor, second);
    assert_eq!(after.blend, before.blend, "one undo did not put it back");
    assert_eq!(after.mask, before.mask);
    assert_eq!(after.motion, before.motion);
    assert_eq!(after.color, before.color);
}

/// §12: selecting clips on the timeline takes their linked sound too. A sound
/// has no look, and refusing the whole paste over one would break the feature
/// exactly where it is most useful.
#[test]
fn a_sound_in_the_selection_is_skipped_rather_than_refused() {
    let (mut editor, first, second, sound) = editor_with_clips();
    make_up(&mut editor, first);

    let look = editor.clip_look(first).expect("a look");
    let taken = editor
        .paste_look(&look, [second, sound])
        .expect("the sound made the paste fail");

    assert_eq!(taken, 1, "the sound was counted as having taken the look");
    assert_eq!(clip(&editor, second).blend, BlendMode::Screen);
}

/// Nothing to paste onto is not an error, and must not leave an undo step that
/// undoes nothing — the user presses undo and watches the editor do nothing.
#[test]
fn pasting_onto_nothing_leaves_no_history_behind() {
    let (mut editor, first, _, sound) = editor_with_clips();
    make_up(&mut editor, first);

    let look = editor.clip_look(first).expect("a look");
    let depth = editor.undo_depth();
    assert_eq!(editor.paste_look(&look, [sound]).unwrap(), 0);
    assert_eq!(
        editor.undo_depth(),
        depth,
        "an empty paste still added a step to the history"
    );
}

/// A sound has no look to copy in the first place.
#[test]
fn a_sound_has_no_look() {
    let (editor, _, _, sound) = editor_with_clips();
    assert!(editor.clip_look(sound).is_none());
}

/// Everything a look carries, checked against the *clip* rather than against
/// another list — and structurally, so a field nobody thought about is covered.
///
/// The kinds test below checks that copying and clearing agree with each other.
/// That is worth having and is not enough on its own: a field missing from
/// *both* lists passes it, which is exactly how motion blur went missing for a
/// tick. A list of assertions here would have the same weakness one level down,
/// so the source clip is written as an **exhaustive** struct literal: adding a
/// field to `VideoClip` stops this compiling until someone has set it, and the
/// comparison then says whether a look should have carried it.
#[test]
fn a_pasted_clip_matches_the_one_it_was_copied_from() {
    let (mut editor, _, second, _) = editor_with_clips();
    let media = clip(&editor, second).media_id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    let plain = VideoClip::new(
        media,
        TimelineTime::from_seconds(20),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    let dressed = VideoClip {
        corner_pin: Default::default(),
        angle: None,
        old_film: 0.0,
        glow: 0.0,
        shadow: Default::default(),
        border: Default::default(),
        sharpen: 45.0,
        rgb_split: 30.0,
        glitch: 12.0,
        pixelate: 20.0,
        zoom_blur: 15.0,
        vignette: 0.25,
        light_leak: 30.0,
        beat_pulse: 40.0,
        smooth_motion: false,
        curves: bettercut_timeline::curves::ColourCurves {
            master: [0.0, 0.2, 0.5, 0.8, 1.0],
            ..Default::default()
        },
        reflection: bettercut_timeline::Reflection::Kaleidoscope,
        // Timing, not look, so not part of what is being compared here.
        reversed: false,
        color_label: bettercut_timeline::ColorLabel::None,
        name: None,
        lut: Some(bettercut_timeline::ClipLut {
            lut: bettercut_foundation::LutId::new(),
            strength: 0.6,
        }),
        crop: bettercut_timeline::Crop::NONE,
        // Every look field, set to something that is not its default.
        opacity: 0.75,
        color: ColorAdjust {
            brightness: 1.1,
            contrast: 1.2,
            saturation: 0.9,
            temperature: 0.4,
            tint: -0.25,
            vibrance: 0.0,
            wheels: Default::default(),
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
        blur: 12.0,
        blend: BlendMode::Screen,
        mask: Some(Mask {
            shape: MaskShape::Ellipse,
            ..Mask::default()
        }),
        chroma_key: Some(ChromaKey::default()),
        luma_key: None,
        motion: ClipMotion {
            intro: Some(Motion::new(MotionKind::Spin, TimelineTime::from_seconds(1))),
            outro: None,
        },
        motion_blur: true,
        backdrop: Backdrop::Blur,

        // Not part of a look: identity, where it sits, what it reads, how fast,
        // its framing, its keys, its cut. Each is tested on its own elsewhere.
        id: plain.id,
        media_id: plain.media_id,
        timeline: plain.timeline,
        source: plain.source,
        transform: plain.transform,
        keyframes: plain.keyframes.clone(),
        speed: plain.speed,
        link: plain.link,
        transition_out: plain.transition_out,
        lens: plain.lens,
        tilt_band: plain.tilt_band,
        tilt_centre: plain.tilt_centre,
        posterise: plain.posterise,
        frozen: plain.frozen,
        enabled: plain.enabled,
    };
    let first = dressed.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(dressed)))
        .unwrap();

    let look = editor.clip_look(first).expect("a look");
    editor.paste_look(&look, [second]).unwrap();

    let source = clip(&editor, first).clone();
    let pasted = clip(&editor, second).clone();
    let expected = VideoClip {
        corner_pin: Default::default(),
        angle: None,
        id: pasted.id,
        media_id: pasted.media_id,
        timeline: pasted.timeline,
        source: pasted.source,
        transform: pasted.transform,
        keyframes: pasted.keyframes.clone(),
        speed: pasted.speed,
        link: pasted.link,
        transition_out: pasted.transition_out,
        lens: pasted.lens,
        tilt_band: pasted.tilt_band,
        tilt_centre: pasted.tilt_centre,
        posterise: pasted.posterise,
        frozen: pasted.frozen,
        enabled: pasted.enabled,
        ..source
    };

    assert_eq!(
        pasted, expected,
        "a look left something behind, or carried something it should not"
    );
}

/// Clearing and copying have to agree about what a look *is*. Two lists that
/// drifted apart would leave whatever the second one forgot still on the clip —
/// a "Clear Look" that quietly left the mask behind.
#[test]
fn a_plain_look_covers_exactly_what_a_copied_one_does() {
    let (mut editor, first, _, _) = editor_with_clips();
    make_up(&mut editor, first);

    let copied: Vec<_> = editor
        .clip_look(first)
        .expect("a look")
        .iter()
        .map(|property| property.kind())
        .collect();
    let plain: Vec<_> = Editor::plain_look()
        .iter()
        .map(|property| property.kind())
        .collect();

    assert_eq!(copied, plain, "the two lists have drifted apart");
}

/// And it really does strip a clip back — to exactly what a clip that was never
/// touched looks like, not merely to something tidier.
#[test]
fn clearing_a_look_leaves_the_clip_as_it_was_made() {
    let (mut editor, first, second, _) = editor_with_clips();
    make_up(&mut editor, first);

    let untouched = clip(&editor, second).clone();
    editor.paste_look(&Editor::plain_look(), [first]).unwrap();

    let cleared = clip(&editor, first);
    assert_eq!(cleared.blend, untouched.blend);
    assert_eq!(cleared.mask, untouched.mask);
    assert_eq!(cleared.chroma_key, untouched.chroma_key);
    assert_eq!(cleared.motion, untouched.motion);
    assert_eq!(cleared.color, untouched.color);
    assert_eq!(cleared.opacity, untouched.opacity);
    assert_eq!(cleared.blur, untouched.blur);
    assert_eq!(cleared.backdrop, untouched.backdrop);
}
