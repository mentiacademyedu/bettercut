//! Crash recovery (§39).
//!
//! ```text
//! On launch:
//! 1. Detect incomplete previous session
//! 2. Locate most recent snapshot
//! 3. Replay journal
//! 4. Offer project recovery
//! 5. Never overwrite the original project automatically
//! ```
//!
//! Step 5 is the one that matters most and the easiest to get wrong. Recovery
//! produces a project *offered* to the user; it does not touch their file. A
//! recovery that silently overwrites is worse than no recovery at all, because
//! it can destroy a good file using a stale journal.

use bettercut_project_format::Project;

use crate::command::Command;
use crate::journal::{RecoveryPaths, read_journal};

/// Work found from a session that did not shut down cleanly.
#[derive(Debug)]
pub struct RecoverableSession {
    /// The rebuilt project — snapshot plus replayed journal.
    pub project: Project,
    /// Where the recovery data came from, so it can be discarded once the user
    /// has decided what to do.
    pub paths: RecoveryPaths,
    /// How many journalled commands replayed successfully.
    pub replayed: usize,
    /// Commands that would not replay.
    ///
    /// Non-zero means the recovered project is missing some of the user's most
    /// recent edits, which they should be told rather than left to discover.
    pub failed: usize,
}

impl RecoverableSession {
    /// Take the recovered project and remove the recovery data.
    ///
    /// Called once the user has accepted it: the work now lives in the editor,
    /// and leaving the files behind would make the *next* launch offer the same
    /// recovery again.
    pub fn accept(mut self) -> Project {
        self.discard_files();
        self.project
    }

    /// Throw the recovered work away, at the user's explicit request.
    pub fn discard(mut self) {
        self.discard_files();
    }

    fn discard_files(&mut self) {
        if self.paths.dir.exists()
            && let Err(err) = std::fs::remove_dir_all(&self.paths.dir)
        {
            tracing::warn!(%err, "could not remove recovery data");
        }
    }

    /// A short line for the recovery prompt.
    pub fn summary(&self) -> String {
        let clips = self.project.clip_count();
        match self.failed {
            0 => format!(
                "Recovered “{}” with {clips} clip(s) and {} unsaved edit(s)",
                self.project.name, self.replayed
            ),
            failed => format!(
                "Recovered “{}” with {clips} clip(s) and {} unsaved edit(s); \
                 {failed} could not be replayed",
                self.project.name, self.replayed
            ),
        }
    }
}

/// Look for a recoverable session at `paths`.
///
/// Returns `None` when there is nothing to recover, which is the normal case
/// and must never be reported as an error.
pub fn recover(paths: RecoveryPaths) -> Option<RecoverableSession> {
    if !paths.exists() {
        return None;
    }

    let snapshot = paths.snapshot();
    let mut project = match bettercut_project_format::load(&snapshot) {
        Ok(project) => project,
        Err(err) => {
            // A corrupt snapshot is not recoverable, but it is also not a
            // reason to refuse to start (§50).
            tracing::error!(path = %snapshot.display(), %err, "recovery snapshot is unreadable");
            return None;
        }
    };

    let commands = read_journal(&paths.journal());
    let (replayed, failed) = replay(&mut project, commands);

    tracing::info!(
        path = %snapshot.display(),
        replayed,
        failed,
        "recovered a session that did not shut down cleanly"
    );

    Some(RecoverableSession {
        project,
        paths,
        replayed,
        failed,
    })
}

/// Find recoverable sessions from projects that were never saved.
///
/// A saved project's recovery data sits beside it and is found by path. An
/// unsaved one has no path, so its data goes to a per-session temp directory —
/// and the only way to find it later is to look.
///
/// §38 exists to stop people losing work, and work is most easily lost *before*
/// a filename has been chosen. Skipping this case would leave the largest gap
/// unprotected.
pub fn scan_unsaved() -> Vec<RecoverableSession> {
    // Newest first, by the snapshot's own timestamp — the session that was
    // lost is the one that was open most recently.
    //
    // Ordered by a `stat` rather than by replaying: recovering a session means
    // parsing a project and re-executing its journal, and doing that to every
    // directory in order to *choose* one costs the whole startup. On this
    // machine that was nineteen seconds before the window appeared, against a
    // temp directory holding nine thousand of them.
    let mut candidates = unsaved_sessions();
    candidates.truncate(OFFER_AT_MOST);

    candidates
        .into_iter()
        .filter_map(|(dir, _)| recover(RecoveryPaths { dir }))
        .collect()
}

/// The most sessions to attempt recovery on.
///
/// The interface offers one. A handful rather than one exactly, so a snapshot
/// that turns out to be unreadable falls through to the next instead of leaving
/// the user with nothing.
const OFFER_AT_MOST: usize = 5;

