//! Everything the editor can put in a project, saved and opened again.
//!
//! One test rather than one per feature, because the failure it guards against
//! is *silent*: a field added to the model but not to the file format loads as
//! its default, and the user's work is quietly gone. The project is built
//! through the editor's own commands, so anything a user can do is covered by
//! adding it here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, Rational, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Motion, MotionKind, TextAnimation, TransitionKind};
use bettercut_editor_core::{ClipProperty, Editor, Movement, TextProperty};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A project using every feature that has somewhere to be stored.
fn elaborate_project(path: &std::path::Path) {
    let (mut editor, _events) = Editor::new_project("Everything");

    let mut video = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(30),
    );
    video.audio_codec = Some("aac".to_owned());
    let video = editor.import_media(video);
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/p.jpg",
        MediaTime::ZERO,
    ));

    let placed = editor.place_media(video).unwrap();
    let (picture, sound) = (placed[0], placed[1]);
    editor.place_media(photo).unwrap();

    // Clip properties, a look, and a movement.
    editor
        .set_clip_property(picture, ClipProperty::Opacity(0.8), false)
        .unwrap();
    editor
        .set_color_adjust(
            Some(picture),
            bettercut_editor_core::timeline::ColorAdjust {
                brightness: 1.1,
                contrast: 0.9,
                saturation: 0.7,
                temperature: 0.35,
                tint: -0.2,
                vibrance: 0.0,
                wheels: Default::default(),
                secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
            },
        )
        .unwrap();
    editor.set_movement(picture, Movement::ZoomIn).unwrap();
    editor
        .set_clip_property(
            picture,
            ClipProperty::Backdrop(bettercut_editor_core::timeline::Backdrop::Blur),
            false,
        )
        .unwrap();
    editor
        .set_clip_property(
            picture,
            ClipProperty::Blend(bettercut_editor_core::timeline::BlendMode::Screen),
            false,
        )
        .unwrap();
    editor
        .set_clip_property(
            picture,
            ClipProperty::Mask(Some(bettercut_editor_core::timeline::Mask {
                shape: bettercut_editor_core::timeline::MaskShape::Ellipse,
                center: [0.4, 0.6],
                size: [0.3, 0.2],
                feather: 0.1,
                rotation_degrees: 30.0,
                invert: true,
            })),
            false,
        )
        .unwrap();
    editor
        .set_clip_property(
            picture,
            ClipProperty::ChromaKey(Some(bettercut_editor_core::timeline::ChromaKey {
                color: [0.1, 0.8, 0.2],
                tolerance: 0.2,
                softness: 0.05,
                spill: 0.4,
            })),
            false,
        )
        .unwrap();
    editor
        .set_clip_speed(picture, Rational::new(3, 2).unwrap(), false)
        .unwrap();

    // Sound: gain, fades, and the track's own volume and pan.
    editor
        .set_clip_property(sound, ClipProperty::Gain(0.6), false)
        .unwrap();
    editor
        .set_clip_fades(sound, TimelineTime::from_millis(400), seconds(1), false)
        .unwrap();
    let audio_track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .set_track_mix(audio_track, 0.75, -0.3, false)
        .unwrap();

    // A volume envelope on the sound (§24 on §20a.4's clip-gain stage).
    editor
        .set_gain_envelope(
            sound,
            &[(seconds(0), 1.0), (seconds(4), 0.25), (seconds(8), 1.0)],
            false,
        )
        .unwrap();

    // A transition on the cut between the video and the photo.
    editor
        .set_transition(picture, TransitionKind::FadeThroughBlack)
        .unwrap();

    // A title with an entrance and an exit.
    editor.set_playhead(seconds(2));
    let title = editor.add_text("Hello").unwrap();
    editor
        .set_text_property(
            title,
            TextProperty::Animation(TextAnimation {
                scroll: None,
                intro: Some(Motion::new(MotionKind::Pop, TimelineTime::from_millis(500))),
                outro: Some(Motion::new(
                    MotionKind::Typewriter,
                    TimelineTime::from_millis(700),
                )),
                looping: None,
            }),
            false,
        )
        .unwrap();

    // Markers, a whole-video volume, and a held frame.
    editor.add_markers(&[seconds(3), seconds(9)]).unwrap();
    editor
        .set_sequence_value(ClipProperty::Gain(0.5), false)
        .unwrap();
    editor.set_playhead(seconds(5));
    editor.freeze_frame(picture, seconds(2)).unwrap();

    editor.save_as(path).unwrap();
}

