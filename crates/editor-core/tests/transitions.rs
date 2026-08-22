//! Transitions through the editor (§25).
//!
//! The geometry is tested in the timeline crate and the resolution in the
//! playback crate. What matters here is everything around them: that the length
//! stored is one the media can actually supply, that undo puts back exactly what
//! was there, and that a transition survives the §38.2 journal.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{
    Clip, DEFAULT_TRANSITION, MIN_TRANSITION, SourceRange, Transition, TransitionKind, VideoClip,
};
use bettercut_editor_core::{ClipPayload, Editor};

/// Two four-second clips cut together at 4 s, trimmed so the handles — not the
/// clip lengths — are what bounds a crossfade.
///
/// A reads 55–59 s of a 60-second file, leaving 1 s spare past its out-point.
/// B reads from 0.5 s in, leaving half a second before its in-point. The
/// smaller of those is what a centred crossfade has to fit in each half, so the
/// ceiling is 1 s, well under the 4-second shots.
fn editor_with_a_cut() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Transitions");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    let a = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_seconds(55), MediaTime::from_seconds(59)).unwrap(),
    )
    .unwrap();
    let b = VideoClip::new(
        media,
        TimelineTime::from_seconds(4),
        SourceRange::new(MediaTime::from_millis(500), MediaTime::from_millis(4500)).unwrap(),
    )
    .unwrap();
    let (a_id, b_id) = (a.id, b.id);

    editor
        .add_clip(track, ClipPayload::Video(Box::new(a)))
        .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(b)))
        .unwrap();
    (editor, a_id, b_id)
}

fn transition_of(editor: &Editor, clip: ClipId) -> Option<Transition> {
    editor.video_clip(clip).unwrap().transition_out
}

#[test]
fn a_transition_is_added_at_the_default_length() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();

    let t = transition_of(&editor, a).expect("no transition");
    assert_eq!(t.kind, TransitionKind::Crossfade);
    assert_eq!(t.duration, DEFAULT_TRANSITION);
}

#[test]
fn adding_and_removing_are_both_undoable() {
    let (mut editor, a, _) = editor_with_a_cut();

    editor.set_transition(a, TransitionKind::Crossfade).unwrap();
    editor.undo().unwrap();
    assert_eq!(transition_of(&editor, a), None, "undo left it behind");

    editor.redo().unwrap();
    assert!(transition_of(&editor, a).is_some(), "redo lost it");

    editor.remove_transition(a).unwrap();
    assert_eq!(transition_of(&editor, a), None);
    editor.undo().unwrap();
    assert_eq!(
        transition_of(&editor, a).map(|t| t.kind),
        Some(TransitionKind::Crossfade),
        "undoing a removal must put the same transition back"
    );
}

/// Changing the kind keeps the length the user chose. Silently resetting a
/// two-second dissolve to the default when they switch it to a fade would undo
/// a decision they had already made.
#[test]
fn changing_the_kind_keeps_the_length() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();
    editor
        .set_transition_duration(a, TimelineTime::from_millis(800))
        .unwrap();

    editor
        .set_transition(a, TransitionKind::FadeThroughBlack)
        .unwrap();

    let t = transition_of(&editor, a).expect("no transition");
    assert_eq!(t.kind, TransitionKind::FadeThroughBlack);
    assert_eq!(t.duration, TimelineTime::from_millis(800));
}

/// A crossfade reads a handle either side, so a length beyond what the file has
/// is clamped rather than stored. Storing it would look right in the inspector
/// and flash black on export.
#[test]
fn a_length_beyond_the_handles_is_clamped() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();

    // Half a second is the smaller handle, and each half of the window eats
    // one, so a second is the ceiling.
    editor
        .set_transition_duration(a, TimelineTime::from_seconds(30))
        .unwrap();

    assert_eq!(
        transition_of(&editor, a).unwrap().duration,
        TimelineTime::from_seconds(1),
        "twice the smaller handle"
    );
}

/// A fade through black never reads outside either clip, so handles do not
/// constrain it — only the clips themselves.
#[test]
fn a_fade_through_black_is_bounded_by_the_clips_not_the_handles() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor
        .set_transition(a, TransitionKind::FadeThroughBlack)
        .unwrap();
    editor
        .set_transition_duration(a, TimelineTime::from_seconds(30))
        .unwrap();

    assert_eq!(
        transition_of(&editor, a).unwrap().duration,
        TimelineTime::from_seconds(4),
        "the length of the shorter shot"
    );
}

/// Below a tenth of a second a dissolve reads as a glitch, so the floor applies
/// as firmly as the ceiling.
#[test]
fn a_length_below_the_floor_is_raised() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();
    editor
        .set_transition_duration(a, TimelineTime::from_millis(5))
        .unwrap();

    assert_eq!(transition_of(&editor, a).unwrap().duration, MIN_TRANSITION);
}

