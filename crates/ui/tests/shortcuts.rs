//! Keys that move the playhead between cuts, and the Shortcuts window.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::shortcuts::{HANDLED_KEYS, SECTIONS, matching};
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

/// Everything the Shortcuts window draws, as one string.
fn window_text(state: &mut UiState) -> String {
    state.shortcuts_open = true;
    let ctx = egui::Context::default();
    let mut words = String::new();
    // Three times: a window sizes itself on its first frame, and wrapped rows
    // settle their height on the next.
    for _ in 0..3 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::shortcuts::help_window(ui.ctx(), state);
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
    words
}

/// Every row of the table reaches the screen, under its section's heading.
#[test]
fn the_window_lists_every_shortcut() {
    let words = window_text(&mut UiState::default());
    for section in SECTIONS {
        assert!(
            words.contains(section.title),
            "{} heading missing: {words}",
            section.title
        );
        for row in section.shortcuts {
            assert!(words.contains(row.keys), "{} missing:\n{words}", row.keys);
            assert!(
                words.contains(row.action),
                "{} missing:\n{words}",
                row.action
            );
        }
    }
}

/// **The sheet and the keys cannot drift apart.** Every key the handler
/// listens for has a row that explains it, every key a row claims is one the
/// handler listens for, and no key is claimed by two rows.
#[test]
fn every_handled_key_has_exactly_one_row() {
    let bound: Vec<Key> = SECTIONS
        .iter()
        .flat_map(|section| section.shortcuts)
        .flat_map(|row| row.bound.iter().copied())
        .collect();
    for key in HANDLED_KEYS {
        let rows = bound.iter().filter(|b| *b == key).count();
        assert_eq!(rows, 1, "{key:?} is explained by {rows} rows of the sheet");
    }
    for key in &bound {
        assert!(
            HANDLED_KEYS.contains(key),
            "the sheet lists {key:?}, but nothing handles it"
        );
    }
}

/// A search keeps the rows whose keys or words match, in any case, and drops
/// the sections it empties.
#[test]
fn searching_finds_rows_by_key_or_by_words() {
    let zoom = matching("ZOOM");
    assert_eq!(zoom.len(), 1, "{zoom:?}");
    assert_eq!(zoom[0].0, "Mouse");
    assert_eq!(zoom[0].1.len(), 1);
    assert_eq!(zoom[0].1[0].keys, "Ctrl + wheel");

    let by_key = matching("ctrl + d");
    assert!(
        by_key
            .iter()
            .flat_map(|(_, rows)| rows)
            .any(|row| row.action == "Duplicate")
    );

    let everything: usize = SECTIONS.iter().map(|s| s.shortcuts.len()).sum();
    assert_eq!(
        matching("  ")
            .iter()
            .map(|(_, rows)| rows.len())
            .sum::<usize>(),
        everything,
        "a blank search should show every row"
    );
    assert!(matching("teleport").is_empty());
}

/// The window shows only what the search matched, and says so when that is
/// nothing rather than showing an empty box.
#[test]
fn the_window_follows_the_search() {
    let mut state = UiState::default();
    state.shortcut_search = "marker".to_owned();
    let words = window_text(&mut state);
    assert!(words.contains("Add / remove a marker (Shift: on the clip)"));
    assert!(
        !words.contains("Duplicate"),
        "an unmatched row is still shown"
    );
    assert!(
        !words.contains("Mouse"),
        "an emptied section kept its heading"
    );

    state.shortcut_search = "teleport".to_owned();
    let words = window_text(&mut state);
    assert!(words.contains("No shortcut matches that"));
}

/// Ctrl+H is on the sheet and opens the History window.
#[test]
fn ctrl_h_opens_the_history_window() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        // egui takes held modifiers from a `ModifiersChanged` event, not from
        // the key event's own field (see `delete_keys.rs`).
        events: vec![
            Event::ModifiersChanged(Modifiers::COMMAND),
            Event::Key {
                key: Key::H,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            },
        ],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::shortcuts::handle(ui.ctx(), &mut editor, &mut state, None);
    });
    output.textures_delta.clear();
    assert!(state.history_open);
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

