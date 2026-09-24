//! The command palette: finding actions by typing, and running them by name.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::palette;

#[test]
fn typing_narrows_the_actions() {
    let names: Vec<&str> = palette::matching("gaps").iter().map(|a| a.name).collect();
    assert!(names.contains(&"Close Gaps on Every Lane"), "{names:?}");
    assert!(!names.contains(&"Undo"));
    assert_eq!(
        palette::matching("").len(),
        palette::ACTIONS.len(),
        "nothing typed shows all"
    );
    assert!(palette::matching("zzzz nothing").is_empty());
}

#[test]
fn every_action_has_a_unique_name() {
    let mut names: Vec<&str> = palette::ACTIONS.iter().map(|a| a.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), palette::ACTIONS.len());
}

#[test]
fn an_action_runs_by_name() {
    let (mut editor, _events) = Editor::new_project("Palette");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(2),
    ));
    let placed = editor.place_media(media).unwrap();
    let mut state = UiState::default();
    state.selected_clips.insert(placed[0]);
    assert!(palette::run(&mut editor, &mut state, "Invert Selection"));
    assert!(!state.selected_clips.contains(&placed[0]));
    assert!(!palette::run(&mut editor, &mut state, "No Such Thing"));
}

#[test]
fn recent_actions_come_first_and_the_list_stays_short() {
    let mut recent = Vec::new();
    for action in palette::ACTIONS.iter().take(palette::RECENT + 2) {
        palette::remember(&mut recent, action.name);
    }
    assert_eq!(recent.len(), palette::RECENT);
    palette::remember(&mut recent, "Undo");
    assert_eq!(recent[0], "Undo", "run again, it moves to the front");
    assert_eq!(recent.iter().filter(|r| **r == "Undo").count(), 1);

    let first = palette::ordered("", &recent)[0].name;
    assert_eq!(first, "Undo");
}

#[test]
fn a_timecode_is_a_place_to_go_and_a_word_is_an_action() {
    assert!(palette::is_timecode_query("1:02"));
    assert!(palette::is_timecode_query(" +10"));
    assert!(palette::is_timecode_query("-5"));
    assert!(!palette::is_timecode_query("split"));
    assert!(!palette::is_timecode_query(""));
}

#[test]
fn named_markers_are_found_by_what_they_say() {
    let (mut editor, _events) = Editor::new_project("Places");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/song.mp4",
        MediaTime::from_seconds(20),
    ));
    editor.place_media(media).unwrap();
    use bettercut_editor_core::foundation::TimelineTime;
    editor
        .add_markers(&[TimelineTime::from_seconds(3), TimelineTime::from_seconds(9)])
        .unwrap();
    editor
        .set_marker_label(TimelineTime::from_seconds(9), "Second chorus")
        .unwrap();

    let found = palette::marker_matches(&editor, "chorus");
    assert_eq!(
        found,
        [("Second chorus".to_owned(), TimelineTime::from_seconds(9))]
    );
    assert!(
        palette::marker_matches(&editor, "").is_empty(),
        "nothing typed, no markers"
    );
    assert!(palette::marker_matches(&editor, "verse").is_empty());
}

#[test]
fn recent_projects_are_found_by_name() {
    let dir = std::env::temp_dir().join(format!("bettercut-palette-recent-{}", std::process::id()));
    let mut recent = bettercut_ui::recent::RecentProjects::stored_in(dir.join("recent.txt"));
    recent.touch(&dir.join("Wedding film.vproj"));
    recent.touch(&dir.join("Holiday.vproj"));
    let found = palette::project_matches(&recent, "wedd");
    assert_eq!(found, [dir.join("Wedding film.vproj")]);
    assert!(palette::project_matches(&recent, "").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_title_added_from_the_palette_is_selected() {
    let (mut editor, _events) = Editor::new_project("Adding");
    let mut state = UiState::default();
    assert!(palette::run(&mut editor, &mut state, "Add Title"));
    assert_eq!(state.selected_clips.len(), 1);
    let titles: usize = editor
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .map(|t| t.clips().len())
        .sum();
    assert_eq!(titles, 1);
}
