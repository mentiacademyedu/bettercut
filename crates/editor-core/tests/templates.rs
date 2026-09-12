//! Applying a template (§31, §77): one undo step, fills checked, nothing
//! half-done.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::templates::{Template, parse};
use bettercut_editor_core::timeline::TransitionKind;
use bettercut_editor_core::{Editor, EditorError, SlotFill};
use bettercut_foundation::{MediaId, MediaTime, Rational, TimelineTime};

fn secs(s: i64) -> TimelineTime {
    TimelineTime::from_seconds(s)
}

fn video(editor: &mut Editor, name: &str, seconds: i64) -> MediaId {
    editor.import_media(MediaAsset::new(
        MediaKind::Video,
        format!("C:/media/{name}.mp4"),
        MediaTime::from_seconds(seconds),
    ))
}

fn music(editor: &mut Editor, seconds: i64) -> MediaId {
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(seconds),
    );
    asset.audio_codec = Some("mp3".to_owned());
    editor.import_media(asset)
}

/// Two shots cut together with a fade, a logo above them, a headline, a fixed
/// line of text, and music.
fn template() -> Template {
    parse(
        r#"{
        "schema_version": 1,
        "id": "promo",
        "name": "Promo",
        "category": "Social",
        "duration": 6.0,
        "slots": [
            { "id": "shot_a", "type": "video" },
            { "id": "shot_b", "type": "video" },
            { "id": "logo", "type": "logo" },
            { "id": "headline", "type": "text", "default_text": "Hello" },
            { "id": "music", "type": "audio" }
        ],
        "elements": [
            { "type": "clip", "slot": "shot_a", "start": 0, "duration": 3,
              "transition_out": { "kind": "fade_through_black", "duration": 1 } },
            { "type": "clip", "slot": "shot_b", "start": 3, "duration": 3,
              "movement": "zoom_in" },
            { "type": "clip", "slot": "logo", "start": 0, "duration": 6, "track": 1,
              "transform": { "position": [0.4, -0.4], "scale": 0.25 }, "opacity": 0.9 },
            { "type": "text", "slot": "headline", "start": 1, "duration": 2 },
            { "type": "text", "text": "Out now", "start": 4, "duration": 2,
              "animation": { "in": { "kind": "pop", "duration": 0.5 } } },
            { "type": "audio", "slot": "music", "start": 0, "duration": 6, "volume": 0.5,
              "fade_out": 1 }
        ]
    }"#,
    )
    .expect("the fixture template is valid")
}

struct Fixture {
    editor: Editor,
    fills: HashMap<String, SlotFill>,
}

fn fixture() -> Fixture {
    let (mut editor, _events) = Editor::new_project("Templates");
    let a = video(&mut editor, "a", 10);
    let b = video(&mut editor, "b", 10);
    let logo = video(&mut editor, "logo", 10);
    let song = music(&mut editor, 60);
    let fills = HashMap::from([
        ("shot_a".to_owned(), SlotFill::Media(a)),
        ("shot_b".to_owned(), SlotFill::Media(b)),
        ("logo".to_owned(), SlotFill::Media(logo)),
        ("headline".to_owned(), SlotFill::Text("Big sale".to_owned())),
        ("music".to_owned(), SlotFill::Media(song)),
    ]);
    Fixture { editor, fills }
}

#[test]
fn a_template_lands_where_asked_as_one_undo_step() {
    let Fixture { mut editor, fills } = fixture();
    let before_depth = editor.undo_depth();

    let applied = editor
        .apply_template(&template(), &fills, secs(10))
        .expect("applies");

    assert_eq!(applied.clips.len(), 6);
    assert!(applied.unfilled.is_empty());
    assert!(applied.shortened.is_empty());
    assert_eq!(applied.dropped_transitions, 0);
    assert_eq!(
        editor.undo_depth(),
        before_depth + 1,
        "one Ctrl+Z undoes the whole template"
    );

    let sequence = editor.active_sequence().unwrap();
    // The logo asked for track 1, so a second video track was added for it.
    assert_eq!(sequence.video_tracks.len(), 2);
    let v1 = sequence.video_tracks[0].clips();
    assert_eq!(v1.len(), 2);
    assert_eq!(v1[0].timeline.start, secs(10));
    assert_eq!(v1[0].timeline.end, secs(13));
    assert_eq!(v1[1].timeline.start, secs(13));
    let transition = v1[0].transition_out.expect("the fade was applied");
    assert_eq!(transition.kind, TransitionKind::FadeThroughBlack);
    assert_eq!(transition.duration, secs(1));

    let logo = &sequence.video_tracks[1].clips()[0];
    assert_eq!(logo.transform.scale.x, 0.25);
    assert_eq!(logo.transform.position.x, 0.4);
    assert_eq!(logo.opacity, 0.9);

    let words: Vec<&str> = sequence.text_tracks[0]
        .clips()
        .iter()
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(
        words,
        ["Big sale", "Out now"],
        "the fill replaced the default"
    );
    let out_now = &sequence.text_tracks[0].clips()[1];
    assert_eq!(
        out_now.animation.intro.map(|m| m.kind),
        Some(bettercut_editor_core::timeline::MotionKind::Pop),
        "the template's entrance was lost"
    );

    let song = &sequence.audio_tracks[0].clips()[0];
    assert_eq!(song.gain, 0.5);
    assert_eq!(song.fade_out, secs(1), "the template's fade was lost");
    assert_eq!(song.timeline.end, secs(16));

    editor.undo().expect("undo");
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.clip_count(), 0);
    assert_eq!(sequence.video_tracks.len(), 1, "the added track went too");

    editor.redo().expect("redo");
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 6);
}