/// Alt with , or . moves the selected sound a millisecond — and only the
/// sound: the picture it is linked to stays on its frame.
#[test]
fn alt_comma_and_period_nudge_sound_a_millisecond() {
    let (mut editor, _events) = Editor::new_project("Sync");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    asset.audio_sample_rate = Some(48_000);
    asset.audio_channels = Some(2);
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    let (picture, sound) = {
        let sequence = editor.active_sequence().unwrap();
        (
            sequence.video_tracks[0].clips()[0].id,
            sequence.audio_tracks[0].clips()[0].id,
        )
    };
    let mut state = UiState::default();
    state.selected_clips.insert(sound);
    let start = |editor: &Editor, clip| editor.clip_payload(clip).unwrap().start();

    press_with(&mut editor, &mut state, Key::Period, Modifiers::ALT);
    assert_eq!(start(&editor, sound), TimelineTime::from_millis(1));
    assert_eq!(
        start(&editor, picture),
        TimelineTime::ZERO,
        "the picture moved"
    );

    press_with(&mut editor, &mut state, Key::Comma, Modifiers::ALT);
    assert_eq!(start(&editor, sound), TimelineTime::ZERO);
    // And not before the start.
    press_with(&mut editor, &mut state, Key::Comma, Modifiers::ALT);
    assert_eq!(start(&editor, sound), TimelineTime::ZERO);

    // A picture clip alone: nothing moves, and it says why.
    state.clear_selection();
    state.selected_clips.insert(picture);
    press_with(&mut editor, &mut state, Key::Period, Modifiers::ALT);
    assert_eq!(start(&editor, picture), TimelineTime::ZERO);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("sound"))
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

/// I and O mark in and out at the playhead, and Alt with either clears them.
#[test]
fn i_and_o_mark_and_alt_clears() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    editor.set_playhead(TimelineTime::from_seconds(2));
    press(&mut editor, &mut state, Key::I);
    editor.set_playhead(TimelineTime::from_seconds(6));
    press(&mut editor, &mut state, Key::O);

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.mark_in, Some(TimelineTime::from_seconds(2)));
    assert_eq!(sequence.mark_out, Some(TimelineTime::from_seconds(6)));

    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![
            Event::ModifiersChanged(Modifiers::ALT),
            Event::Key {
                key: Key::I,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::ALT,
            },
        ],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::shortcuts::handle(ui.ctx(), &mut editor, &mut state, None);
    });
    output.textures_delta.clear();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!((sequence.mark_in, sequence.mark_out), (None, None));
}

fn press_with(editor: &mut Editor, state: &mut UiState, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![
            Event::ModifiersChanged(modifiers),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::shortcuts::handle(ui.ctx(), editor, state, None);
    });
    output.textures_delta.clear();
}

/// Shift+Z zooms in on the selection — the view scrolls to it and tightens —
/// and with nothing selected zooms out to the whole edit from the start.
#[test]
fn shift_z_zooms_to_the_selection_or_the_whole_edit() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    let second = editor
        .active_sequence()
        .map(|s| s.video_tracks[0].clips()[1].id)
        .unwrap();
    state.selected_clips.insert(second);
    let before = state.ticks_per_pixel();

    press_with(&mut editor, &mut state, Key::Z, Modifiers::SHIFT);
    assert!(
        state.scroll_ticks > 0,
        "the view did not scroll to the clip"
    );
    assert!(
        state.ticks_per_pixel() < before,
        "a six-second clip in an 800 px window should zoom in from {before} ticks/px, got {}",
        state.ticks_per_pixel()
    );

    state.clear_selection();
    press_with(&mut editor, &mut state, Key::Z, Modifiers::SHIFT);
    assert_eq!(state.scroll_ticks, 0, "the whole edit starts at the start");
}

/// Ctrl+A selects every clip, titles included; with Shift, only the clips that
/// start at or after the playhead.
#[test]
fn ctrl_a_selects_everything_and_shift_everything_after_the_playhead() {
    let mut editor = two_clips(); // 0–4 s and 4–10 s
    editor.set_playhead(TimelineTime::from_seconds(5));
    let title = editor.add_text("Later").unwrap();
    let clips: Vec<_> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.id)
        .collect();
    let mut state = UiState::default();

    press_with(&mut editor, &mut state, Key::A, Modifiers::COMMAND);
    assert_eq!(state.selected_clips.len(), 3, "{:?}", state.selected_clips);
    assert!(
        state.selected_clips.contains(&title),
        "titles were left out"
    );

    editor.set_playhead(TimelineTime::from_seconds(4));
    press_with(
        &mut editor,
        &mut state,
        Key::A,
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    let expected: std::collections::HashSet<_> = [clips[1], title].into_iter().collect();
    assert_eq!(state.selected_clips, expected);

    // A plain A is not a shortcut, and must not select anything.
    state.clear_selection();
    press(&mut editor, &mut state, Key::A);
    assert!(state.selected_clips.is_empty());
}

