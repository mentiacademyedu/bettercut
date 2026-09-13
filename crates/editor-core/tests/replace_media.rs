//! Replacing a clip's media (`bettercut_editor_core::replace`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, Rational, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::replace::SoundChange;
use bettercut_editor_core::timeline::ColorLabel;
use bettercut_editor_core::{Editor, EditorError, TrimEdge};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn media_seconds(n: i64) -> MediaTime {
    MediaTime::from_seconds(n)
}

struct Files {
    /// Ten seconds, with sound.
    take_one: MediaId,
    /// Twenty seconds, with sound.
    take_two: MediaId,
    /// Six seconds, silent.
    silent: MediaId,
    photo: MediaId,
    /// Thirty seconds of music, no picture.
    music: MediaId,
}

fn import(editor: &mut Editor, kind: MediaKind, name: &str, length: i64, sound: bool) -> MediaId {
    let mut asset = MediaAsset::new(kind, format!("C:/media/{name}"), media_seconds(length));
    if sound {
        asset.audio_codec = Some("aac".to_owned());
    }
    editor.import_media(asset)
}

fn setup() -> (Editor, Files) {
    let (mut editor, _events) = Editor::new_project("Replace");
    let files = Files {
        take_one: import(&mut editor, MediaKind::Video, "take1.mp4", 10, true),
        take_two: import(&mut editor, MediaKind::Video, "take2.mp4", 20, true),
        silent: import(&mut editor, MediaKind::Video, "silent.mp4", 6, false),
        photo: import(&mut editor, MediaKind::Image, "logo.png", 0, false),
        music: import(&mut editor, MediaKind::Audio, "song.mp3", 30, true),
    };
    (editor, files)
}

/// Place a file and return its picture and sound.
fn place(editor: &mut Editor, media: MediaId) -> (ClipId, Option<ClipId>) {
    let placed = editor.place_media(media).unwrap();
    let sound = placed
        .iter()
        .copied()
        .find(|id| editor.audio_clip(*id).is_some());
    (placed[0], sound)
}

fn trim(editor: &mut Editor, clip: ClipId, edge: TrimEdge, at: i64) {
    let track = editor.track_of(clip).unwrap();
    editor.trim_clip(track, clip, edge, seconds(at)).unwrap();
}

/// The picture and its sound read the new file, from the same in-point, for
/// the same span — and keep everything done to them. One undo puts the old
/// file back under both.
#[test]
fn the_clip_keeps_its_place_and_edits_and_reads_the_new_file() {
    let (mut editor, files) = setup();
    let (picture, sound) = place(&mut editor, files.take_one);
    let sound = sound.unwrap();
    trim(&mut editor, picture, TrimEdge::Start, 2);
    editor.set_color_label(&[picture], ColorLabel::Red).unwrap();
    let before = (
        editor.video_clip(picture).unwrap().clone(),
        editor.audio_clip(sound).unwrap().clone(),
    );

    let change = editor.replace_media(picture, files.take_two).unwrap();
    assert_eq!(change, SoundChange::Replaced);

    let video = editor.video_clip(picture).unwrap();
    assert_eq!(video.media_id, files.take_two);
    assert_eq!(video.source, before.0.source, "the in-point moved");
    assert_eq!(
        video.timeline, before.0.timeline,
        "the clip moved on the timeline"
    );
    assert_eq!(video.color_label, ColorLabel::Red, "an edit was lost");
    let audio = editor.audio_clip(sound).unwrap();
    assert_eq!(
        audio.media_id, files.take_two,
        "the sound kept the old file"
    );
    assert_eq!(audio.source, before.1.source);
    assert!(video.link.is_some() && audio.link == video.link, "unlinked");
    assert_eq!(editor.undo_label().as_deref(), Some("Replace Media"));

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(picture).unwrap(), &before.0);
    assert_eq!(editor.audio_clip(sound).unwrap(), &before.1);
}

/// A file too short for the footage the clip uses is refused, changing
/// nothing; one long enough, but not from the same in-point, gives the
/// stretch nearest to it.
#[test]
fn a_short_file_is_refused_and_a_tight_one_fits_as_near_as_it_can() {
    let (mut editor, files) = setup();
    let (picture, _) = place(&mut editor, files.take_one);
    let before = editor.video_clip(picture).unwrap().clone();
    let depth = editor.undo_depth();

    match editor.replace_media(picture, files.silent) {
        Err(EditorError::ReplacementTooShort { needed, available }) => {
            assert!((needed - 10.0).abs() < 1e-6 && (available - 6.0).abs() < 1e-6);
        }
        other => panic!("expected too short, got {other:?}"),
    }
    assert_eq!(editor.video_clip(picture).unwrap(), &before);
    assert_eq!(editor.undo_depth(), depth);

    // Four seconds of footage, from 5 s in: the six-second file has four
    // seconds, but only from 2 s in.
    trim(&mut editor, picture, TrimEdge::Start, 5);
    trim(&mut editor, picture, TrimEdge::End, 9);
    editor.replace_media(picture, files.silent).unwrap();
    let source = editor.video_clip(picture).unwrap().source;
    assert_eq!(
        (source.start, source.end),
        (media_seconds(2), media_seconds(6))
    );
}

/// A file with no sound takes the old sound away; undo brings it back, tied to
/// its picture again.
#[test]
fn a_silent_file_takes_the_old_sound_away() {
    let (mut editor, files) = setup();
    let (picture, sound) = place(&mut editor, files.take_one);
    let sound = sound.unwrap();
    trim(&mut editor, picture, TrimEdge::End, 5);

    let change = editor.replace_media(picture, files.silent).unwrap();
    assert_eq!(change, SoundChange::Removed);
    assert!(editor.audio_clip(sound).is_none(), "the old sound stayed");
    assert!(editor.link_of(picture).is_none(), "a link to nothing");

    editor.undo().unwrap();
    assert!(editor.audio_clip(sound).is_some());
    assert_eq!(editor.link_of(picture), editor.link_of(sound));
    assert!(editor.link_of(picture).is_some());
}