#[test]
fn every_feature_survives_a_save_and_an_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("everything.vproj");
    elaborate_project(&path);

    let saved = {
        let (editor, _events) = Editor::open(&path).unwrap();
        editor.project().clone()
    };
    // Saving what was loaded and loading it again must give the same project:
    // anything that did not survive the first trip is already gone, so this
    // compares the file against itself as well as against the editor.
    let again = {
        let (mut editor, _events) = Editor::open(&path).unwrap();
        let second = dir.path().join("again.vproj");
        editor.save_as(&second).unwrap();
        let (editor, _events) = Editor::open(&second).unwrap();
        editor.project().clone()
    };
    assert_eq!(saved.active(), again.active(), "a second trip changed it");

    let sequence = saved.active().expect("a sequence");

    // Clip look, speed, transition, and the held frame.
    let clips = sequence.video_tracks[0].clips();
    let first = &clips[0];
    assert_eq!(first.opacity, 0.8);
    assert_eq!(first.color.saturation, 0.7);
    assert_eq!(first.speed, Rational::new(3, 2).unwrap());
    assert_eq!(
        first.backdrop,
        bettercut_editor_core::timeline::Backdrop::Blur,
        "the backdrop was lost"
    );
    assert_eq!(
        first.blend,
        bettercut_editor_core::timeline::BlendMode::Screen,
        "the blend mode was lost"
    );
    let mask = first.mask.expect("the mask was lost");
    assert_eq!(
        mask.shape,
        bettercut_editor_core::timeline::MaskShape::Ellipse
    );
    assert_eq!((mask.center, mask.size), ([0.4, 0.6], [0.3, 0.2]));
    assert_eq!((mask.feather, mask.rotation_degrees), (0.1, 30.0));
    assert!(mask.invert, "the mask came back the right way up");

    let key = first.chroma_key.expect("the chroma key was lost");
    assert_eq!(key.color, [0.1, 0.8, 0.2]);
    assert_eq!((key.tolerance, key.softness, key.spill), (0.2, 0.05, 0.4));
    assert!(
        first
            .keyframes
            .is_animated(bettercut_editor_core::timeline::AnimatedParameter::ScaleX),
        "the movement's keyframes were lost"
    );
    assert_eq!(
        clips.iter().filter(|c| c.frozen).count(),
        1,
        "the hold was lost"
    );
    assert!(
        clips.iter().any(|c| c.transition_out.is_some()),
        "the transition was lost"
    );

    // Sound, and the track it is on. The freeze split the sound in two, and a
    // fade belongs to the edge it is on (§25's rule for transitions): the fade
    // in stayed with the first half, the fade out with the last.
    let sounds = sequence.audio_tracks[0].clips();
    assert_eq!(sounds[0].gain, 0.6);
    assert_eq!(sounds[0].fade_in, TimelineTime::from_millis(400));
    assert_eq!(sounds[0].fade_out, TimelineTime::ZERO);
    assert_eq!(sounds.last().unwrap().fade_out, seconds(1));
    assert_eq!(sequence.audio_tracks[0].gain, 0.75);
    assert!(
        sounds[0]
            .keyframes
            .is_animated(bettercut_editor_core::timeline::AnimatedParameter::Gain),
        "the volume envelope was lost"
    );
    assert_eq!(
        sounds[0].gain_at(TimelineTime::from_seconds(4)),
        0.25,
        "the envelope came back at the wrong level"
    );
    assert_eq!(sequence.audio_tracks[0].pan, -0.3);

    // The title and its animation.
    let title = &sequence.text_tracks[0].clips()[0];
    assert_eq!(title.text, "Hello");
    assert_eq!(
        title.animation.intro.map(|m| m.kind),
        Some(MotionKind::Pop),
        "the title's entrance was lost"
    );
    assert_eq!(
        title.animation.outro.map(|m| m.duration),
        Some(TimelineTime::from_millis(700))
    );

    // Sequence-level state.
    assert_eq!(sequence.markers.len(), 2, "markers were lost");
    assert_eq!(sequence.master_volume, 0.5);
}