/// Columns keep the sections in order, never split one, and share the rows out.
#[test]
fn the_sheet_splits_into_balanced_columns() {
    use bettercut_ui::shortcuts::split_columns;

    let sections: Vec<(&'static str, Vec<u8>)> = [4, 6, 5, 6, 7]
        .into_iter()
        .zip(["a", "b", "c", "d", "e"])
        .map(|(rows, title)| (title, vec![0; rows]))
        .collect();
    for columns in 1..=5 {
        let split = split_columns(&sections, columns);
        assert_eq!(split.len(), columns);
        let rejoined: Vec<&str> = split.iter().flat_map(|c| c.iter().map(|s| s.0)).collect();
        assert_eq!(rejoined, ["a", "b", "c", "d", "e"], "{columns} columns");
        assert!(
            split.iter().all(|c| !c.is_empty()),
            "{columns}: an empty column"
        );
    }
    let rows = |column: &[(&str, Vec<u8>)]| column.iter().map(|s| s.1.len()).sum::<usize>();
    let three = split_columns(&sections, 3);
    let tallest = three.iter().map(|c| rows(c)).max().unwrap();
    assert!(
        tallest <= 13,
        "an unbalanced split: {:?}",
        three.iter().map(|c| rows(c)).collect::<Vec<_>>()
    );
}

/// Holding K and tapping L or J steps one frame either way — which needs no
/// renderer, so it works where the shuttle itself cannot.
#[test]
fn holding_k_steps_a_frame_with_j_and_l() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    editor.set_playhead(TimelineTime::from_seconds(2));
    let start = editor.playhead();
    let frame = editor.active_sequence().unwrap().ticks_per_frame();

    let ctx = egui::Context::default();
    let key = |key, pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    let frame_with = |editor: &mut Editor, state: &mut UiState, events: Vec<Event>| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::shortcuts::handle(ui.ctx(), editor, state, None);
        });
        output.textures_delta.clear();
    };

    frame_with(&mut editor, &mut state, vec![key(Key::K, true)]);
    frame_with(
        &mut editor,
        &mut state,
        vec![key(Key::L, true), key(Key::L, false)],
    );
    frame_with(
        &mut editor,
        &mut state,
        vec![key(Key::L, true), key(Key::L, false)],
    );
    assert_eq!(editor.playhead().ticks(), start.ticks() + 2 * frame);
    frame_with(
        &mut editor,
        &mut state,
        vec![key(Key::J, true), key(Key::J, false)],
    );
    assert_eq!(editor.playhead().ticks(), start.ticks() + frame);

    // K let go: L is the shuttle again, which needs a renderer — the playhead
    // does not step, and the status line says why.
    frame_with(&mut editor, &mut state, vec![key(Key::K, false)]);
    frame_with(
        &mut editor,
        &mut state,
        vec![key(Key::L, true), key(Key::L, false)],
    );
    assert_eq!(editor.playhead().ticks(), start.ticks() + frame);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("No preview")),
        "{:?}",
        state.status
    );
}

/// Q and W ripple trim the clip under the playhead when nothing is selected.
#[test]
fn q_and_w_ripple_trim_under_the_playhead() {
    let mut editor = two_clips(); // 0–4 s and 4–10 s
    let mut state = UiState::default();
    let spans = |editor: &Editor| -> Vec<(i64, i64)> {
        editor.active_sequence().unwrap().video_tracks[0]
            .clips()
            .iter()
            .map(|c| {
                (
                    c.timeline.start.ticks() / 960_000,
                    c.timeline.end.ticks() / 960_000,
                )
            })
            .collect()
    };

    editor.set_playhead(TimelineTime::from_seconds(6));
    press(&mut editor, &mut state, Key::Q);
    assert_eq!(spans(&editor), vec![(0, 4), (4, 8)]);
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(4));

    editor.set_playhead(TimelineTime::from_seconds(2));
    press(&mut editor, &mut state, Key::W);
    assert_eq!(spans(&editor), vec![(0, 2), (2, 6)]);
}