/// How long an unsaved session is kept before it is assumed abandoned.
///
/// Long enough to cover a weekend away from the machine, which is the case that
/// matters: an unsaved project is the work §38 exists to protect. Short enough
/// that the directory does not grow without bound.
pub const KEEP_UNSAVED_FOR: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// How many unsaved sessions are kept regardless of age.
///
/// A hard bound as well as the age one, because age alone does not stop a bad
/// afternoon — or a test suite — from leaving thousands behind inside the
/// window.
pub const KEEP_UNSAVED_AT_MOST: usize = 50;

/// Delete abandoned unsaved-session directories. Returns how many went.
///
/// These accumulate: one per editor session, and only a clean shutdown removes
/// its own. Every crash, every kill, and every test that constructs an `Editor`
/// leaves one behind, and nothing ever collected them.
///
/// Deliberately narrow about what it will delete:
///
/// * only inside the temp root — a saved project's recovery data sits beside
///   the project file and is never touched here (§39.5);
/// * never this process's own directories, which are in use;
/// * newest first, so what survives is what someone might actually want.
pub fn prune_unsaved() -> usize {
    let doomed = stale_sessions(
        &unsaved_sessions(),
        std::time::SystemTime::now(),
        &format!("{}-", std::process::id()),
    );

    let mut removed = 0;
    for dir in doomed {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => removed += 1,
            // Another instance may be using it, or have removed it already.
            // Neither is worth a word to the user.
            Err(err) => tracing::debug!(path = %dir.display(), %err, "could not prune"),
        }
    }

    if removed > 0 {
        tracing::info!(removed, "pruned abandoned recovery sessions");
    }
    removed
}

/// Which of `sessions` should go — the rule, with no filesystem in it.
///
/// Separated from the deleting so it can be tested against a list rather than
/// against the machine's real recovery directory. A test that had to plant
/// files in the actual temp root to check a *deletion* policy would be one
/// mistake away from removing the developer's unsaved work.
///
/// `sessions` must be newest first, as [`unsaved_sessions`] returns them: the
/// index is then how many newer ones there are, which is what the count bound
/// is about. `mine` is the current process's directory-name prefix, whose
/// sessions are in use and never candidates.
pub fn stale_sessions(
    sessions: &[(std::path::PathBuf, std::time::SystemTime)],
    now: std::time::SystemTime,
    mine: &str,
) -> Vec<std::path::PathBuf> {
    let cutoff = now
        .checked_sub(KEEP_UNSAVED_FOR)
        .unwrap_or(std::time::UNIX_EPOCH);

    sessions
        .iter()
        .enumerate()
        .filter(|(index, (dir, modified))| {
            let is_mine = dir
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with(mine));
            !is_mine && (*index >= KEEP_UNSAVED_AT_MOST || *modified < cutoff)
        })
        .map(|(_, (dir, _))| dir.clone())
        .collect()
}

/// Every unsaved-session directory and when its snapshot was last written,
/// newest first. Stat only — nothing is parsed.
pub fn unsaved_sessions() -> Vec<(std::path::PathBuf, std::time::SystemTime)> {
    let root = crate::journal::unsaved_root();

    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut sessions: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.is_dir())
        .map(|dir| {
            // A directory with no snapshot has nothing to recover and is still
            // worth pruning, so it is kept in the list at the epoch — which
            // sorts it last, and past any cutoff.
            let modified = std::fs::metadata(dir.join(crate::journal::SNAPSHOT_FILE))
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (dir, modified)
        })
        .collect();

    // Newest first, so the index is how many newer ones there are.
    sessions.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    sessions
}