/// A template's slow zoom arrives as ordinary keyframes, so the clip's own
/// controls can adjust it afterwards (§24).
#[test]
fn a_templates_movement_becomes_keyframes() {
    use bettercut_editor_core::timeline::AnimatedParameter;

    let Fixture { mut editor, fills } = fixture();
    editor
        .apply_template(&template(), &fills, TimelineTime::ZERO)
        .expect("applies");

    let clip = &editor.active_sequence().unwrap().video_tracks[0].clips()[1];
    let keys = clip
        .keyframes
        .track(AnimatedParameter::ScaleX)
        .expect("a zoom")
        .keys();
    assert_eq!(keys.len(), 2);
    assert!(keys[0].value < keys[1].value, "the zoom runs backwards");
    assert_eq!(
        editor.movement_of(clip.id),
        Some(bettercut_editor_core::Movement::ZoomIn)
    );
}

#[test]
fn an_empty_media_slot_skips_its_elements_and_says_so() {
    let Fixture {
        mut editor,
        mut fills,
    } = fixture();
    fills.remove("logo");
    fills.remove("headline");

    let applied = editor
        .apply_template(&template(), &fills, TimelineTime::ZERO)
        .expect("applies");

    assert_eq!(applied.unfilled, ["logo"]);
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks.len(),
        1,
        "no track is added for an element that was skipped"
    );
    // An unfilled text slot is not skipped: it says its default.
    assert_eq!(sequence.text_tracks[0].clips()[0].text, "Hello");
}

#[test]
fn media_shorter_than_the_shot_ends_early_and_says_so() {
    let Fixture {
        mut editor,
        mut fills,
    } = fixture();
    let short = video(&mut editor, "short", 2);
    fills.insert("shot_b".to_owned(), SlotFill::Media(short));

    let applied = editor
        .apply_template(&template(), &fills, TimelineTime::ZERO)
        .expect("applies");

    assert_eq!(applied.shortened, ["shot_b"]);
    let clip = &editor.active_sequence().unwrap().video_tracks[0].clips()[1];
    assert_eq!(clip.timeline.start, secs(3));
    assert_eq!(
        clip.timeline.end,
        secs(5),
        "two seconds of footage, not three"
    );
}

#[test]
fn a_slowed_shot_reads_less_footage() {
    let (mut editor, _events) = Editor::new_project("Speed");
    let media = video(&mut editor, "a", 10);
    let template = parse(
        r#"{ "schema_version": 1, "id": "slow", "name": "Slow", "category": "x",
             "duration": 4,
             "slots": [ { "id": "shot", "type": "video" } ],
             "elements": [ { "type": "clip", "slot": "shot", "start": 0, "duration": 4,
                             "speed": 0.5 } ] }"#,
    )
    .unwrap();
    let fills = HashMap::from([("shot".to_owned(), SlotFill::Media(media))]);

    editor
        .apply_template(&template, &fills, TimelineTime::ZERO)
        .expect("applies");

    let clip = &editor.active_sequence().unwrap().video_tracks[0].clips()[0];
    assert_eq!(clip.speed, Rational::new(1, 2).unwrap());
    assert_eq!(clip.timeline.end, secs(4));
    assert_eq!(clip.source.end, MediaTime::from_seconds(2));
}

/// Nothing changes when a fill is wrong — no half-applied template to undo.
#[test]
fn a_fill_that_does_not_fit_its_slot_changes_nothing() {
    type MakeFill = fn(&mut Editor) -> SlotFill;
    let cases: Vec<(&str, MakeFill, &str)> = vec![
        (
            "shot_a",
            |_| SlotFill::Text("words".to_owned()),
            "takes a file",
        ),
        (
            "headline",
            |e| SlotFill::Media(video(e, "x", 5)),
            "takes words",
        ),
        (
            "music",
            |e| SlotFill::Media(video(e, "silent", 5)),
            "no sound",
        ),
        (
            "nowhere",
            |_| SlotFill::Text("x".to_owned()),
            "no such slot",
        ),
        ("logo", |e| SlotFill::Media(music(e, 30)), "no picture"),
    ];

    for (slot, fill, needle) in cases {
        let Fixture {
            mut editor,
            mut fills,
        } = fixture();
        let fill = fill(&mut editor);
        fills.insert(slot.to_owned(), fill);
        let depth = editor.undo_depth();

        let err = editor
            .apply_template(&template(), &fills, TimelineTime::ZERO)
            .expect_err("the fill should be refused");

        assert!(
            matches!(err, EditorError::TemplateFill { .. }) && err.to_string().contains(needle),
            "{slot}: {err}"
        );
        assert_eq!(editor.active_sequence().unwrap().clip_count(), 0);
        assert_eq!(editor.undo_depth(), depth);
    }
}