/// A clip trimmed to the very start of its file has nothing to crossfade from.
/// Refusing says so; accepting would store a transition that renders as a black
/// flash.
#[test]
fn a_crossfade_with_no_handle_is_refused() {
    let (mut editor, _rx) = Editor::new_project("No handles");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    let a = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    // Reads from the very start of the file: no material before its in-point.
    let b = VideoClip::new(
        media,
        TimelineTime::from_seconds(4),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    let a_id = a.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(a)))
        .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(b)))
        .unwrap();

    assert!(
        editor
            .set_transition(a_id, TransitionKind::Crossfade)
            .is_err(),
        "a crossfade with no handles was accepted"
    );
    assert_eq!(transition_of(&editor, a_id), None);

    // The same cut takes a fade through black, which reads nothing outside
    // either clip.
    assert!(
        editor
            .set_transition(a_id, TransitionKind::FadeThroughBlack)
            .is_ok(),
        "a fade through black needs no handles"
    );
}

/// A gap is not a cut. There is nothing on the other side to fade to.
#[test]
fn a_transition_needs_a_clip_on_the_other_side() {
    let (mut editor, _rx) = Editor::new_project("Gap");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(5)).unwrap(),
    )
    .unwrap();
    let id = clip.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    assert!(
        editor
            .set_transition(id, TransitionKind::Crossfade)
            .is_err()
    );
    assert_eq!(editor.transition_room(id, TransitionKind::Crossfade), None);
}

/// What the interface asks before offering the control, so it can bound the
/// slider instead of rejecting what the user picks.
#[test]
fn the_room_available_is_reported() {
    let (editor, a, b) = editor_with_a_cut();

    assert_eq!(
        editor.transition_room(a, TransitionKind::Crossfade),
        Some(TimelineTime::from_seconds(1)),
        "twice the smaller handle, not the length of the shots"
    );
    assert_eq!(
        editor.transition_room(a, TransitionKind::FadeThroughBlack),
        Some(TimelineTime::from_seconds(4)),
    );
    assert_eq!(
        editor.transition_room(b, TransitionKind::Crossfade),
        None,
        "the last clip has no cut after it"
    );
}

/// §38.2: an edit lost to a crash is worse than one never made.
#[test]
fn a_transition_survives_a_save_and_load() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();
    editor
        .set_transition_duration(a, TimelineTime::from_millis(750))
        .unwrap();

    let json = serde_json::to_string(editor.project()).unwrap();
    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).unwrap();

    let clip = loaded.sequences[0].video_tracks[0]
        .clips()
        .iter()
        .find(|c| c.id == a)
        .unwrap();
    let t = clip.transition_out.expect("transition did not survive");
    assert_eq!(t.kind, TransitionKind::Crossfade);
    assert_eq!(t.duration, TimelineTime::from_millis(750));
    assert_eq!(clip.timeline().end, TimelineTime::from_seconds(4));
}

/// An older project has no transitions in it at all, and must still load.
#[test]
fn an_older_project_loads_without_transitions() {
    let (editor, a, _) = editor_with_a_cut();
    let mut json: serde_json::Value = serde_json::to_value(editor.project()).unwrap();
    // Strip the field the way a file written before §25 would not have it.
    strip(&mut json);

    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json).unwrap();
    let clip = loaded.sequences[0].video_tracks[0]
        .clips()
        .iter()
        .find(|c| c.id == a)
        .unwrap();
    assert_eq!(clip.transition_out, None);
}

fn strip(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("transition_out");
            for (_, v) in map.iter_mut() {
                strip(v);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
        _ => {}
    }
}

/// A transition sits on the *end* of a clip, so splitting must leave it on the
/// half that still has that end. Cloning it onto both would turn one dissolve
/// into two, the second landing on a cut the user never asked to soften.
#[test]
fn splitting_leaves_the_transition_on_the_right_half() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();

    editor.set_playhead(TimelineTime::from_seconds(2));
    editor.split_at_playhead(&[a]).unwrap();

    let track = &editor.active_sequence().unwrap().video_tracks[0];
    let halves: Vec<_> = track
        .clips()
        .iter()
        .filter(|c| c.timeline().start < TimelineTime::from_seconds(4))
        .collect();
    assert_eq!(halves.len(), 2, "the split did not produce two halves");
    assert_eq!(halves[0].transition_out, None, "the left half kept it");
    assert!(halves[1].transition_out.is_some(), "the right half lost it");
}

/// A transition describes a cut, not a clip's appearance, so pasting a copy
/// somewhere else must not bring it along — it would lie dormant until the copy
/// happened to abut something and then dissolve into a neighbour the user never
/// paired it with.
#[test]
fn pasting_a_copy_does_not_carry_the_transition() {
    let (mut editor, a, _) = editor_with_a_cut();
    editor.set_transition(a, TransitionKind::Crossfade).unwrap();

    let before: Vec<ClipId> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.id)
        .collect();

    editor.copy_clips(&[a]);
    editor.set_playhead(TimelineTime::from_seconds(20));
    editor.paste_at_playhead().unwrap();

    let pasted = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .find(|c| !before.contains(&c.id))
        .expect("nothing was pasted");
    assert_eq!(
        pasted.transition_out, None,
        "the pasted copy came with a transition"
    );
    assert!(
        transition_of(&editor, a).is_some(),
        "the original lost its transition"
    );
}
