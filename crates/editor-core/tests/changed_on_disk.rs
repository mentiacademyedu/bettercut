//! A project file saved by another program while it is open — an assistant
//! through bettercut-mcp, a sync folder — is noticed; the editor's own saves
//! are not mistaken for one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;

#[test]
fn another_programs_save_is_noticed_and_our_own_is_not() {
    let dir = std::env::temp_dir().join(format!("bettercut-disk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shared.vproj");

    let (mut first, _first_events) = Editor::new_project("Shared");
    first.save_as(&path).unwrap();
    assert!(!first.changed_on_disk(), "its own save is not a change");

    // Some file systems keep modification times to the second or coarser;
    // wait long enough for the next write to look newer everywhere.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Another editor — what bettercut-mcp is — opens and saves the same file.
    let (mut other, _other_events) = Editor::open(&path).unwrap();
    other.add_text("From the assistant").unwrap();
    other.save().unwrap();

    assert!(
        first.changed_on_disk(),
        "a save by someone else goes unnoticed"
    );

    // "Keep Mine": the same change is not raised again.
    first.accept_disk_state();
    assert!(!first.changed_on_disk());

    // Loading it brings the other program's edit, and a clean slate.
    let (reloaded, _events) = Editor::open(&path).unwrap();
    assert!(!reloaded.changed_on_disk());
    let titles: usize = reloaded
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .map(|t| t.clips().len())
        .sum();
    assert_eq!(titles, 1);

    // A project never saved has no file to watch.
    let (fresh, _fresh_events) = Editor::new_project("Fresh");
    assert!(!fresh.changed_on_disk());
    let _ = std::fs::remove_dir_all(&dir);
}
