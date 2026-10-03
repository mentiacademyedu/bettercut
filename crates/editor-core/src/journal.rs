//! The autosave command journal (§38.2).
//!
//! ```text
//! Edit -> append serialized command to journal (small, fast, append-only)
//! Every N commands or T seconds -> full atomic snapshot, truncate journal
//! ```
//!
//! # Why not just save the project
//!
//! §38.2 gives the reason: rewriting the whole project on every autosave does
//! not scale to §52's 10,000-clip benchmark. A command is a few hundred bytes;
//! a large project is megabytes.
//!
//! It is also *better* recovery, not merely cheaper. Debounced full saves lose
//! everything since the last write. A journal loses at most the one command in
//! flight, because every edit is appended the moment it succeeds.
//!
//! # Layout
//!
//! ```text
//! project.vproj          the user's file, written only when they ask
//! recovery/
//!     snapshot.vproj     last full snapshot
//!     journal.log        commands executed since that snapshot
//! ```
//!
//! Recovery is snapshot + replay (§39). That works only because commands are
//! deterministic — every ID a command creates is part of the request, so
//! replaying produces the identical project rather than a similar one.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::command::Command;
use crate::error::EditorError;

/// Commands appended before a snapshot is forced.
///
/// §38.2 says "every N commands or T seconds". 200 keeps replay short while
/// leaving snapshots rare enough to stay cheap on a big project.
pub const SNAPSHOT_EVERY_COMMANDS: u32 = 200;

/// Seconds before a snapshot is forced, if any commands are pending.
pub const SNAPSHOT_EVERY_SECONDS: u64 = 60;
/// The least and most a person may set that to: under ten seconds the disk
/// never rests, over ten minutes a crash costs real work.
pub const MIN_SNAPSHOT_SECONDS: u64 = 10;
pub const MAX_SNAPSHOT_SECONDS: u64 = 600;

pub const RECOVERY_DIR: &str = "recovery";
pub const SNAPSHOT_FILE: &str = "snapshot.vproj";
pub const JOURNAL_FILE: &str = "journal.log";

/// Where unsaved projects keep their recovery data, one folder per session:
/// the machine's temp folder — or `BETTERCUT_RECOVERY_ROOT` when set, which
/// the repository's cargo config does for every build and test, so a test
/// run never leaves work behind for the real app to offer back at its next
/// launch.
pub fn unsaved_root() -> PathBuf {
    std::env::var_os("BETTERCUT_RECOVERY_ROOT").map_or_else(
        || std::env::temp_dir().join("bettercut").join(RECOVERY_DIR),
        PathBuf::from,
    )
}

/// Where a project's recovery data lives.
///
/// Beside the project file when it has been saved, so recovery data travels
/// with the project. An unsaved project uses the OS temp directory keyed by a
/// session id — losing an hour of work because the user had not chosen a
/// filename yet is exactly the failure §38 exists to prevent.
#[derive(Debug, Clone)]
pub struct RecoveryPaths {
    pub dir: PathBuf,
}

impl RecoveryPaths {
    pub fn for_project(project_path: Option<&Path>, session: &str) -> Self {
        // One folder per project file. The whole `recovery` folder used to
        // be the project's, so two projects saved side by side wrote over
        // each other's crash data — and opening one could offer the other's.
        let dir = match (
            project_path.and_then(Path::parent),
            project_path.and_then(Path::file_name),
        ) {
            (Some(parent), Some(name)) => parent.join(RECOVERY_DIR).join(name),
            _ => unsaved_root().join(session),
        };
        Self { dir }
    }

    /// Where a saved project's recovery data lived before it had a folder of
    /// its own: the shared `recovery` folder beside it. Still read after a
    /// crash, so work from before the change is not lost — but only accepted
    /// when the snapshot there is this project's.
    pub fn shared_beside(project_path: &Path) -> Option<Self> {
        Some(Self {
            dir: project_path.parent()?.join(RECOVERY_DIR),
        })
    }

    pub fn snapshot(&self) -> PathBuf {
        self.dir.join(SNAPSHOT_FILE)
    }

