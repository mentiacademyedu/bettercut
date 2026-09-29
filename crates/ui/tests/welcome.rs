//! The first-run welcome window.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn words(state: &mut UiState) -> String {
    let (mut editor, _events) = bettercut_editor_core::Editor::new_project("W");
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::welcome::show(ui.ctx(), &mut editor, state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push(' ');
            }
        }
    }
    words
}

#[test]
fn the_welcome_shows_its_steps_only_when_open() {
    let mut state = UiState::default();
    assert!(
        words(&mut state).is_empty(),
        "closed by default in a fresh state"
    );
    state.welcome_open = true;
    let shown = words(&mut state);
    for (title, _) in bettercut_ui::welcome::STEPS {
        assert!(shown.contains(title), "missing {title}: {shown}");
    }
    // Written the way this platform writes it: ⌘K on a Mac.
    let palette_key = bettercut_ui::keys::keys("Ctrl+K");
    assert!(shown.contains(&*palette_key), "no {palette_key} in {shown}");
    assert!(bettercut_ui::palette::run(
        &mut bettercut_editor_core::Editor::new_project("W").0,
        &mut state,
        "Welcome"
    ));
}

#[test]
fn the_sample_is_three_titled_scenes_with_sound_and_a_marker() {
    let (mut editor, _events) = bettercut_editor_core::Editor::new_project("Sample");
    bettercut_ui::sample::build(&mut editor).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let pictures: usize = sequence.video_tracks.iter().map(|t| t.clips().len()).sum();
    let titles: usize = sequence.text_tracks.iter().map(|t| t.clips().len()).sum();
    let sounds: usize = sequence.audio_tracks.iter().map(|t| t.clips().len()).sum();
    assert_eq!(pictures, bettercut_ui::sample::SCENES.len());
    assert_eq!(titles, bettercut_ui::sample::SCENES.len());
    assert_eq!(sounds, 1);
    assert_eq!(editor.markers().len(), 1);
    let transitions = sequence.video_tracks[0]
        .clips()
        .iter()
        .filter(|c| c.transition_out.is_some())
        .count();
    assert_eq!(transitions, 2, "a crossfade at each join");

    // Opened over unsaved work, it asks first and changes nothing yet.
    let mut state = UiState::default();
    bettercut_ui::sample::open(&mut editor, &mut state);
    assert_eq!(
        state.pending_switch,
        Some(bettercut_ui::save_prompt::Switch::Sample)
    );
    assert_eq!(editor.markers().len(), 1, "untouched while asking");

    // "Don't Save": a fresh sample replaces it.
    editor
        .add_markers(&[bettercut_editor_core::foundation::TimelineTime::from_seconds(1)])
        .unwrap();
    assert_eq!(editor.markers().len(), 2);
    bettercut_ui::save_prompt::proceed(
        &mut editor,
        &mut state,
        bettercut_ui::save_prompt::Switch::Sample,
    );
    assert_eq!(
        editor.markers().len(),
        1,
        "the unsaved marker went with the old project"
    );
    assert!(!state.discard_ok, "the pass is one use only");
}

#[test]
fn whats_new_shows_after_an_update_only() {
    use bettercut_ui::whats_new::{VERSION, should_show};
    assert!(should_show("0.0.1-old", true), "an earlier build ran here");
    assert!(!should_show(VERSION, true), "the same build again");
    assert!(!should_show("", true), "no record: not an update");
    assert!(
        !should_show("0.0.1-old", false),
        "the welcome has the floor"
    );
    assert!(!bettercut_ui::whats_new::CHANGES.is_empty());
}

#[test]
fn answering_the_close_question_lets_the_window_close() {
    let (mut editor, _events) = bettercut_editor_core::Editor::new_project("Quit");
    let mut state = UiState::default();
    bettercut_ui::save_prompt::proceed(
        &mut editor,
        &mut state,
        bettercut_ui::save_prompt::Switch::Quit,
    );
    assert!(state.quit_now);
}
