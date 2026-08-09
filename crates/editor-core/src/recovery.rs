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
    let root = std::env::temp_dir()
        .join("bettercut")
        .join(crate::journal::RECOVERY_DIR);

    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let paths = RecoveryPaths { dir: entry.path() };
        if let Some(session) = recover(paths) {
            found.push(session);
        }
    }

    // Most recent first, so the prompt offers the likeliest candidate.
    found.sort_by_key(|session| std::cmp::Reverse(session.replayed));
    found
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
