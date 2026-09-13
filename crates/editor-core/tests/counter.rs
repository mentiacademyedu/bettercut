//! Timer titles: countdowns and stopwatches (`bettercut_timeline::counter`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::timeline::{CountDirection, CountFormat, Counter};
use bettercut_editor_core::{Editor, TextProperty};

/// A countdown lands at the playhead as a ten-second title counting from ten,
/// in one undo step, and draws its number.
#[test]
fn a_countdown_is_added_at_the_playhead() {
    let (mut editor, _events) = Editor::new_project("Timer");
    editor.set_playhead(TimelineTime::from_seconds(2));
    let depth = editor.undo_depth();

    let clip = editor.add_counter(CountDirection::Down).unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);
    let title = editor.text_clip(clip).unwrap();
    assert_eq!(title.timeline.start, TimelineTime::from_seconds(2));
    assert_eq!(title.timeline.duration(), TimelineTime::from_seconds(10));
    let counter = title.counter.expect("no counter on the timer");
    assert_eq!(counter.direction, CountDirection::Down);
    assert_eq!(title.shown_text(TimelineTime::ZERO), "10");
    assert_eq!(title.shown_text(TimelineTime::from_seconds(10)), "0");

    editor.undo().unwrap();
    assert!(editor.text_clip(clip).is_none());
}

/// A stopwatch after a title that is in the way goes where it fits whole.
#[test]
fn a_timer_is_not_dropped_onto_another_title() {
    let (mut editor, _events) = Editor::new_project("Timer");
    editor.set_playhead(TimelineTime::from_seconds(5));
    let blocking = editor.add_text("In the way").unwrap();
    editor.set_playhead(TimelineTime::ZERO);

    let clip = editor.add_counter(CountDirection::Up).unwrap();
    let timer = editor.text_clip(clip).unwrap().timeline;
    let other = editor.text_clip(blocking).unwrap().timeline;
    assert!(!timer.overlaps(other), "{timer:?} overlaps {other:?}");
    assert_eq!(
        editor
            .text_clip(clip)
            .unwrap()
            .shown_text(TimelineTime::ZERO),
        "0.0"
    );
}

/// Changing a timer is one undo step, a wild start is held in range, and
/// taking the counter away shows the text again.
#[test]
fn a_timer_is_edited_like_any_title_property() {
    let (mut editor, _events) = Editor::new_project("Timer");
    let clip = editor.add_counter(CountDirection::Down).unwrap();

    let clock = Counter {
        direction: CountDirection::Up,
        from: TimelineTime::from_seconds(-30),
        format: CountFormat::MinutesSeconds,
    };
    editor
        .set_text_property(clip, TextProperty::Counter(Some(clock)), false)
        .unwrap();
    let title = editor.text_clip(clip).unwrap();
    assert_eq!(
        title.counter.unwrap().from,
        TimelineTime::ZERO,
        "a negative start was kept"
    );
    assert_eq!(title.shown_text(TimelineTime::from_seconds(65)), "1:05");

    editor
        .set_text_property(clip, TextProperty::Counter(None), false)
        .unwrap();
    assert_eq!(
        editor
            .text_clip(clip)
            .unwrap()
            .shown_text(TimelineTime::ZERO),
        "Countdown"
    );

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(
        editor.text_clip(clip).unwrap().counter,
        Some(Counter::countdown(TimelineTime::from_seconds(10)))
    );
}

/// A project saved before timers existed loads with ordinary titles.
#[test]
fn an_old_title_loads_without_a_counter() {
    let (mut editor, _events) = Editor::new_project("Timer");
    let clip = editor.add_text("Plain").unwrap();
    let mut json = serde_json::to_value(editor.project()).unwrap();
    let title = &mut json["sequences"][0]["text_tracks"][0]["clips"][0];
    assert!(
        title.get("counter").is_some(),
        "the field is not where this test looks"
    );
    title.as_object_mut().unwrap().remove("counter");
    let project: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json).unwrap();
    let loaded = project.sequences[0].text_clip(clip).unwrap();
    assert_eq!(loaded.counter, None);
}
