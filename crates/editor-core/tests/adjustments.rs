//! Adjustment layers, edited through the editor (`bettercut_timeline::adjustment`).
//!
//! An adjustment is a new kind of clip on a new kind of lane, and most of what
//! makes that work is not checked by the compiler. The matches on clip and lane
//! kinds are exhaustive, so a missing arm will not build; but the operations
//! that *find* a clip or a lane do so with chains of lookups, and a lookup that
//! forgets the adjustment lane does not fail to compile — it quietly reports
//! "not found" for a clip that is plainly there, and the edit does nothing.
//!
//! So every ordinary edit is driven here, through the same public calls the
//! interface uses: add, grade, move, trim, split, ripple delete, remove, hide,
//! remove the lane, save and reopen, and recover after a crash.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::{Command, TrackFlag};
use bettercut_editor_core::foundation::{ClipId, TimelineTime, TrackId};
use bettercut_editor_core::timeline::{AdjustmentLook, ColorAdjust};
use bettercut_editor_core::{Editor, TrimEdge};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn warm() -> AdjustmentLook {
    AdjustmentLook {
        color: ColorAdjust {
            temperature: 0.4,
            saturation: 1.2,
            ..ColorAdjust::default()
        },
        ..AdjustmentLook::default()
    }
}

fn lane(editor: &Editor) -> TrackId {
    editor.active_sequence().unwrap().adjustment_tracks[0].id
}

fn span(editor: &Editor, clip: ClipId) -> (TimelineTime, TimelineTime) {
    let clip = editor.adjustment_clip(clip).expect("the adjustment exists");
    (clip.timeline.start, clip.timeline.end)
}

fn adjustment_count(editor: &Editor) -> usize {
    editor
        .active_sequence()
        .unwrap()
        .adjustment_tracks
        .iter()
        .map(|t| t.len())
        .sum()
}

#[test]
fn adding_the_first_adjustment_makes_its_lane_in_the_same_undo_step() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    assert!(
        editor
            .active_sequence()
            .unwrap()
            .adjustment_tracks
            .is_empty()
    );
    let before = editor.undo_depth();

    let clip = editor.add_adjustment().unwrap();
    assert_eq!(editor.active_sequence().unwrap().adjustment_tracks.len(), 1);
    assert!(editor.adjustment_clip(clip).unwrap().look.is_identity());
    assert_eq!(
        editor.undo_depth(),
        before + 1,
        "lane and clip were two steps"
    );

    editor.undo().unwrap();
    assert!(
        editor
            .active_sequence()
            .unwrap()
            .adjustment_tracks
            .is_empty(),
        "undo left an empty lane behind"
    );
}

/// A second adjustment goes on the same lane, after the first rather than on
/// top of it.
#[test]
fn a_second_adjustment_shares_the_lane_and_finds_room() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let first = editor.add_adjustment().unwrap();
    let second = editor.add_adjustment().unwrap();

    assert_eq!(editor.active_sequence().unwrap().adjustment_tracks.len(), 1);
    assert_eq!(
        span(&editor, second).0,
        span(&editor, first).1,
        "the second adjustment did not start where the first ended"
    );
}

#[test]
fn a_look_goes_on_and_comes_off() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();

    editor.set_adjustment_look(clip, warm(), false).unwrap();
    assert_eq!(editor.adjustment_clip(clip).unwrap().look, warm());

    editor.undo().unwrap();
    assert!(editor.adjustment_clip(clip).unwrap().look.is_identity());
}

/// §11: a slider drag is one undo step, and undoing it goes back to before the
/// drag began rather than one frame earlier.
#[test]
fn a_look_dragged_is_one_undo_step() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    let before = editor.undo_depth();

    for step in 0..20 {
        let look = AdjustmentLook {
            color: ColorAdjust {
                temperature: step as f32 / 20.0,
                ..ColorAdjust::default()
            },
            ..AdjustmentLook::default()
        };
        editor.set_adjustment_look(clip, look, step > 0).unwrap();
    }
    assert_eq!(
        editor.undo_depth(),
        before + 1,
        "the drag left several steps"
    );

    editor.undo().unwrap();
    assert!(
        editor.adjustment_clip(clip).unwrap().look.is_identity(),
        "undo went back one frame of the drag instead of to before it"
    );
}

/// Moving goes through the operation shared by every lane kind — the one that
/// works out which lane it is looking at without the compiler's help.
#[test]
fn an_adjustment_moves() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    let track = lane(&editor);

    editor.move_clip(track, track, clip, secs(10)).unwrap();
    assert_eq!(span(&editor, clip).0, secs(10));

    editor.undo().unwrap();
    assert_eq!(span(&editor, clip).0, TimelineTime::ZERO);
}

