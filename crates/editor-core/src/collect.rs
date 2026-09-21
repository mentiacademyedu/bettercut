//! Collect: the project and every file it uses, copied into one folder.
//!
//! What it is for: handing an edit to somebody else, moving it to another
//! drive, or putting it away when the job is finished. A project file is a
//! list of paths into whatever folders the footage happened to be in, and
//! e-mailing that to a colleague hands them a file that opens with everything
//! missing (§66).
//!
//! What it does *not* do is move or delete anything (§2): the originals stay
//! where they are, and the collected copy is a second set of files with a
//! project pointing at them.

use std::path::{Path, PathBuf};

use crate::editor::Editor;
use crate::error::EditorError;

/// What a collect produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collected {
    /// The project file written in the folder.
    pub project: PathBuf,
    /// Files copied in.
    pub copied: usize,
    /// Files that could not be copied, with why — a missing original, or a
    /// folder that refused to be written to. The project is still written, so
    /// what *did* come across is usable.
    pub missing: Vec<String>,
    /// Bytes copied, for the "that is 40 GB, are you sure" a caller may want
    /// to ask before starting.
    pub bytes: u64,
}

/// The folder media is copied into, inside the collected folder.
pub const MEDIA_FOLDER: &str = "media";

impl Editor {
    /// How much the files this project uses add up to, in bytes, and how many
    /// there are — what a caller shows before starting.
    ///
    /// Generated entries (a colour clip, a compound) have no file and count
    /// for nothing.
    pub fn collect_size(&self) -> (usize, u64) {
        let mut files = 0;
        let mut bytes = 0;
        for asset in &self.project().media {
            if asset.generated.is_some() || asset.path.as_os_str().is_empty() {
                continue;
            }
            files += 1;
            bytes += std::fs::metadata(&asset.path).map(|m| m.len()).unwrap_or(0);
        }
        (files, bytes)
    }

    /// Copy the project and every file it uses into `folder`, and write a
    /// project there that points at the copies.
    ///
    /// The folder is made if it is not there. A file already in the folder
    /// with the same name and size is left alone rather than copied again, so
    /// collecting twice over the same folder is quick and safe.
    pub fn collect_into(&self, folder: impl AsRef<Path>) -> Result<Collected, EditorError> {
        let folder = folder.as_ref();
        let media_folder = folder.join(MEDIA_FOLDER);
        std::fs::create_dir_all(&media_folder).map_err(|e| EditorError::Io(e.to_string()))?;

        // A copy of the project to rewrite: the one on screen keeps pointing
        // at the originals, because the user is still editing with it.
        let mut project = self.project().clone();
        let mut copied = 0;
        let mut bytes = 0;
        let mut missing = Vec::new();
        let mut taken: Vec<String> = Vec::new();

        for asset in &mut project.media {
            if asset.generated.is_some() || asset.path.as_os_str().is_empty() {
                continue;
            }
            let name = unique_name(&asset.path, &mut taken);
            let destination = media_folder.join(&name);
            match copy_if_needed(&asset.path, &destination) {
                Ok(Some(size)) => {
                    copied += 1;
                    bytes += size;
                }
                // Already there, the same size: nothing to do, and it still
                // counts as collected.
                Ok(None) => copied += 1,
                Err(why) => {
                    missing.push(format!("{}: {why}", asset.file_name));
                    // Left pointing at the original, which at least opens on
                    // this machine. §66's relink is the way back otherwise.
                    continue;
                }
            }
            asset.path = destination;
        }

        let name = self
            .path()
            .and_then(|path| path.file_name().map(|n| n.to_os_string()))
            .unwrap_or_else(|| {
                let mut name = std::ffi::OsString::from(sanitise(&project.name));
                name.push(".");
                name.push(bettercut_project_format::PROJECT_EXTENSION);
                name
            });
        let project_path = folder.join(name);
        bettercut_project_format::save(&project, &project_path)?;

        Ok(Collected {
            project: project_path,
            copied,
            missing,
            bytes,
        })
    }
}

/// Copy `from` to `to` unless an identical-looking file is already there.
///
/// `Ok(None)` means it was already there. Size rather than contents: a hash of
/// forty gigabytes to avoid copying forty gigabytes is the wrong trade, and a
/// file of the same name *and* size in a folder this made is the same file.
fn copy_if_needed(from: &Path, to: &Path) -> Result<Option<u64>, String> {
    let source = std::fs::metadata(from).map_err(|e| e.to_string())?;
    if let Ok(existing) = std::fs::metadata(to)
        && existing.len() == source.len()
    {
        return Ok(None);
    }
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    Ok(Some(source.len()))
}

/// A file name for `path` that nothing else has taken in this folder.
///
/// Two shots called `clip.mp4` from two cards is the normal case, not the
/// exception, and the second must not land on the first.
fn unique_name(path: &Path, taken: &mut Vec<String>) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    if !taken.iter().any(|seen| seen.eq_ignore_ascii_case(&name)) {
        taken.push(name.clone());
        return name;
    }
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let unique = (2..)
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| {
            !taken
                .iter()
                .any(|seen| seen.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(name);
    taken.push(unique.clone());
    unique
}

/// A project name turned into something a file system will accept.
fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().to_owned();
    if trimmed.is_empty() {
        "project".to_owned()
    } else {
        trimmed
    }
}
