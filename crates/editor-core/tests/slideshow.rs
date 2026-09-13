//! §33's auto slideshow.
//!
//! The slideshow is a generated template, which is most of why it is worth
//! testing at this level rather than at the template engine's: the interesting
//! claims are about what it *generates* — that every picture gets a slot, that
//! the cuts get transitions and the last one does not, and that the whole thing
//! is one undo step — not about placement, which the template tests own.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::slideshow::{MAX_HOLD, MIN_HOLD, Slideshow};

/// An editor with `count` photos imported, in order.
fn with_photos(count: usize) -> (Editor, Vec<MediaId>) {
    let (mut editor, _events) = Editor::new_project("Slideshow");
    let media = (0..count)
        .map(|index| {
            let mut asset = MediaAsset::new(
                MediaKind::Image,
                format!("C:/photos/{index}.jpg"),
                // A still's own duration; `placement_duration` gives it
                // `STILL_DURATION` regardless, and the handles a transition
                // needs come from there.
                MediaTime::ZERO,
            );
            asset.width = 4000;
            asset.height = 3000;
            editor.import_media(asset)
        })
        .collect();
    (editor, media)
}

fn video_clips(editor: &Editor) -> Vec<bettercut_editor_core::timeline::VideoClip> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .to_vec()
}

#[test]
fn every_picture_lands_once_and_in_order() {
    let (mut editor, photos) = with_photos(4);
    let applied = editor
        .build_slideshow(&photos, Slideshow::default(), TimelineTime::ZERO)
        .expect("the slideshow is built");

    assert_eq!(applied.clips.len(), 4, "a picture was dropped");
    assert!(applied.unfilled.is_empty(), "a slot went unfilled");

    let clips = video_clips(&editor);
    assert_eq!(clips.len(), 4);

    // In the order given, and each one a different photo — a slideshow that
    // placed the first picture four times would otherwise pass every count.
    let placed: Vec<_> = clips.iter().map(|clip| clip.media_id).collect();
    assert_eq!(placed, photos, "the pictures are not in the order given");
}

/// Back to back, with no gaps: §25 overlaps a transition rather than inserting
/// it, so the whole runs as long as the holds add up to.
#[test]
fn the_pictures_run_back_to_back() {
    let plan = Slideshow {
        hold: TimelineTime::from_seconds(2),
        ..Slideshow::default()
    };
    let (mut editor, photos) = with_photos(3);
    editor
        .build_slideshow(&photos, plan, TimelineTime::ZERO)
        .expect("the slideshow is built");

    let clips = video_clips(&editor);
    for (index, clip) in clips.iter().enumerate() {
        assert_eq!(
            clip.timeline.start,
            TimelineTime::from_seconds(2 * index as i64),
            "picture {index} does not start where the one before ended"
        );
    }
    assert_eq!(plan.total(3), TimelineTime::from_seconds(6));
}

/// A transition at every cut, and none after the last picture — there is
/// nothing beyond it to cut to, and a transition into nothing is a fade the
/// user did not ask for.
#[test]
fn every_cut_gets_a_transition_except_the_last() {
    let (mut editor, photos) = with_photos(3);
    editor
        .build_slideshow(&photos, Slideshow::default(), TimelineTime::ZERO)
        .expect("the slideshow is built");

    let clips = video_clips(&editor);
    let with_transition = clips
        .iter()
        .filter(|clip| clip.transition_out.is_some())
        .count();
    assert_eq!(
        with_transition, 2,
        "expected a transition at each of the two cuts, and none after the last"
    );
    assert!(
        clips.last().unwrap().transition_out.is_none(),
        "the last picture cuts to nothing"
    );
}

/// Hard cuts when asked for, which is the other half of the control being real.
#[test]
fn no_transition_means_no_transition() {
    let plan = Slideshow {
        transition: None,
        ..Slideshow::default()
    };
    let (mut editor, photos) = with_photos(3);
    editor
        .build_slideshow(&photos, plan, TimelineTime::ZERO)
        .expect("the slideshow is built");

    assert!(
        video_clips(&editor)
            .iter()
            .all(|clip| clip.transition_out.is_none()),
        "a transition appeared on a slideshow that asked for hard cuts"
    );
}

/// §79: one slideshow is one undo step, however many pictures it placed.
/// Undoing it a picture at a time would be unusable.
#[test]
fn a_slideshow_is_one_undo_step() {
    let (mut editor, photos) = with_photos(5);
    let before = editor.undo_depth();

    editor
        .build_slideshow(&photos, Slideshow::default(), TimelineTime::ZERO)
        .expect("the slideshow is built");
    assert_eq!(editor.undo_depth(), before + 1);

    editor.undo().unwrap();
    assert!(
        video_clips(&editor).is_empty(),
        "undo left part of the slideshow behind"
    );
}