#[test]
fn an_adjustment_trims() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    let track = lane(&editor);

    editor
        .trim_clip(track, clip, TrimEdge::End, secs(1))
        .unwrap();
    assert_eq!(span(&editor, clip).1, secs(1));
}

/// Splitting looks the clip's lane and its ends up by id — three lookups, none
/// of them checked by the compiler.
#[test]
fn an_adjustment_splits_and_both_halves_keep_the_look() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    editor.set_adjustment_look(clip, warm(), false).unwrap();

    let pieces = editor.split_clip_at(clip, &[secs(1)]).unwrap();
    assert_eq!(pieces, 1, "the adjustment was not split");
    assert_eq!(adjustment_count(&editor), 2);
    for half in editor.active_sequence().unwrap().adjustment_tracks[0].clips() {
        assert_eq!(half.look, warm(), "a half lost the grade");
    }
}

#[test]
fn an_adjustment_ripple_deletes_and_comes_back() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    editor.set_adjustment_look(clip, warm(), false).unwrap();
    let track = lane(&editor);

    editor.ripple_delete(track, clip).unwrap();
    assert_eq!(adjustment_count(&editor), 0);

    editor.undo().unwrap();
    assert_eq!(
        editor.adjustment_clip(clip).unwrap().look,
        warm(),
        "undo brought back the span but not the grade"
    );
}

#[test]
fn removing_an_adjustment_is_undone_whole() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    editor.set_adjustment_look(clip, warm(), false).unwrap();

    editor.remove_adjustment(clip).unwrap();
    assert!(editor.adjustment_clip(clip).is_none());

    editor.undo().unwrap();
    assert_eq!(editor.adjustment_clip(clip).unwrap().look, warm());
}

/// Hiding a lane is read back by the lane header, through a lookup of its own.
#[test]
fn an_adjustment_lane_can_be_hidden_and_reads_back_hidden() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    editor.add_adjustment().unwrap();
    let track = lane(&editor);
    assert!(editor.track_flag(track, TrackFlag::Enabled));

    editor
        .set_track_flag(track, TrackFlag::Enabled, false)
        .unwrap();
    assert!(
        !editor.active_sequence().unwrap().adjustment_tracks[0].enabled,
        "the lane was not hidden"
    );
    assert!(
        !editor.track_flag(track, TrackFlag::Enabled),
        "the lane is hidden but reads back as shown"
    );
}

/// Removing the lane finds it with a chain of lookups too, and undo has to put
/// back the lane with its clips on it.
#[test]
fn an_adjustment_lane_can_be_removed_and_restored() {
    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    editor.set_adjustment_look(clip, warm(), false).unwrap();
    let sequence = editor.active_sequence().unwrap().id;
    let track = lane(&editor);

    editor
        .dispatch(Command::RemoveTrack { sequence, track })
        .expect("an adjustment lane can be removed");
    assert!(
        editor
            .active_sequence()
            .unwrap()
            .adjustment_tracks
            .is_empty()
    );

    editor.undo().unwrap();
    assert_eq!(editor.adjustment_clip(clip).unwrap().look, warm());
}

#[test]
fn an_adjustment_survives_a_save_and_an_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adjusted.vproj");

    let (mut editor, _events) = Editor::new_project("Adjust");
    let clip = editor.add_adjustment().unwrap();
    editor.set_adjustment_look(clip, warm(), false).unwrap();
    let before = editor.adjustment_clip(clip).unwrap().clone();
    editor.save_as(&path).unwrap();

    let (reopened, _events) = Editor::open(&path).unwrap();
    assert_eq!(
        reopened.adjustment_clip(clip),
        Some(&before),
        "the adjustment did not come back as it went in"
    );
}

/// §39: an adjustment made after the last save survives a crash — which means
/// every command it took replays from the journal.
#[test]
fn an_adjustment_made_after_the_last_save_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crashy.vproj");
    let (clip, look) = {
        let (mut editor, _events) = Editor::new_project("Adjust");
        editor.save_as(&path).unwrap();
        let clip = editor.add_adjustment().unwrap();
        editor.set_adjustment_look(clip, warm(), false).unwrap();
        let look = editor.adjustment_clip(clip).unwrap().look;
        std::mem::forget(editor);
        (clip, look)
    };

    let session = recover(RecoveryPaths::for_project(Some(&path), "irrelevant"))
        .expect("there should be work to recover");
    assert_eq!(session.failed, 0, "some adjustment edits would not replay");
    let recovered = session
        .project
        .active()
        .unwrap()
        .adjustment_clip(clip)
        .expect("the adjustment was not recovered");
    assert_eq!(recovered.look, look);
}