/// Apply journalled commands to a snapshot.
///
/// A command that fails is **skipped, not fatal**. Replay is best-effort by
/// nature: the journal may end mid-sequence, and stopping at the first failure
/// would discard every later edit that would have applied cleanly.
///
/// Returns `(replayed, failed)`.
fn replay(project: &mut Project, commands: Vec<Command>) -> (usize, usize) {
    let mut replayed = 0;
    let mut failed = 0;

    for command in commands {
        match crate::ops::build_for_replay(command) {
            Ok(mut built) => match built.execute(project) {
                Ok(()) => replayed += 1,
                Err(err) => {
                    tracing::warn!(%err, "a journalled command would not replay");
                    failed += 1;
                }
            },
            Err(err) => {
                tracing::warn!(%err, "a journalled command could not be rebuilt");
                failed += 1;
            }
        }
    }

    (replayed, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;
    use crate::{Editor, TrackKindRepr};
    use bettercut_foundation::TrackId;

    fn paths_in(dir: &std::path::Path) -> RecoveryPaths {
        RecoveryPaths {
            dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn nothing_to_recover_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(recover(paths_in(&dir.path().join("absent"))).is_none());
    }

    /// The whole point: a snapshot plus a journal reconstructs the state the
    /// user had when the process died.
    #[test]
    fn a_snapshot_plus_journal_rebuilds_the_edits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = Journal::new(paths_in(dir.path()));

        // Snapshot an empty project, then journal two edits.
        let project = Project::new("Original");
        let sequence = project.active().expect("sequence").id;
        journal.snapshot(&project).expect("snapshot");

        journal.append(&Command::RenameProject {
            name: "Renamed".to_owned(),
        });
        journal.append(&Command::AddTrack {
            sequence,
            kind: TrackKindRepr::Video,
            name: "V2".to_owned(),
            id: TrackId::new(),
        });

        let recovered = recover(paths_in(dir.path())).expect("something to recover");

        assert_eq!(recovered.replayed, 2);
        assert_eq!(recovered.failed, 0);
        assert_eq!(recovered.project.name, "Renamed");
        assert_eq!(
            recovered
                .project
                .active()
                .expect("sequence")
                .video_tracks
                .len(),
            2,
            "the journalled track was not replayed"
        );
    }

    /// Replay must be deterministic: the recovered project has to be identical
    /// to the one that was lost, not merely similar.
    #[test]
    fn replay_reproduces_identical_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = Journal::new(paths_in(dir.path()));

        let project = Project::new("Deterministic");
        let sequence = project.active().expect("sequence").id;
        journal.snapshot(&project).expect("snapshot");

        let track_id = TrackId::new();
        journal.append(&Command::AddTrack {
            sequence,
            kind: TrackKindRepr::Video,
            name: "V2".to_owned(),
            id: track_id,
        });

        let recovered = recover(paths_in(dir.path())).expect("recoverable");
        assert_eq!(
            recovered.project.active().expect("sequence").video_tracks[1].id,
            track_id,
            "replay invented a new track id"
        );
    }

    /// A command that no longer applies must not abort the rest of the replay.
    #[test]
    fn a_command_that_will_not_replay_is_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = Journal::new(paths_in(dir.path()));

        let project = Project::new("Partial");
        journal.snapshot(&project).expect("snapshot");

        // Refers to a sequence that does not exist in the snapshot.
        journal.append(&Command::AddTrack {
            sequence: bettercut_foundation::SequenceId::new(),
            kind: TrackKindRepr::Video,
            name: "orphan".to_owned(),
            id: TrackId::new(),
        });
        journal.append(&Command::RenameProject {
            name: "Still Applied".to_owned(),
        });

        let recovered = recover(paths_in(dir.path())).expect("recoverable");
        assert_eq!(recovered.failed, 1);
        assert_eq!(recovered.replayed, 1);
        assert_eq!(
            recovered.project.name, "Still Applied",
            "a later command was dropped because an earlier one failed"
        );
    }

    #[test]
    fn an_unreadable_snapshot_does_not_stop_the_app_starting() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path()).expect("mkdir");
        std::fs::write(dir.path().join(crate::journal::SNAPSHOT_FILE), b"not json").expect("write");

        assert!(recover(paths_in(dir.path())).is_none());
    }

    /// §39.5: recovery must never overwrite the user's file by itself.
    #[test]
    fn recovering_does_not_touch_the_original_project_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("original.vproj");

        let (mut editor, _rx) = Editor::new_project("Saved");
        editor.save_as(&original).expect("save");
        let before = std::fs::read(&original).expect("read");

        let mut journal = Journal::new(paths_in(&dir.path().join("recovery")));
        journal
            .snapshot(&Project::new("Crashed"))
            .expect("snapshot");
        journal.append(&Command::RenameProject {
            name: "Edited After Crash".to_owned(),
        });

        let recovered = recover(paths_in(&dir.path().join("recovery"))).expect("recoverable");
        assert_eq!(recovered.project.name, "Edited After Crash");

        let after = std::fs::read(&original).expect("read");
        assert_eq!(before, after, "recovery modified the user's project file");
    }

    #[test]
    fn the_summary_mentions_lost_edits_when_there_are_any() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = Journal::new(paths_in(dir.path()));
        journal.snapshot(&Project::new("p")).expect("snapshot");
        journal.append(&Command::AddTrack {
            sequence: bettercut_foundation::SequenceId::new(),
            kind: TrackKindRepr::Video,
            name: "orphan".to_owned(),
            id: TrackId::new(),
        });

        let recovered = recover(paths_in(dir.path())).expect("recoverable");
        assert!(
            recovered.summary().contains("could not be replayed"),
            "the summary hid a failed replay: {}",
            recovered.summary()
        );
    }
}
