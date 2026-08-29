//! Abandoned recovery sessions, and not drowning in them (§38, §39).
//!
//! One recovery directory is created per editor session, and only a clean
//! shutdown removes its own. Every crash, every kill and every test that builds
//! an `Editor` leaves one behind — and nothing collected them, so on this
//! machine the temp directory reached nine thousand and startup spent nineteen
//! seconds replaying all of them to choose one.
//!
//! These run against a temp root of their own rather than the real one, because
//! the functions under test *delete directories* and a test that reaches into
//! the developer's actual recovery data would be the worst kind.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bettercut_editor_core::journal::{RecoveryPaths, SNAPSHOT_FILE};
use bettercut_editor_core::{Editor, recover};

/// A directory holding a snapshot that `recover` will accept.
fn plant_session(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("create");

    // Written by the project format itself, not by hand: a snapshot is a
    // versioned file, and a hand-rolled one would be rejected for the wrong
    // reason and make this test pass or fail for reasons unrelated to pruning.
    let (editor, _rx) = Editor::new_project("Planted");
    bettercut_editor_core::project_format::save(editor.project(), dir.join(SNAPSHOT_FILE))
        .expect("write snapshot");
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bettercut-recovery-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create");
    dir
}

/// The heart of it: choosing a session must not mean recovering every session.
/// Sorting by anything the snapshot *contains* costs a parse and a replay each,
/// which is exactly what made startup unusable.
#[test]
fn a_planted_session_is_recoverable() {
    let root = scratch("recoverable");
    let dir = plant_session(&root, "1234-abcd");

    let session = recover(RecoveryPaths { dir }).expect("planted session was not recovered");
    assert_eq!(session.failed, 0);
}

/// A directory with no snapshot in it is not a session. It is also the shape
/// left behind by a process that died before its first autosave, so there are
/// plenty of them.
#[test]
fn a_directory_with_no_snapshot_is_not_a_session() {
    let root = scratch("empty");
    let dir = root.join("1234-empty");
    std::fs::create_dir_all(&dir).expect("create");

    assert!(recover(RecoveryPaths { dir }).is_none());
}

/// The count bound. Age alone does not stop a bad afternoon — or a test suite —
/// from leaving thousands behind inside the window.
#[test]
fn the_newest_sessions_survive_a_prune_and_the_rest_do_not() {
    let root = scratch("bounded");

    // Ordered by the snapshot's modification time, so plant them with times
    // set explicitly rather than relying on how fast the filesystem clock
    // ticks — on Windows it does not tick fast enough to order sixty writes.
    let mut planted = Vec::new();
    for index in 0..60 {
        let dir = plant_session(&root, &format!("9999-{index:04}"));
        set_age(&dir, Duration::from_secs(60 * (60 - index) as u64));
        planted.push(dir);
    }

    let kept = prune_in(&root);

    assert_eq!(kept.len(), 50, "the bound was not applied");
    // The newest fifty: index 0 was given the *largest* age, so the survivors
    // are the tail.
    for dir in &planted[10..] {
        assert!(dir.exists(), "{} was pruned", dir.display());
    }
    for dir in &planted[..10] {
        assert!(!dir.exists(), "{} survived", dir.display());
    }
}

/// The age bound, which is what keeps the directory from filling with sessions
/// nobody will ever open again.
#[test]
fn sessions_past_the_cutoff_are_pruned_however_few_there_are() {
    let root = scratch("aged");

    let fresh = plant_session(&root, "1111-fresh");
    let stale = plant_session(&root, "1111-stale");
    set_age(&stale, Duration::from_secs(30 * 24 * 60 * 60));

    prune_in(&root);

    assert!(fresh.exists(), "a session from today was pruned");
    assert!(!stale.exists(), "a month-old session survived");
}

/// §39.5's spirit: recovery must never destroy the thing it is protecting. A
/// saved project's recovery data sits *beside the project file*, and pruning
/// only ever looks inside the temp root.
#[test]
fn a_projects_own_recovery_directory_is_never_in_the_temp_root() {
    let beside = RecoveryPaths::for_project(Some(Path::new("D:/work/film.vproj")), "1234-abcd");
    let temp_root = std::env::temp_dir().join("bettercut").join("recovery");

    assert!(
        !beside.dir.starts_with(&temp_root),
        "a saved project's recovery data would be in the pruner's reach: {}",
        beside.dir.display()
    );
    assert!(beside.dir.starts_with("D:/work"));

    // And an unsaved one *is* in reach, which is the whole point.
    let unsaved = RecoveryPaths::for_project(None, "1234-abcd");
    assert!(unsaved.dir.starts_with(&temp_root));
}

/// Backdate a session by rewriting its snapshot's modification time.
fn set_age(dir: &Path, age: Duration) {
    let when = SystemTime::now()
        .checked_sub(age)
        .expect("a time that far back");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(SNAPSHOT_FILE))
        .expect("open snapshot");
    file.set_modified(when).expect("set modified");
}

/// Apply the **real** pruning rule to a root of this test's own.
///
/// `stale_sessions` is the policy with no filesystem in it, which is exactly so
/// that this can exercise it without pointing a deletion routine at the
/// machine's actual recovery directory. Only the listing and the removal happen
/// here; the decision is the shipped one.
fn prune_in(root: &Path) -> Vec<PathBuf> {
    let mut sessions: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(root)
        .expect("read root")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|dir| {
            let modified = std::fs::metadata(dir.join(SNAPSHOT_FILE))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (dir, modified)
        })
        .collect();
    sessions.sort_by(|a, b| b.1.cmp(&a.1));

    // A prefix nothing here starts with: these sessions belong to no live
    // process, which is the case the rule is for.
    let doomed = bettercut_editor_core::stale_sessions(&sessions, SystemTime::now(), "no-such-pid-");
    for dir in &doomed {
        std::fs::remove_dir_all(dir).expect("remove");
    }

    sessions
        .into_iter()
        .map(|(dir, _)| dir)
        .filter(|dir| !doomed.contains(dir))
        .collect()
}

/// The current process's own sessions are in use, and pruning must not touch
/// them however old the machine thinks they are.
#[test]
fn this_processs_own_sessions_are_never_pruned() {
    let mine = format!("{}-", std::process::id());
    let sessions = vec![(
        PathBuf::from(format!("/tmp/bettercut/recovery/{mine}abcd")),
        SystemTime::UNIX_EPOCH,
    )];

    let doomed = bettercut_editor_core::stale_sessions(&sessions, SystemTime::now(), &mine);
    assert!(
        doomed.is_empty(),
        "the running session's own recovery data was marked for deletion"
    );
}
