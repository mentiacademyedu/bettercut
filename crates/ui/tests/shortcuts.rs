//! Keys that move the playhead between cuts, and the Shortcuts window.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::shortcuts::SHORTCUTS;
use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};

fn press(editor: &mut Editor, state: &mut UiState, key: Key) {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::shortcuts::handle(ui.ctx(), editor, state, None);
    });
    output.textures_delta.clear();
}

/// Two clips back to back: cuts at 0, 4 and 10 seconds.
fn two_clips() -> Editor {
    let (mut editor, _events) = Editor::new_project("Keys");
    for (name, seconds) in [("a", 4), ("b", 6)] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(seconds),
        ));
        editor.place_media(media).unwrap();
    }
    editor
}

#[test]
fn down_and_up_jump_between_cuts() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    editor.set_playhead(TimelineTime::from_seconds(1));

    press(&mut editor, &mut state, Key::ArrowDown);
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(4));
    press(&mut editor, &mut state, Key::ArrowDown);
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(10));
    press(&mut editor, &mut state, Key::ArrowDown);
    assert_eq!(
        editor.playhead(),
        TimelineTime::from_seconds(10),
        "nothing past the last cut"
    );

    press(&mut editor, &mut state, Key::ArrowUp);
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(4));
    press(&mut editor, &mut state, Key::ArrowUp);
    assert_eq!(editor.playhead(), TimelineTime::ZERO);
}

#[test]
fn f1_opens_and_closes_the_shortcuts_window() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    press(&mut editor, &mut state, Key::F1);
    assert!(state.shortcuts_open);
    press(&mut editor, &mut state, Key::F1);
    assert!(!state.shortcuts_open);
}

/// Every row of the table reaches the screen.
#[test]
fn the_window_lists_every_shortcut() {
    let mut state = UiState::default();
    state.shortcuts_open = true;
    let ctx = egui::Context::default();
    let mut words = String::new();
    // Twice: a window sizes itself on its first frame.
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::shortcuts::help_window(ui.ctx(), &mut state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push('\n');
            }
        }
    }
    for (keys, action) in SHORTCUTS {
        assert!(words.contains(keys), "{keys} missing:\n{words}");
        assert!(words.contains(action), "{action} missing:\n{words}");
    }
}

#[test]
fn m_marks_the_playhead_and_up_down_stop_at_marks() {
    let mut editor = two_clips(); // cuts at 0, 4 and 10 s
    let mut state = UiState::default();
    editor.set_playhead(TimelineTime::from_seconds(7));
    press(&mut editor, &mut state, Key::M);
    assert_eq!(editor.markers().len(), 1);

    editor.set_playhead(TimelineTime::from_seconds(4));
    press(&mut editor, &mut state, Key::ArrowDown);
    assert_eq!(
        editor.playhead(),
        TimelineTime::from_seconds(7),
        "the marker is a stop between the two cuts"
    );

    press(&mut editor, &mut state, Key::M);
    assert!(editor.markers().is_empty(), "M on a marker removes it");
}

#[test]
fn comma_and_period_nudge_the_selection() {
    let mut editor = two_clips(); // 0–4 s and 4–10 s, butt-joined
    let mut state = UiState::default();
    let second = editor.active_sequence().unwrap().video_tracks[0].clips()[1].id;
    state.selected_clips.insert(second);
    let start = |editor: &Editor| editor.clip_payload(second).unwrap().start();

    press(&mut editor, &mut state, Key::Period);
    assert_eq!(
        start(&editor),
        TimelineTime::from_seconds(4) + TimelineTime::from_ticks(32_000)
    );

    press(&mut editor, &mut state, Key::Comma);
    assert_eq!(start(&editor), TimelineTime::from_seconds(4));

    // Another nudge left would run into the clip in front of it, which the
    // track refuses — and the user is told rather than left wondering.
    press(&mut editor, &mut state, Key::Comma);
    assert_eq!(
        start(&editor),
        TimelineTime::from_seconds(4),
        "it moved anyway"
    );
    assert!(
        state.status.as_ref().is_some_and(|s| s.is_error),
        "a refused nudge said nothing"
    );
}

#[test]
fn brackets_trim_the_selection_to_the_playhead() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    let clip = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    state.selected_clips.insert(clip);
    editor.set_playhead(TimelineTime::from_seconds(1));

    press(&mut editor, &mut state, Key::OpenBracket);

    assert_eq!(
        editor.clip_payload(clip).unwrap().start(),
        TimelineTime::from_seconds(1)
    );
}
