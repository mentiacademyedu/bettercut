//! The projects opened or saved most recently, newest first.
//!
//! Every other editor has this, and without it reopening yesterday's edit is
//! a trip through a file dialog to a folder the user has to remember.
//!
//! ## Stored as plain lines
//!
//! One path per line in the editor's per-user data folder, beside the
//! templates. Text rather than JSON because there is nothing to structure, and
//! a list someone can open in Notepad and fix by hand is a list that cannot be
//! corrupted beyond repair. A file that is missing, unreadable or full of
//! nonsense is an empty list, never an error: this is a convenience, and it
//! must not be able to stop the editor starting.
//!
//! ## Moved and deleted projects stay listed
//!
//! A project on an unplugged drive is still one the user wants back when the
//! drive returns. So a missing file is shown as missing rather than silently
//! dropped, and taken off the list only when the user picks it and it is still
//! not there — or clears the list.

use std::path::{Path, PathBuf};

/// How many projects are remembered. A menu longer than this is a list to read
/// rather than a shortcut.
pub const MAX_RECENT: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentProjects {
    /// Newest first, no repeats, at most [`MAX_RECENT`].
    paths: Vec<PathBuf>,
    /// Where the list is kept. `None` keeps it in memory only — what tests and
    /// a fresh `UiState` get, so nothing but the running editor ever writes
    /// the user's real list.
    file: Option<PathBuf>,
}

impl RecentProjects {
    /// The list kept in `file`, reading whatever is there now.
    pub fn stored_in(file: PathBuf) -> Self {
        let paths = std::fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            paths,
            file: Some(file),
        }
    }

    /// Where the running editor keeps its list: beside the user's templates.
    pub fn default_file() -> PathBuf {
        bettercut_editor_core::templates::library::user_dir()
            .parent()
            .map_or_else(std::env::temp_dir, Path::to_path_buf)
            .join("recent-projects.txt")
    }

    /// Newest first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// `path` was just opened or saved: first in the list, once.
    pub fn touch(&mut self, path: &Path) {
        self.paths.retain(|known| !same_file(known, path));
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_RECENT);
        self.store();
    }

    /// Take `path` off the list — it was picked and is not there any more.
    pub fn forget(&mut self, path: &Path) {
        self.paths.retain(|known| !same_file(known, path));
        self.store();
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.store();
    }

    /// Written on every change, so a crash loses nothing. Failure to write is
    /// logged and otherwise ignored, for the reason in the module docs.
    fn store(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let text: String = self
            .paths
            .iter()
            .map(|path| format!("{}\n", path.display()))
            .collect();
        let written = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, text));
        if let Err(err) = written {
            tracing::warn!(file = %file.display(), %err, "could not save the recent projects list");
        }
    }
}

/// The list in a stored file: one path per line, blanks and repeats skipped,
/// capped — so a hand-edited or damaged file still reads as a sensible list.
fn parse(text: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let path = PathBuf::from(line);
        if !paths.iter().any(|known| same_file(known, &path)) {
            paths.push(path);
        }
        if paths.len() == MAX_RECENT {
            break;
        }
    }
    paths
}

/// Whether two paths name the same project. Windows paths are not case
/// sensitive, so `C:\Edits\trip.vproj` and `c:\edits\Trip.vproj` are one entry
/// rather than two that open the same file.
fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// How a recent project is shown: its name, and the folder it is in so two
/// projects called "Untitled" can be told apart.
pub fn menu_label(path: &Path) -> (String, String) {
    let name = path.file_stem().map_or_else(
        || path.display().to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let folder = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    (name, folder)
}
