//! A transition between two clips placed whole: neither has footage past the
//! cut, so the clips overlap to make it, as in CapCut.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::TransitionKind;
use bettercut_editor_core::{Editor, EditorError};

/// Three whole shots with sound, 3 s each, back to back.
fn three_whole_shots() -> Editor {
    let (mut editor, _events) = Editor::new_project("Overlap");
    for name in ["one", "two", "three"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(3),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap();
    }
    editor
}

fn ms(n: i64) -> TimelineTime {
    TimelineTime::from_millis(n)
}

#[test]
fn whole_clips_overlap_to_make_room_for_a_crossfade() {
    let mut editor = three_whole_shots();
    let before = editor.active_sequence().unwrap().clone();
    let pictures = before.video_tracks[0].clips().to_vec();
    let length = before.duration();
    let wanted = editor.project().settings.transition_length;
    assert_eq!(
        editor.transition_room(pictures[0].id, TransitionKind::Crossfade),
        Some(TimelineTime::ZERO),
        "placed whole, nothing past the cut"
    );
    // What the menu says before it is chosen: about one transition shorter.
    let cost = editor
        .transition_overlap(pictures[0].id, TransitionKind::Crossfade)
        .expect("overlapping would work");
    assert!(cost >= wanted);
    // A kind that needs no footage either side is offered plainly.
    assert_eq!(
        editor.transition_overlap(pictures[0].id, TransitionKind::Glitch),
        None
    );
    // The last clip has no cut after it.
    assert_eq!(
        editor.transition_overlap(pictures[2].id, TransitionKind::Crossfade),
        None
    );
    let depth = editor.undo_depth();

    editor
        .set_transition(pictures[0].id, TransitionKind::Crossfade)
        .unwrap();
    assert_eq!(
        length.ticks() - editor.active_sequence().unwrap().duration().ticks(),
        cost.ticks(),
        "it cost what it said"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    let after = editor.active_sequence().unwrap().clone();
    let lane = after.video_tracks[0].clips();
    let first = &lane[0];
    let second = &lane[1];
    let transition = first.transition_out.expect("the crossfade is there");
    assert_eq!(transition.kind, TransitionKind::Crossfade);
    assert_eq!(
        transition.duration, wanted,
        "the length asked for, not less"
    );
    // Each gave up half: the edit is one transition shorter, no gap left.
    assert_eq!(first.timeline.end, second.timeline.start);
    // Shorter by the transition — up to a frame more, each half being rounded
    // up to a whole frame so both cuts stay on frames.
    let frame = bettercut_editor_core::foundation::ticks_per_frame(after.frame_rate).unwrap();
    let shortened = length.ticks() - after.duration().ticks();
    assert!(
        (wanted.ticks()..=wanted.ticks() + frame).contains(&shortened),
        "shortened by {shortened}, transition {}",
        wanted.ticks()
    );
    let half = wanted.ticks() / 2;
    assert!(first.timeline.duration().ticks() <= ms(3000).ticks() - half);
    assert!(second.source.start.ticks() >= half);
    // And the room is real now.
    assert!(
        editor
            .transition_room(first.id, TransitionKind::Crossfade)
            .unwrap()
            >= wanted
    );

    // The sound kept with its picture, and the third shot moved up with both.
    for picture in [first, second, &lane[2]] {
        let sound = editor
            .linked_with(picture.id)
            .into_iter()
            .find_map(|c| editor.audio_clip(c).cloned())
            .unwrap();
        assert_eq!(sound.timeline, picture.timeline);
        assert_eq!(sound.source, picture.source);
    }

    editor.undo().unwrap();
    assert_eq!(
        *editor.active_sequence().unwrap(),
        before,
        "one undo, all back"
    );
}

#[test]
fn every_cut_of_whole_clips_gets_its_transition_in_one_step() {
    let mut editor = three_whole_shots();
    let before = editor.active_sequence().unwrap().clone();
    let track = before.video_tracks[0].id;
    let depth = editor.undo_depth();

    let (applied, skipped) = editor
        .transition_every_cut(track, TransitionKind::Crossfade)
        .unwrap();
    assert_eq!((applied, skipped), (2, 0));
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    let after = editor.active_sequence().unwrap().clone();
    let lane = after.video_tracks[0].clips();
    assert!(lane[0].transition_out.is_some() && lane[1].transition_out.is_some());
    for pair in lane.windows(2) {
        assert_eq!(pair[0].timeline.end, pair[1].timeline.start, "no gaps left");
    }
    editor.undo().unwrap();
    assert_eq!(*editor.active_sequence().unwrap(), before);
}

#[test]
fn a_cut_with_footage_to_spare_is_not_trimmed() {
    let (mut editor, _events) = Editor::new_project("Handles");
    let asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/long.mp4",
        MediaTime::from_seconds(20),
    );
    let media = editor.import_media(asset);
    editor
        .place_media_range(
            media,
            Some((MediaTime::from_seconds(2), MediaTime::from_seconds(6))),
        )
        .unwrap();
    editor
        .place_media_range(
            media,
            Some((MediaTime::from_seconds(10), MediaTime::from_seconds(14))),
        )
        .unwrap();
    let before = editor.active_sequence().unwrap().duration();
    let first = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    editor
        .set_transition(first, TransitionKind::Crossfade)
        .unwrap();
    assert_eq!(editor.active_sequence().unwrap().duration(), before);
}

#[test]
fn clips_too_short_to_give_anything_are_refused() {
    let (mut editor, _events) = Editor::new_project("Short");
    for name in ["a", "b"] {
        let asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_millis(300),
        );
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap();
    }
    let before = editor.active_sequence().unwrap().clone();
    let first = before.video_tracks[0].clips()[0].id;
    assert!(matches!(
        editor.set_transition(first, TransitionKind::Crossfade),
        Err(EditorError::NoRoomForTransition)
    ));
    assert_eq!(
        *editor.active_sequence().unwrap(),
        before,
        "nothing changed"
    );
}
