//! The sequence tabs above the timeline (`bettercut_ui::panels::sequence_tabs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_ui::UiState;
use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};

/// One frame with `events`; returns each drawn word and where it is.
fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut UiState,
    events: Vec<Event>,
) -> Vec<(String, Rect)> {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            bettercut_ui::panels::sequence_tabs(ui, editor, state);
        });
    });
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            _ => None,
        })
        .collect()
}

fn click(at: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        },
    ]
}

/// Every sequence has a tab, and clicking one shows it.
#[test]
fn a_tab_per_sequence_switches_on_click() {
    let (mut editor, _events) = Editor::new_project("Tabs");
    let first = editor.active_sequence().unwrap().clone();
    let second = editor.add_sequence().unwrap();
    editor.rename_sequence(second, "Vertical").unwrap();
    assert_eq!(editor.active_sequence().unwrap().id, second);

    let (ctx, mut state) = (egui::Context::default(), UiState::default());
    let words = frame(&ctx, &mut editor, &mut state, Vec::new());
    let names: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
    assert!(
        names.contains(&first.name.as_str()),
        "no tab for {:?}: {names:?}",
        first.name
    );
    assert!(
        names.contains(&"Vertical"),
        "no tab for the second: {names:?}"
    );
    assert!(names.contains(&"+"), "no way to add a sequence: {names:?}");

    let tab = words
        .iter()
        .find(|(w, _)| *w == first.name)
        .unwrap()
        .1
        .center();
    let _ = frame(&ctx, &mut editor, &mut state, click(tab));
    let _ = frame(&ctx, &mut editor, &mut state, Vec::new());
    assert_eq!(
        editor.active_sequence().unwrap().id,
        first.id,
        "the click did not switch"
    );
    assert_eq!(editor.sequence_list().len(), 2);
}

/// The "+" tab adds a sequence and shows it.
#[test]
fn the_plus_tab_adds_a_sequence() {
    let (mut editor, _events) = Editor::new_project("Tabs");
    let (ctx, mut state) = (egui::Context::default(), UiState::default());
    let words = frame(&ctx, &mut editor, &mut state, Vec::new());
    let plus = words.iter().find(|(w, _)| w == "+").unwrap().1.center();
    let _ = frame(&ctx, &mut editor, &mut state, click(plus));
    assert_eq!(editor.sequence_list().len(), 2);
    let words = frame(&ctx, &mut editor, &mut state, Vec::new());
    let shown = editor.active_sequence().unwrap().name.clone();
    assert!(
        words.iter().any(|(w, _)| *w == shown),
        "the new tab is not drawn"
    );
}