/// §24's slow move, from the same place the Movement buttons write it — so a
/// slideshow's zoom and a user's are one thing, not two that drift.
#[test]
fn the_pictures_move_when_asked_to() {
    let (mut editor, photos) = with_photos(2);
    editor
        .build_slideshow(&photos, Slideshow::default(), TimelineTime::ZERO)
        .expect("the slideshow is built");

    for clip in video_clips(&editor) {
        assert!(
            !clip.keyframes.is_empty(),
            "a picture sits dead on screen with no movement"
        );
    }

    // And no movement means no keys, so the control is real in both directions.
    let (mut editor, photos) = with_photos(2);
    editor
        .build_slideshow(
            &photos,
            Slideshow {
                movement: bettercut_editor_core::Movement::None,
                ..Slideshow::default()
            },
            TimelineTime::ZERO,
        )
        .expect("the slideshow is built");
    for clip in video_clips(&editor) {
        assert!(
            clip.keyframes.is_empty(),
            "keyframes appeared on a slideshow that asked for no movement"
        );
    }
}

/// Nothing selected is refused, rather than quietly making an empty slideshow:
/// an undo step that changed nothing is worse than being told.
#[test]
fn a_slideshow_of_nothing_is_refused() {
    let (mut editor, _) = with_photos(0);
    let before = editor.undo_depth();
    assert!(
        editor
            .build_slideshow(&[], Slideshow::default(), TimelineTime::ZERO)
            .is_err()
    );
    assert_eq!(
        editor.undo_depth(),
        before,
        "a refusal left a history entry"
    );
}

/// The hold is brought into range, and the transition with it.
///
/// The second clause is the one worth stating: a transition longer than the
/// picture it joins would reach past the next cut, and §25 measures its handles
/// from a clip that is still there when the next one starts.
#[test]
fn an_impossible_plan_is_brought_into_range() {
    let absurd = Slideshow {
        hold: TimelineTime::from_seconds(0),
        transition_length: TimelineTime::from_seconds(60),
        ..Slideshow::default()
    }
    .clamped();

    assert_eq!(absurd.hold, MIN_HOLD, "a hold of nothing was allowed");
    assert!(
        absurd.transition_length.ticks() * 2 <= absurd.hold.ticks(),
        "the transition is longer than half the picture it joins: {absurd:?}"
    );

    let long = Slideshow {
        hold: TimelineTime::from_seconds(600),
        ..Slideshow::default()
    }
    .clamped();
    assert_eq!(long.hold, MAX_HOLD);
}

/// A slideshow will not write over work that is already there — it goes through
/// the template engine, and that is the engine's rule.
#[test]
fn a_slideshow_refuses_to_land_on_existing_clips() {
    let (mut editor, photos) = with_photos(3);
    editor
        .build_slideshow(&photos, Slideshow::default(), TimelineTime::ZERO)
        .expect("the first slideshow is built");
    let placed = video_clips(&editor).len();

    let err = editor.build_slideshow(&photos, Slideshow::default(), TimelineTime::ZERO);
    assert!(err.is_err(), "the second slideshow wrote over the first");
    assert_eq!(
        video_clips(&editor).len(),
        placed,
        "a refused slideshow still changed the timeline"
    );
}

/// Every transition the options offer has to actually land on the cuts.
///
/// The popover lists them all, and §25 gives some kinds rules about the footage
/// either side of the cut. A choice that quietly fell back to a hard cut would
/// be a control that lies — so every kind is built and checked, rather than the
/// crossfade the default uses.
#[test]
fn every_transition_on_offer_lands_on_the_cuts() {
    use bettercut_editor_core::timeline::TransitionKind;

    let mut refused = Vec::new();
    for kind in TransitionKind::ALL {
        let (mut editor, photos) = with_photos(3);
        let applied = editor
            .build_slideshow(
                &photos,
                Slideshow {
                    transition: Some(kind),
                    ..Slideshow::default()
                },
                TimelineTime::ZERO,
            )
            .expect("the slideshow is built");

        let landed: Vec<_> = video_clips(&editor)
            .iter()
            .filter_map(|clip| clip.transition_out.map(|t| t.kind))
            .collect();
        if landed != [kind, kind] || applied.dropped_transitions != 0 {
            refused.push(format!(
                "{}: landed {landed:?}, dropped {}",
                kind.label(),
                applied.dropped_transitions
            ));
        }
    }

    assert!(
        refused.is_empty(),
        "these transitions did not reach both cuts:\n  {}",
        refused.join("\n  ")
    );
}

/// A long hold is held for its whole length. A photo has no duration of its
/// own, and the template engine must not treat the five seconds it is given on
/// import as a limit — or a ten-second slideshow would leave gaps.
#[test]
fn a_long_hold_is_not_cut_short() {
    let (mut editor, photos) = with_photos(2);
    let applied = editor
        .build_slideshow(
            &photos,
            Slideshow {
                hold: TimelineTime::from_seconds(12),
                ..Slideshow::default()
            },
            TimelineTime::ZERO,
        )
        .expect("the slideshow is built");

    assert!(
        applied.shortened.is_empty(),
        "photos were reported as too short to hold: {:?}",
        applied.shortened
    );
    for clip in video_clips(&editor) {
        assert_eq!(
            clip.timeline.end - clip.timeline.start,
            TimelineTime::from_seconds(12),
            "a photo was not held for the whole twelve seconds"
        );
    }
}
