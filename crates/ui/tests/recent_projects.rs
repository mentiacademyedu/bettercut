//! The recent projects list (`bettercut_ui::recent`) and the Open menu's use
//! of it.
//!
//! What would annoy a user: the list in the wrong order, the same project
//! twice, a list that forgets itself on restart, a damaged list file stopping
//! the editor, a New or Open that wipes the list, and a recent project that
//! opens over unsaved work.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_editor_core::Editor;
use bettercut_ui::UiState;
use bettercut_ui::recent::{MAX_RECENT, RecentProjects};

fn p(name: &str) -> PathBuf {
    PathBuf::from(format!("C:/edits/{name}.vproj"))
}

/// Newest first, each project once, and no longer than the limit.
#[test]
fn the_newest_project_is_first_and_each_is_listed_once() {
    let mut recent = RecentProjects::default();
    recent.touch(&p("a"));
    recent.touch(&p("b"));
    recent.touch(&p("a"));
    assert_eq!(recent.paths(), &[p("a"), p("b")]);

    for n in 0..MAX_RECENT + 5 {
        recent.touch(&p(&format!("n{n}")));
    }
    assert_eq!(recent.paths().len(), MAX_RECENT);
    assert_eq!(recent.paths()[0], p(&format!("n{}", MAX_RECENT + 4)));
}

/// On Windows a path's case does not name a different file, so it must not
/// make a second entry that opens the same project.
#[cfg(windows)]
#[test]
fn a_path_in_different_case_is_the_same_project() {
    let mut recent = RecentProjects::default();
    recent.touch(Path::new("C:/Edits/Trip.vproj"));
    recent.touch(Path::new("c:/edits/trip.vproj"));
    assert_eq!(recent.paths().len(), 1);
}

/// The list survives a restart: what one editor stored, the next one reads.
/// Forgetting and clearing are stored too.
#[test]
fn the_list_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nested").join("recent.txt");

    let mut first = RecentProjects::stored_in(file.clone());
    first.touch(&p("old"));
    first.touch(&p("new"));
    assert_eq!(
        RecentProjects::stored_in(file.clone()).paths(),
        &[p("new"), p("old")]
    );

    first.forget(&p("old"));
    assert_eq!(RecentProjects::stored_in(file.clone()).paths(), &[p("new")]);

    first.clear();
    assert!(RecentProjects::stored_in(file).is_empty());
}

/// A list file that is missing, or was edited by hand into a mess, is still a
/// sensible list — never an error that could stop the editor starting.
#[test]
fn a_missing_or_damaged_file_is_a_sensible_list() {
    let dir = tempfile::tempdir().unwrap();
    assert!(RecentProjects::stored_in(dir.path().join("none.txt")).is_empty());

    let file = dir.path().join("messy.txt");
    let mut text =
        String::from("\n   \nC:/edits/a.vproj\nC:/edits/a.vproj\n  C:/edits/b.vproj  \n");
    for n in 0..30 {
        text.push_str(&format!("C:/edits/extra{n}.vproj\n"));
    }
    std::fs::write(&file, text).unwrap();

    let recent = RecentProjects::stored_in(file);
    assert_eq!(recent.paths().len(), MAX_RECENT);
    assert_eq!(&recent.paths()[..2], &[p("a"), p("b")]);

    // Not even text at all.
    let binary = dir.path().join("binary.txt");
    std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x9c]).unwrap();
    assert!(RecentProjects::stored_in(binary).is_empty());
}

/// A project saved to disk, for the open tests.
fn saved_project(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(format!("{name}.vproj"));
    let (mut editor, _events) = Editor::new_project(name);
    editor.save_as(&path).unwrap();
    path
}

/// Opening from the list opens the project, puts it first, and the list is
/// still there afterwards — opening a project resets the interface, and the
/// list belongs to the person, not the project.
#[test]
fn opening_a_recent_project_keeps_the_list_and_puts_it_first() {
    let dir = tempfile::tempdir().unwrap();
    let older = saved_project(dir.path(), "older");
    let newer = saved_project(dir.path(), "newer");

    let (mut editor, _events) = Editor::new_project("Current");
    let mut state = UiState::default();
    state.recent.touch(&older);
    state.recent.touch(&newer);

    bettercut_ui::panels::open_project_at(&mut editor, &mut state, &older);

    assert_eq!(editor.project().name, "older");
    assert_eq!(
        state.recent.paths(),
        &[older, newer],
        "the list was lost or not reordered"
    );
}

/// A project that has gone is taken off the list when picked, with a message,
/// and the current project stays open.
#[test]
fn a_project_that_has_gone_is_taken_off_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone.vproj");
    let (mut editor, _events) = Editor::new_project("Current");
    let mut state = UiState::default();
    state.recent.touch(&gone);

    bettercut_ui::panels::open_project_at(&mut editor, &mut state, &gone);

    assert!(state.recent.is_empty());
    assert_eq!(editor.project().name, "Current");
    let status = state.status.as_ref().expect("a message");
    assert!(
        status.is_error && status.text.contains("gone.vproj"),
        "{}",
        status.text
    );
}

/// A recent project one click away must not make throwing away an edit one
/// click away: unsaved work is asked about first (Save / Don't Save /
/// Cancel), and nothing is replaced until the question is answered.
#[test]
fn unsaved_work_is_not_replaced_by_a_recent_project() {
    let dir = tempfile::tempdir().unwrap();
    let other = saved_project(dir.path(), "other");
    let (mut editor, _events) = Editor::new_project("Current");
    editor
        .toggle_marker(bettercut_editor_core::foundation::TimelineTime::from_seconds(1))
        .unwrap();
    assert!(editor.is_dirty(), "setup: the edit is unsaved");
    let mut state = UiState::default();

    bettercut_ui::panels::open_project_at(&mut editor, &mut state, &other);

    assert_eq!(
        editor.project().name,
        "Current",
        "unsaved work was replaced"
    );
    assert_eq!(
        state.pending_switch,
        Some(bettercut_ui::save_prompt::Switch::OpenPath(other.clone())),
        "the question was not asked"
    );

    // "Don't Save": now the other project opens.
    bettercut_ui::save_prompt::proceed(
        &mut editor,
        &mut state,
        bettercut_ui::save_prompt::Switch::OpenPath(other),
    );
    assert_eq!(editor.project().name, "other");
}

/// Saving a project that already has a file puts it on the list, and a New
/// project keeps the list.
#[test]
fn saving_lists_the_project_and_new_keeps_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.vproj");
    let (mut editor, _events) = Editor::new_project("Saved");
    editor.save_as(&path).unwrap();
    let mut state = UiState::default();

    bettercut_ui::panels::save_project(&mut editor, &mut state);
    assert_eq!(state.recent.paths(), std::slice::from_ref(&path));

    bettercut_ui::panels::new_project(&mut editor, &mut state);
    assert_eq!(
        state.recent.paths(),
        &[path],
        "a new project wiped the list"
    );
}