/// A photo fills the clip's whole length, at normal speed and forwards.
#[test]
fn a_photo_fills_the_clip_at_normal_speed() {
    let (mut editor, files) = setup();
    let (picture, _) = place(&mut editor, files.take_two);
    editor
        .set_clip_speed(picture, Rational::new(2, 1).unwrap(), false)
        .unwrap();
    editor.set_reversed(picture, true).unwrap();
    let span = editor.video_clip(picture).unwrap().timeline;

    editor.replace_media(picture, files.photo).unwrap();
    let video = editor.video_clip(picture).unwrap();
    assert_eq!(video.timeline, span, "the clip changed length");
    assert_eq!(video.speed, Rational::ONE);
    assert!(!video.reversed);
    assert_eq!(video.source.start, MediaTime::ZERO);
    assert_eq!(video.source.duration().ticks(), span.duration().ticks());

    // And back to footage: the photo's length is the footage it needs.
    editor.replace_media(picture, files.take_one).unwrap();
    let video = editor.video_clip(picture).unwrap();
    assert_eq!(video.source.duration().ticks(), span.duration().ticks());
    assert_eq!(video.source.start, MediaTime::ZERO);
}

/// A picture with no sound gains the new file's, tied to it, when there is room.
#[test]
fn a_silent_picture_gains_the_new_files_sound() {
    let (mut editor, files) = setup();
    let (picture, sound) = place(&mut editor, files.silent);
    assert!(sound.is_none());

    let change = editor.replace_media(picture, files.take_two).unwrap();
    assert_eq!(change, SoundChange::Added);
    let video = editor.video_clip(picture).unwrap().clone();
    let partner = editor
        .linked_with(picture)
        .into_iter()
        .find_map(|id| editor.audio_clip(id).cloned())
        .expect("no sound was added");
    assert_eq!(partner.media_id, files.take_two);
    assert_eq!(partner.timeline, video.timeline);
    assert_eq!(partner.source, video.source);

    editor.undo().unwrap();
    assert!(
        editor.active_sequence().unwrap().audio_tracks[0].is_empty(),
        "undo left the added sound"
    );
}

/// A sound clip replaced on its own plays the new sound and is untied from its
/// picture, which keeps its own file.
#[test]
fn replacing_just_the_sound_unties_it() {
    let (mut editor, files) = setup();
    let (picture, sound) = place(&mut editor, files.take_one);
    let sound = sound.unwrap();

    let change = editor.replace_media(sound, files.music).unwrap();
    assert_eq!(change, SoundChange::Unlinked);
    assert_eq!(editor.audio_clip(sound).unwrap().media_id, files.music);
    assert_eq!(editor.video_clip(picture).unwrap().media_id, files.take_one);
    assert!(editor.link_of(sound).is_none());
    assert!(editor.link_of(picture).is_none());
}

/// The wrong kind of file is refused, and only the right kind is offered.
#[test]
fn only_files_with_what_the_clip_needs_are_accepted() {
    let (mut editor, files) = setup();
    let (picture, sound) = place(&mut editor, files.take_one);
    let sound = sound.unwrap();

    assert!(matches!(
        editor.replace_media(picture, files.music),
        Err(EditorError::ReplacementLacks("picture"))
    ));
    assert!(matches!(
        editor.replace_media(sound, files.photo),
        Err(EditorError::ReplacementLacks("sound"))
    ));

    let offered = |editor: &Editor, clip| {
        let mut ids: Vec<MediaId> = editor
            .replacement_candidates(clip)
            .iter()
            .map(|asset| asset.id)
            .collect();
        ids.sort();
        ids
    };
    let mut pictures = vec![files.take_two, files.silent, files.photo];
    pictures.sort();
    let mut sounds = vec![files.take_two, files.music];
    sounds.sort();
    assert_eq!(offered(&editor, picture), pictures);
    assert_eq!(offered(&editor, sound), sounds);
}

/// A replacement is journalled like any edit, so it survives a crash.
#[test]
fn a_replacement_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replace.vproj");
    let (picture, take_two) = {
        let (mut editor, files) = setup();
        let (picture, _) = place(&mut editor, files.take_one);
        editor.save_as(&path).unwrap();
        editor.replace_media(picture, files.take_two).unwrap();
        let result = (picture, files.take_two);
        std::mem::forget(editor);
        result
    };
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    let sequence = session.project.active().unwrap();
    let video = sequence.video_tracks[0].get(picture).unwrap();
    assert_eq!(video.media_id, take_two);
    assert!(
        sequence.audio_tracks[0]
            .clips()
            .iter()
            .all(|clip| clip.media_id == take_two)
    );
}

/// A photo clip dragged out longer than it was placed needs footage for its
/// whole length, whatever its source range says.
#[test]
fn footage_in_place_of_a_stretched_photo_covers_its_whole_length() {
    let (mut editor, files) = setup();
    let (picture, _) = place(&mut editor, files.photo);
    trim(&mut editor, picture, TrimEdge::End, 9);
    let span = editor.video_clip(picture).unwrap().timeline;
    assert_eq!(span.duration(), seconds(9));

    editor.replace_media(picture, files.take_one).unwrap();
    let video = editor.video_clip(picture).unwrap();
    assert_eq!(video.timeline, span);
    assert_eq!(
        (video.source.start, video.source.end),
        (MediaTime::ZERO, media_seconds(9))
    );
}