    pub fn journal(&self) -> PathBuf {
        self.dir.join(JOURNAL_FILE)
    }

    pub fn exists(&self) -> bool {
        self.snapshot().exists()
    }
}

/// Append-only log of executed commands.
pub struct Journal {
    paths: RecoveryPaths,
    file: Option<File>,
    since_snapshot: u32,
    last_snapshot: std::time::Instant,
    /// Seconds before a snapshot is forced: [`SNAPSHOT_EVERY_SECONDS`]
    /// unless the interface asked otherwise.
    snapshot_every_seconds: u64,
    /// Appends that failed. Non-zero means recovery would be incomplete, and
    /// the user deserves to know rather than find out after a crash.
    write_failures: u32,
}

impl Journal {
    pub fn new(paths: RecoveryPaths) -> Self {
        Self {
            paths,
            file: None,
            since_snapshot: 0,
            last_snapshot: std::time::Instant::now(),
            snapshot_every_seconds: SNAPSHOT_EVERY_SECONDS,
            write_failures: 0,
        }
    }

    pub fn paths(&self) -> &RecoveryPaths {
        &self.paths
    }

    pub fn write_failures(&self) -> u32 {
        self.write_failures
    }

    pub fn pending_commands(&self) -> u32 {
        self.since_snapshot
    }

    /// True when no snapshot exists yet.
    ///
    /// Replay is *snapshot plus journal*, so a journal with nothing underneath
    /// it recovers nothing. The baseline has to be written before the first
    /// command is appended — and specifically before that command *executes*,
    /// or the snapshot would already contain it and replay would apply it
    /// twice.
    pub fn needs_baseline(&self) -> bool {
        !self.paths.snapshot().exists()
    }

    /// Record a command that has just executed successfully.
    ///
    /// Only successful commands are journalled: a rejected edit changed
    /// nothing, and replaying it would fail the same way for the same reason.
    ///
    /// Errors are counted, logged, and swallowed. A full disk must not stop the
    /// user editing (§50) — but it must not silently pretend to be protecting
    /// their work either, which is what `write_failures` is for.
    pub fn append(&mut self, command: &Command) {
        self.append_all(std::slice::from_ref(command));
    }

    /// Record several commands that executed together — one undo step's
    /// worth — with one trip to the disk. A sync per command made a split
    /// into a thousand pieces take forty-five seconds; the step is the unit
    /// worth making durable, not its parts.
    pub fn append_all(&mut self, commands: &[Command]) {
        if commands.is_empty() {
            return;
        }
        if let Err(err) = self.try_append(commands) {
            self.write_failures += 1;
            tracing::error!(%err, "could not append to the autosave journal");
        }
    }

    fn try_append(&mut self, commands: &[Command]) -> std::io::Result<()> {
        let mut lines = String::new();
        for command in commands {
            let line = serde_json::to_string(command)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            lines.push_str(&line);
            lines.push('\n');
        }

        let file = match &mut self.file {
            Some(file) => file,
            None => {
                std::fs::create_dir_all(&self.paths.dir)?;
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.paths.journal())?;
                self.file.insert(file)
            }
        };

        // One command per line, so a torn write at the end of the file costs
        // exactly the commands it cut rather than the whole journal.
        file.write_all(lines.as_bytes())?;
        // Without this the journal is only as durable as the OS cache, which
        // defeats the point of journalling at all.
        file.sync_data()?;