/// Alt+Right swaps the one selected clip with the next.
#[test]
fn alt_right_swaps_the_selected_clip_with_the_next() {
    let mut editor = two_clips(); // 0–4 s and 4–10 s
    let mut state = UiState::default();
    let first = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    state.selected_clips.insert(first);
    let playhead = editor.playhead();

    press_with(&mut editor, &mut state, Key::ArrowRight, Modifiers::ALT);
    let clips = editor.active_sequence().unwrap().video_tracks[0].clips();
    assert_eq!(
        clips[1].id, first,
        "the selected clip should now come second"
    );
    assert_eq!(clips[1].timeline.start, TimelineTime::from_seconds(6));
    assert_eq!(
        editor.playhead(),
        playhead,
        "Alt+Right also stepped a frame"
    );
}

/// Ctrl+G groups the selection; Ctrl+Shift+G breaks it up again.
#[test]
fn ctrl_g_groups_and_ctrl_shift_g_ungroups() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    let clips: Vec<_> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.id)
        .collect();
    state.selected_clips.extend(clips.iter().copied());

    press_with(&mut editor, &mut state, Key::G, Modifiers::COMMAND);
    assert!(editor.group_of(clips[0]).is_some(), "Ctrl+G did not group");
    press_with(
        &mut editor,
        &mut state,
        Key::G,
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert!(
        editor.group_of(clips[0]).is_none(),
        "Ctrl+Shift+G did not ungroup"
    );
}

/// Ctrl+Shift+V pastes and pushes what follows along; Ctrl+V would have
/// refused, the space being taken.
#[test]
fn ctrl_shift_v_pastes_and_makes_room() {
    let mut editor = two_clips(); // 0–4 s and 4–10 s
    let mut state = UiState::default();
    let first = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    editor.copy_clips(&[first]);
    editor.set_playhead(TimelineTime::from_seconds(4));

    press_with(
        &mut editor,
        &mut state,
        Key::V,
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    let starts: Vec<i64> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.timeline.start.ticks() / 960_000)
        .collect();
    assert_eq!(starts, vec![0, 4, 8]);
}

/// A shortcut moved to another key: the new key does its job, the old key does
/// nothing, and the next key pressed after asking is the one taken.
#[test]
fn a_moved_shortcut_answers_to_its_new_key() {
    let mut editor = two_clips();
    let mut state = UiState::default();
    editor.set_playhead(TimelineTime::from_seconds(1));

    // Ask to move "M" (add a marker), then press Y.
    state.rebinding = Some(Key::M);
    press(&mut editor, &mut state, Key::Y);
    assert_eq!(state.rebinding, None);
    assert_eq!(state.keymap.key_for(Key::M), Key::Y);
    assert!(
        editor.markers().is_empty(),
        "the key pressed to choose did something"
    );

    press(&mut editor, &mut state, Key::M);
    assert!(editor.markers().is_empty(), "the old key still marks");
    press(&mut editor, &mut state, Key::Y);
    assert_eq!(editor.markers().len(), 1, "the new key does not mark");

    // A key another shortcut uses is refused, and Escape cancels.
    state.rebinding = Some(Key::M);
    press(&mut editor, &mut state, Key::S);
    assert_eq!(state.keymap.key_for(Key::M), Key::Y);
    state.rebinding = Some(Key::M);
    press(&mut editor, &mut state, Key::Escape);
    assert_eq!(state.rebinding, None);
    assert_eq!(state.keymap.key_for(Key::M), Key::Y);
}

/// Inverting picks everything that was not picked, and nothing that was.
#[test]
fn invert_selection_swaps_what_is_picked() {
    use bettercut_editor_core::foundation::MediaTime;
    use bettercut_editor_core::media::{MediaAsset, MediaKind};
    let (mut editor, _events) = bettercut_editor_core::Editor::new_project("Invert");
    let mut placed = Vec::new();
    for name in ["a", "b"] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        ));
        placed.extend(editor.place_media(media).unwrap());
    }
    let mut state = bettercut_ui::UiState::default();
    state.selected_clips.insert(placed[0]);
    bettercut_ui::shortcuts::invert_selection(&editor, &mut state);
    assert!(!state.selected_clips.contains(&placed[0]));
    assert_eq!(state.selected_clips.len(), placed.len() - 1);
}