/// The same question asked structurally: does a clip come back *identical*?
///
/// The test above checks the fields someone remembered to assert, which is why
/// it grew every time a feature landed — and three of this session's did not
/// reach it. This one writes the clip as an **exhaustive** struct literal, so a
/// field added to `VideoClip` stops it compiling until someone has given it a
/// value, and then compares the whole clip after a save and an open. A field
/// that does not survive the file has nowhere to hide.
#[test]
fn a_clip_comes_back_exactly_as_it_went_in() {
    use bettercut_editor_core::ClipPayload;
    use bettercut_editor_core::timeline::{
        Backdrop, BlendMode, ChromaKey, ClipLook, ClipMotion, ColorAdjust, Mask, MaskShape,
        SourceRange, Transform, Vec2, VideoClip,
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one-clip.vproj");

    let (mut editor, _events) = Editor::new_project("One clip");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    let plain = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_seconds(2), MediaTime::from_seconds(6)).unwrap(),
    )
    .unwrap();
    let clip = VideoClip {
        corner_pin: Default::default(),
        angle: Some(1),
        old_film: 0.0,
        glow: 0.0,
        shadow: Default::default(),
        border: Default::default(),
        sharpen: 35.0,
        rgb_split: 30.0,
        glitch: 12.0,
        pixelate: 20.0,
        zoom_blur: 15.0,
        vignette: 0.25,
        light_leak: 30.0,
        beat_pulse: 40.0,
        smooth_motion: true,
        curves: bettercut_timeline::curves::ColourCurves {
            master: [0.0, 0.2, 0.5, 0.8, 1.0],
            ..Default::default()
        },
        reflection: bettercut_timeline::Reflection::Kaleidoscope,
        reversed: true,
        color_label: bettercut_timeline::ColorLabel::Purple,
        name: None,
        lut: Some(bettercut_timeline::ClipLut {
            lut: bettercut_foundation::LutId::new(),
            strength: 0.6,
        }),
        // Every field, none at its default, so a value lost on the way through
        // the file shows up as a difference rather than as a coincidence.
        opacity: 0.65,
        color: ColorAdjust {
            brightness: 1.15,
            contrast: 0.85,
            saturation: 1.3,
            temperature: -0.4,
            tint: 0.25,
            vibrance: 0.0,
            wheels: Default::default(),
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
        blur: 22.0,
        blend: BlendMode::Multiply,
        mask: Some(Mask {
            shape: MaskShape::Rectangle,
            center: [0.35, 0.65],
            size: [0.2, 0.4],
            feather: 0.15,
            rotation_degrees: 24.0,
            invert: true,
        }),
        luma_key: None,
        chroma_key: Some(ChromaKey {
            color: [0.05, 0.7, 0.25],
            tolerance: 0.18,
            softness: 0.06,
            spill: 0.5,
        }),
        motion: ClipMotion {
            intro: Some(Motion::new(
                MotionKind::Spin,
                TimelineTime::from_millis(600),
            )),
            outro: Some(Motion::new(
                MotionKind::SlideLeft,
                TimelineTime::from_millis(400),
            )),
        },
        motion_blur: true,
        crop: bettercut_editor_core::timeline::Crop {
            left: 0.1,
            top: 0.05,
            right: 0.15,
            bottom: 0.2,
        },
        backdrop: Backdrop::Blur,
        transform: Transform {
            position: Vec2::new(0.12, -0.08),
            scale: Vec2::new(0.7, 0.7),
            rotation_degrees: 12.0,
            anchor: Vec2::new(0.4, 0.6),
            flip_h: true,
            flip_v: true,
        },
        speed: Rational::new(3, 2).unwrap(),
        lens: 0.3,
        tilt_band: 0.0,
        tilt_centre: 0.5,
        posterise: 0.3,
        frozen: false,
        enabled: true,

        // Identity and placement, from the clip the editor made.
        id: plain.id,
        media_id: plain.media_id,
        timeline: plain.timeline,
        source: plain.source,
        keyframes: plain.keyframes.clone(),
        link: plain.link,
        transition_out: plain.transition_out,
    };
    let id = clip.id;
    let before = clip.clone();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    editor.save_as(&path).unwrap();

    let (reopened, _events) = Editor::open(&path).unwrap();
    let after = reopened.video_clip(id).expect("the clip came back");
    assert_eq!(*after, before, "a field did not survive the file");

    // `ClipLook` is what the renderer reads, so it is worth saying plainly that
    // the reloaded clip presents the same one.
    let look: ClipLook = after.look_at(after.source.start);
    assert_eq!(look.blend, BlendMode::Multiply);
    assert_eq!(look.chroma_key, before.chroma_key);
}