/// A photo has no length, so it fills a shot of any length and is never
/// "shorter than its spot".
#[test]
fn a_photo_fills_a_picture_slot_for_the_whole_shot() {
    let Fixture {
        mut editor,
        mut fills,
    } = fixture();
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/logo.png",
        MediaTime::ZERO,
    ));
    fills.insert("logo".to_owned(), SlotFill::Media(photo));

    let applied = editor
        .apply_template(&template(), &fills, TimelineTime::ZERO)
        .expect("a photo is a picture");

    assert!(applied.shortened.is_empty(), "{:?}", applied.shortened);
    let logo = &editor.active_sequence().unwrap().video_tracks[1].clips()[0];
    assert_eq!(logo.media_id, photo);
    assert_eq!(logo.timeline.end, secs(6), "the full six seconds");
}

#[test]
fn an_occupied_track_refuses_the_whole_template() {
    let Fixture { mut editor, fills } = fixture();
    // Music on A1 from 0 s. The template's audio is its last element, so the
    // shots, the added V2 and the titles are all staged before the collision.
    let existing = music(&mut editor, 60);
    editor.place_media(existing).expect("place");

    let tracks_before = editor.active_sequence().unwrap().track_count();
    let depth = editor.undo_depth();

    let err = editor
        .apply_template(&template(), &fills, TimelineTime::ZERO)
        .expect_err("A1 is busy");
    assert!(matches!(err, EditorError::NoRoomForTemplate), "{err}");

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.clip_count(),
        1,
        "nothing from the template survived"
    );
    assert_eq!(
        sequence.track_count(),
        tracks_before,
        "a track staged before the failure was rolled back"
    );
    assert_eq!(editor.undo_depth(), depth);
}

#[test]
fn titles_find_a_free_lane() {
    let Fixture { mut editor, fills } = fixture();
    // A title of the user's own, where the headline wants to go.
    editor.set_playhead(TimelineTime::ZERO);
    editor.add_text("Mine").expect("title"); // T1, 0–3 s

    editor
        .apply_template(&template(), &fills, TimelineTime::ZERO)
        .expect("applies");

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.text_tracks.len(),
        2,
        "a lane was added for the headline"
    );
    assert_eq!(sequence.text_tracks[1].clips()[0].text, "Big sale");
    // "Out now" at 4 s fits beside the user's title and goes there.
    assert!(
        sequence.text_tracks[0]
            .clips()
            .iter()
            .any(|c| c.text == "Out now")
    );
}

#[test]
fn a_crossfade_without_footage_to_spare_is_dropped_not_fatal() {
    let Fixture {
        mut editor,
        mut fills,
    } = fixture();
    fills.retain(|slot, _| slot.starts_with("shot"));
    let crossfade = parse(
        r#"{ "schema_version": 1, "id": "xf", "name": "Crossfade", "category": "x",
             "duration": 6,
             "slots": [ { "id": "shot_a", "type": "video" }, { "id": "shot_b", "type": "video" } ],
             "elements": [
               { "type": "clip", "slot": "shot_a", "start": 0, "duration": 3,
                 "transition_out": { "kind": "crossfade", "duration": 1 } },
               { "type": "clip", "slot": "shot_b", "start": 3, "duration": 3 } ] }"#,
    )
    .unwrap();

    // Both shots start at the top of their files, so the incoming one has
    // nothing before its in-point for the first half of a crossfade.
    let applied = editor
        .apply_template(&crossfade, &fills, TimelineTime::ZERO)
        .expect("the template still applies");

    assert_eq!(applied.dropped_transitions, 1);
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 2);
}

/// §38.2 through the real editor: a template applied and then lost to a crash
/// comes back identical.
#[test]
fn an_applied_template_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("template.vproj");

    let expected;
    {
        let Fixture { mut editor, fills } = fixture();
        editor.save_as(&path).expect("save");
        editor
            .apply_template(&template(), &fills, TimelineTime::ZERO)
            .expect("applies");
        expected = editor.project().active().unwrap().clone();
        std::mem::forget(editor);
    }

    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).expect("recoverable");
    assert_eq!(session.failed, 0);
    assert_eq!(session.project.active().unwrap(), &expected);
}