        self.since_snapshot += u32::try_from(commands.len()).unwrap_or(u32::MAX);
        Ok(())
    }

    /// Should a full snapshot be taken now? (§38.2's "every N commands or T
    /// seconds".)
    /// How often a snapshot is forced, in seconds, held to
    /// [`MIN_SNAPSHOT_SECONDS`]..=[`MAX_SNAPSHOT_SECONDS`].
    pub fn set_snapshot_every_seconds(&mut self, seconds: u64) {
        self.snapshot_every_seconds = seconds.clamp(MIN_SNAPSHOT_SECONDS, MAX_SNAPSHOT_SECONDS);
    }

    pub fn snapshot_every_seconds(&self) -> u64 {
        self.snapshot_every_seconds
    }

    pub fn snapshot_is_due(&self) -> bool {
        if self.since_snapshot == 0 {
            return false;
        }
        self.since_snapshot >= SNAPSHOT_EVERY_COMMANDS
            || self.last_snapshot.elapsed().as_secs() >= self.snapshot_every_seconds
    }

    /// Write a full snapshot and truncate the journal.
    ///
    /// The snapshot is written first and the journal cleared only after it
    /// succeeds. Truncating first would leave a window where neither file holds
    /// the recent edits.
    pub fn snapshot(
        &mut self,
        project: &bettercut_project_format::Project,
    ) -> Result<(), EditorError> {
        std::fs::create_dir_all(&self.paths.dir).map_err(|source| {
            EditorError::Project(bettercut_project_format::ProjectError::Io {
                path: self.paths.dir.clone(),
                source,
            })
        })?;

        // Atomic, exactly like a user-initiated save (§38.1).
        bettercut_project_format::save(project, self.paths.snapshot())?;

        // Now the journal can go.
        self.file = None;
        if self.paths.journal().exists()
            && let Err(err) = std::fs::remove_file(self.paths.journal())
        {
            tracing::warn!(%err, "could not truncate the journal after snapshotting");
        }

        self.since_snapshot = 0;
        self.last_snapshot = std::time::Instant::now();
        tracing::debug!(clips = project.clip_count(), "autosave snapshot written");
        Ok(())
    }

    /// Remove the recovery data.
    ///
    /// Called after a clean save and on orderly shutdown: leaving it behind
    /// makes the next launch offer to recover work the user already has (§39).
    pub fn discard(&mut self) {
        self.file = None;
        if self.paths.dir.exists()
            && let Err(err) = std::fs::remove_dir_all(&self.paths.dir)
        {
            tracing::debug!(%err, "could not remove the recovery directory");
        }
        self.since_snapshot = 0;
    }
}