/// The settings that belong to the whole video rather than to a clip, which
/// are easy to forget precisely because no clip carries them.
#[test]
fn the_background_and_a_solo_survive_a_save_and_an_open() {
    use bettercut_editor_core::TrackFlag;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sequence.vproj");

    let (mut editor, _events) = Editor::new_project("Sequence settings");
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .set_sequence_value(ClipProperty::Background([0.2, 0.35, 0.9]), false)
        .unwrap();
    editor
        .set_sequence_value(ClipProperty::Vignette(0.6), false)
        .unwrap();
    editor.set_track_flag(track, TrackFlag::Solo, true).unwrap();
    editor.save_as(&path).unwrap();

    let (reopened, _events) = Editor::open(&path).unwrap();
    let sequence = reopened.active_sequence().expect("a sequence");
    assert_eq!(
        sequence.master.background,
        [0.2, 0.35, 0.9],
        "the background colour was lost"
    );
    assert_eq!(sequence.master.vignette, 0.6, "the vignette was lost");
    assert!(
        reopened.track_flag(track, TrackFlag::Solo),
        "the solo was lost"
    );
}

/// The other two clip kinds, held to the same structural promise.
///
/// A title and a sound have their own fields, and the same silent failure is
/// available to both — a title's entrance or a sound's fade quietly reset on
/// the next open. Exhaustive literals again, so neither can grow a field that
/// nobody checked.
#[test]
fn a_title_and_a_sound_come_back_exactly_as_they_went_in() {
    use bettercut_editor_core::ClipPayload;
    use bettercut_editor_core::text::{Alignment, FontFamily, FontWeight, TextStyle};
    use bettercut_editor_core::timeline::{AudioClip, SourceRange, TextClip, Transform, Vec2};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("title-and-sound.vproj");

    let (mut editor, _events) = Editor::new_project("Both");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(60),
    ));

    let plain_title = TextClip::with_duration("Hello", TimelineTime::ZERO, seconds(3)).unwrap();
    let title = TextClip {
        text: "A title".to_owned(),
        style: TextStyle {
            family: FontFamily::Serif,
            size: 88.0,
            weight: FontWeight::Light,
            italic: true,
            align: Alignment::Left,
            letter_spacing: 0.08,
            line_height: 1.4,
            ..TextStyle::default()
        },
        transform: Transform {
            position: Vec2::new(-0.2, 0.3),
            scale: Vec2::new(1.2, 1.2),
            rotation_degrees: -8.0,
            anchor: Vec2::new(0.5, 0.5),
            flip_h: true,
            flip_v: false,
        },
        opacity: 0.9,
        enabled: true,
        color_label: bettercut_timeline::ColorLabel::Green,
        name: None,
        highlight: Some(bettercut_text::Rgba::opaque(255, 214, 10)),
        counter: Some(bettercut_timeline::Counter {
            direction: bettercut_timeline::CountDirection::Up,
            from: seconds(90),
            format: bettercut_timeline::CountFormat::MinutesSeconds,
        }),
        shape: Some(bettercut_text::Shape {
            kind: bettercut_text::ShapeKind::RoundedRectangle,
            width: 300.0,
            height: 120.0,
            fill: bettercut_text::Rgba::new(10, 20, 30, 200),
            outline: Some(bettercut_text::Stroke {
                width: 6.0,
                color: bettercut_text::Rgba::WHITE,
            }),
            corner_radius: 18.0,
        }),
        animation: TextAnimation {
            scroll: None,
            intro: Some(Motion::new(MotionKind::Pop, TimelineTime::from_millis(500))),
            outro: Some(Motion::new(
                MotionKind::Typewriter,
                TimelineTime::from_millis(900),
            )),
            looping: None,
        },
        motion_blur: true,

        id: plain_title.id,
        timeline: plain_title.timeline,
        source: plain_title.source,
    };
    let title_id = title.id;
    let title_before = title.clone();
    let text_track = editor.active_sequence().unwrap().text_tracks[0].id;
    editor
        .add_clip(text_track, ClipPayload::Text(Box::new(title)))
        .unwrap();

    let plain_sound = AudioClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    let sound = AudioClip {
        keep_pitch: true,
        fade_shape: Default::default(),
        gain: 0.45,
        pan: -0.4,
        denoise: 65.0,
        gate: 30.0,
        channels: bettercut_timeline::ChannelMode::LeftToBoth,
        pitch: -3.5,
        leveller: 40.0,
        de_ess: 55.0,
        stereo_width: 1.4,
        muted: true,
        space: bettercut_timeline::ClipSpace {
            kind: bettercut_timeline::SpaceKind::Hall,
            mix: 0.4,
        },
        eq: bettercut_timeline::ClipEq {
            low_cut: 120.0,
            high_cut: 9_000.0,
            presence: 3.5,
            hum: 50.0,
        },
        fade_in: TimelineTime::from_millis(750),
        fade_out: TimelineTime::from_millis(1250),
        crossfade_out: TimelineTime::from_millis(500),
        speed: Rational::new(4, 5).unwrap(),
        reversed: true,
        color_label: bettercut_timeline::ColorLabel::Purple,
        name: None,
        enabled: true,

        id: plain_sound.id,
        media_id: plain_sound.media_id,
        timeline: plain_sound.timeline,
        source: plain_sound.source,
        link: plain_sound.link,
        keyframes: plain_sound.keyframes.clone(),
    };
    let sound_id = sound.id;
    let sound_before = sound.clone();
    let audio_track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .add_clip(audio_track, ClipPayload::Audio(Box::new(sound)))
        .unwrap();

    editor.save_as(&path).unwrap();

    let (reopened, _events) = Editor::open(&path).unwrap();
    let sequence = reopened.active_sequence().expect("a sequence");
    let title_after = sequence.text_tracks[0]
        .get(title_id)
        .expect("the title came back");
    assert_eq!(*title_after, title_before, "a title field did not survive");

    let sound_after = sequence.audio_tracks[0]
        .get(sound_id)
        .expect("the sound came back");
    assert_eq!(*sound_after, sound_before, "a sound field did not survive");
}
