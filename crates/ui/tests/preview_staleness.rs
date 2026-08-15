//! The preview must re-render after an edit that does not move the playhead.
//!
//! The composited picture is cached against the playhead position, which is the
//! right key for scrubbing and the wrong one for everything else: an edit that
//! changes how a clip *looks* while standing still leaves the cache valid and
//! the picture wrong. Dragging the preview handles made it obvious — the
//! picture did not follow the drag — but it was true of every slider, every
//! colour change, every keyframe and every hidden track since the Inspector was
//! built.
//!
//! §56's event stream is how the preview finds out. These tests cover which
//! events mean "the picture is now wrong", because getting that set wrong is
//! silent in both directions: too few and the picture stays stale, too many and
//! the compositor runs on every mouse move for nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaId, SequenceId, TimelineTime};
use bettercut_editor_core::{Editor, Event};
use bettercut_ui::consume_events;
use bettercut_ui::state::UiState;

fn editor() -> Editor {
    let (editor, _rx) = Editor::new_project("Staleness");
    editor
}

/// Feed one event and report whether the preview was marked stale.
fn marks_stale(event: Event) -> bool {
    let editor = editor();
    let mut state = UiState::default();
    assert!(
        !state.preview_is_stale,
        "a fresh state should not start stale"
    );

    consume_events(vec![event], &editor, &mut state);
    state.preview_is_stale
}

/// The bug this exists for. A clip property changing emits `ProjectChanged`
/// and nothing else; if that does not invalidate the picture, dragging the
/// preview handles moves the box and leaves the video behind it.
#[test]
fn a_project_change_makes_the_picture_stale() {
    assert!(marks_stale(Event::ProjectChanged));
}

/// Opening a project and switching sequence both replace what should be on
/// screen entirely.
#[test]
fn loading_and_switching_sequence_make_the_picture_stale() {
    assert!(marks_stale(Event::ProjectLoaded));
    assert!(marks_stale(Event::ActiveSequenceChanged(SequenceId::new())));
}

/// Moving the playhead already re-renders — the picture is cached against
/// exactly that. Marking it stale as well would be harmless but dishonest, and
/// the point of the flag is that it means something specific.
#[test]
fn moving_the_playhead_does_not_need_the_flag() {
    assert!(!marks_stale(Event::PlaybackPositionChanged(
        TimelineTime::from_seconds(2)
    )));
    assert!(!marks_stale(Event::PlaybackStateChanged { playing: true }));
}

/// Events about jobs, saving and media say nothing about the current frame.
/// A proxy finishing does, but it has its own path — `Preview::proxy_ready`
/// reopens the decoder, which invalidating alone would not do.
#[test]
fn unrelated_events_leave_the_picture_alone() {
    assert!(!marks_stale(Event::ProjectSaved));
    assert!(!marks_stale(Event::MediaImported(MediaId::new())));
    assert!(!marks_stale(Event::JobFinished { id: 1 }));
    assert!(!marks_stale(Event::JobProgress {
        id: 1,
        fraction: 0.5
    }));
}

/// The flag survives a batch: §56 drains the whole queue once per frame, so one
/// interesting event among many still has to be noticed.
#[test]
fn one_edit_among_many_events_is_enough() {
    let editor = editor();
    let mut state = UiState::default();

    consume_events(
        vec![
            Event::JobProgress {
                id: 1,
                fraction: 0.1,
            },
            Event::PlaybackPositionChanged(TimelineTime::ZERO),
            Event::ProjectChanged,
            Event::JobProgress {
                id: 1,
                fraction: 0.2,
            },
        ],
        &editor,
        &mut state,
    );

    assert!(state.preview_is_stale);
    assert!(state.needs_repaint, "and the window still has to repaint");
}
