//! A new or opened project keeps what belongs to the person.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_ui::UiState;

#[test]
fn a_fresh_interface_keeps_the_person_and_drops_the_project() {
    let mut state = UiState::default();
    state.prefs.light_theme = true;
    state.prefs.palette_recent = vec!["Undo".to_owned()];
    state.font_families = vec!["Inter".to_owned()];
    state.palette_recent = vec!["Undo"];
    state.captions_open = true;
    state.snapping = !state.snapping;
    let snapping_default = !state.snapping;

    bettercut_ui::panels::reset_state(&mut state);

    assert!(state.prefs.light_theme, "the prefs were lost");
    assert_eq!(state.prefs.palette_recent, ["Undo"]);
    assert_eq!(state.font_families, ["Inter"], "the font list was lost");
    assert_eq!(state.palette_recent, ["Undo"]);
    assert!(!state.captions_open, "project windows should close");
    assert_eq!(
        state.snapping, snapping_default,
        "project view state should reset"
    );
}