/// Read the commands a journal holds.
///
/// A truncated final line is **dropped, not an error**. That is precisely the
/// crash this whole mechanism exists for: the process died mid-write. Every
/// complete command before it is still good, and refusing to load them because
/// the last one is torn would throw away the work we just went to such trouble
/// to protect.
pub fn read_journal(path: &Path) -> Vec<Command> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };

    let mut commands = Vec::new();
    let mut skipped = 0;

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Command>(&line) {
            Ok(command) => commands.push(command),
            Err(_) => skipped += 1,
        }
    }

    if skipped > 0 {
        tracing::warn!(
            skipped,
            "journal contained unreadable entries; they were skipped"
        );
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{SequenceId, TrackId};
    use bettercut_project_format::Project;

    fn rename(name: &str) -> Command {
        Command::RenameProject {
            name: name.to_owned(),
        }
    }

    fn add_track() -> Command {
        Command::AddTrack {
            sequence: SequenceId::new(),
            kind: crate::command::TrackKindRepr::Video,
            name: "V2".to_owned(),
            id: TrackId::new(),
        }
    }

    fn journal_in(dir: &Path) -> Journal {
        Journal::new(RecoveryPaths {
            dir: dir.to_path_buf(),
        })
    }

    #[test]
    fn appended_commands_can_be_read_back_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = journal_in(dir.path());

        journal.append(&rename("one"));
        journal.append(&rename("two"));
        journal.append(&rename("three"));

        let read = read_journal(&journal.paths().journal());
        assert_eq!(read.len(), 3);
        assert_eq!(read[0], rename("one"));
        assert_eq!(read[2], rename("three"));
        assert_eq!(journal.write_failures(), 0);
    }

    #[test]
    fn commands_carrying_generated_ids_survive_the_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = journal_in(dir.path());

        let command = add_track();
        journal.append(&command);

        let read = read_journal(&journal.paths().journal());
        assert_eq!(read, vec![command], "ids did not survive journalling");
    }

    /// The crash this whole mechanism exists for: the process died mid-write.
    /// Everything before the torn line must still load.
    #[test]
    fn a_torn_final_line_costs_only_that_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = journal_in(dir.path());
        journal.append(&rename("one"));
        journal.append(&rename("two"));

        // Simulate a half-written third entry.
        let path = journal.paths().journal();
        let mut text = std::fs::read_to_string(&path).expect("read");
        text.push_str("{\"op\":\"rename_pro");
        std::fs::write(&path, text).expect("write");

        let read = read_journal(&path);
        assert_eq!(read.len(), 2, "a torn tail discarded good commands");
        assert_eq!(read[0], rename("one"));
    }

    #[test]
    fn a_missing_journal_reads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(read_journal(&dir.path().join("nothing.log")).is_empty());
    }

    #[test]
    fn snapshotting_writes_the_project_and_clears_the_journal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = journal_in(dir.path());
        journal.append(&rename("one"));
        assert_eq!(journal.pending_commands(), 1);

        let project = Project::new("Snapshotted");
        journal.snapshot(&project).expect("snapshot");

        assert!(journal.paths().snapshot().exists());
        assert!(
            !journal.paths().journal().exists(),
            "journal survived the snapshot"
        );
        assert_eq!(journal.pending_commands(), 0);

        // And the snapshot is a real, loadable project.
        let loaded = bettercut_project_format::load(journal.paths().snapshot()).expect("load");
        assert_eq!(loaded.name, "Snapshotted");
    }

    #[test]
    fn appending_after_a_snapshot_starts_a_fresh_journal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = journal_in(dir.path());
        journal.append(&rename("before"));
        journal.snapshot(&Project::new("p")).expect("snapshot");

        journal.append(&rename("after"));

        let read = read_journal(&journal.paths().journal());
        assert_eq!(read, vec![rename("after")], "old commands came back");
    }

    #[test]
    fn a_snapshot_is_due_after_enough_commands() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut journal = journal_in(dir.path());

        assert!(!journal.snapshot_is_due(), "due with nothing pending");

        for i in 0..SNAPSHOT_EVERY_COMMANDS {
            journal.append(&rename(&format!("n{i}")));
        }
        assert!(journal.snapshot_is_due());
    }

    /// §50: a full disk must not stop the user editing — but it must not let
    /// the editor quietly pretend to be protecting their work either. The count
    /// is what the status bar reads to say so.
    #[test]
    fn a_journal_that_cannot_write_counts_the_failures() {
        let dir = tempfile::tempdir().expect("tempdir");

        // A *file* where the recovery directory should be, so creating it
        // fails the way a full disk or a read-only folder does.
        let blocked = dir.path().join("recovery");
        std::fs::write(&blocked, b"in the way").expect("write");

        let mut journal = Journal::new(RecoveryPaths { dir: blocked });
        assert_eq!(journal.write_failures(), 0);

        journal.append(&rename("first"));
        journal.append(&rename("second"));

        assert_eq!(
            journal.write_failures(),
            2,
            "the journal swallowed the failures without counting them"
        );
    }

    #[test]
    fn discarding_removes_the_recovery_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("recovery");
        let mut journal = journal_in(&nested);

        journal.append(&rename("one"));
        journal.snapshot(&Project::new("p")).expect("snapshot");
        assert!(nested.exists());

        journal.discard();
        assert!(!nested.exists(), "recovery data survived discard");
    }

    #[test]
    fn recovery_paths_sit_beside_a_saved_project() {
        let paths = RecoveryPaths::for_project(Some(Path::new("C:/films/cut.vproj")), "session");
        assert_eq!(
            paths.dir,
            Path::new("C:/films").join(RECOVERY_DIR).join("cut.vproj")
        );
        // Each project in a folder has its own.
        let other = RecoveryPaths::for_project(Some(Path::new("C:/films/trailer.vproj")), "s");
        assert_ne!(paths.dir, other.dir);
    }

    /// An unsaved project still gets protection: §38 is about not losing work,
    /// and work is most easily lost before the user has chosen a filename.
    #[test]
    fn an_unsaved_project_still_gets_a_recovery_location() {
        let paths = RecoveryPaths::for_project(None, "abc123");
        assert!(paths.dir.starts_with(unsaved_root()));
        assert!(paths.dir.ends_with("abc123"));
    }
}
