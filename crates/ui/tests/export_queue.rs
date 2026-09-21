//! The export queue window (`bettercut_ui::export_queue`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_ui::UiState;
use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};

fn frame(ctx: &egui::Context, state: &mut UiState, events: Vec<Event>) -> Vec<(String, Rect)> {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::export_queue::show(ui.ctx(), state);
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

/// The window lists the running export and the waiting ones; Remove asks for
/// that one, Stop asks to stop the running one.
#[test]
fn the_queue_lists_exports_and_takes_requests() {
    let ctx = egui::Context::default();
    let mut state = UiState::default();
    state.export_queue.open = true;
    state.export_queue.running = Some("Exporting trip.mp4".to_owned());
    state.export_queue.waiting = vec![
        "Exporting trip-Square.mp4".to_owned(),
        "Exporting trip-Vertical.mp4".to_owned(),
    ];

    // Windows settle their size over a couple of frames.
    let _ = frame(&ctx, &mut state, Vec::new());
    let words = frame(&ctx, &mut state, Vec::new());
    let names: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
    assert!(names.contains(&"Exporting trip.mp4"), "{names:?}");
    assert!(names.contains(&"Exporting trip-Vertical.mp4"), "{names:?}");
    assert!(names.contains(&"2 waiting"), "{names:?}");

    // The second Remove button belongs to the second waiting export.
    let removes: Vec<Rect> = words
        .iter()
        .filter(|(w, _)| w == "Remove")
        .map(|(_, r)| *r)
        .collect();
    assert_eq!(removes.len(), 2);
    let _ = frame(&ctx, &mut state, click(removes[1].center()));
    assert_eq!(state.export_queue.remove, Some(1));

    let stop = words.iter().find(|(w, _)| w == "Stop").unwrap().1.center();
    let _ = frame(&ctx, &mut state, click(stop));
    assert!(state.export_stop_requested);
}

#[test]
fn a_closed_queue_draws_nothing() {
    let ctx = egui::Context::default();
    let mut state = UiState::default();
    state.export_queue.waiting = vec!["Exporting a.mp4".to_owned()];
    assert!(frame(&ctx, &mut state, Vec::new()).is_empty());
}
