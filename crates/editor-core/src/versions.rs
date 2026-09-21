//! Earlier versions of a project, kept as it is saved.
//!
//! Every Save over an existing project file first copies the file that is
//! about to be replaced into a folder beside it — `trip versions/` for
//! `trip.vproj` — named for when it was saved, so the folder lists oldest to
//! newest by name alone. The newest [`KEPT_VERSIONS`] are kept; older ones are
//! removed as new ones arrive.
//!
//! Restoring one never loses anything either: the project as it stands is
//! kept as a version first, so a restore can itself be undone by restoring
//! that.

use std::path::{Path, PathBuf};

/// How many earlier versions are kept.
pub const KEPT_VERSIONS: usize = 20;

/// One kept version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub path: PathBuf,
    /// When it was kept, for a list: "2026-09-14 01:32:05" (UTC).
    pub label: String,
}

/// The folder a project's versions go in: beside it, named after it.
pub fn versions_folder(project: &Path) -> PathBuf {
    let stem = project.file_stem().map_or_else(
        || "project".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    project.with_file_name(format!("{stem} versions"))
}

/// Seconds since 1970 as a UTC date and time, `(y, mo, d, h, mi, s)`.
fn civil(seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3_600) as u32,
        (rest % 3_600 / 60) as u32,
        (rest % 60) as u32,
    )
}

/// A version's file name: the project's name and when, sortable.
pub fn version_file_name(stem: &str, seconds: u64, extension: &str) -> String {
    let (y, mo, d, h, mi, s) = civil(seconds);
    format!("{stem} {y:04}-{mo:02}-{d:02} {h:02}-{mi:02}-{s:02}.{extension}")
}

/// Copy `project`'s file, as it is on disk now, into its versions folder, and
/// trim the folder to [`KEPT_VERSIONS`]. Nothing to keep when there is no file
/// yet. Returns the version written.
pub fn keep_version(project: &Path, seconds: u64) -> std::io::Result<Option<PathBuf>> {
    if !project.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(project)?;
    write_version(project, &bytes, seconds, "").map(Some)
}

/// Write `bytes` as a version of `project`, with `note` after the time.
pub fn write_version(
    project: &Path,
    bytes: &[u8],
    seconds: u64,
    note: &str,
) -> std::io::Result<PathBuf> {
    let folder = versions_folder(project);
    std::fs::create_dir_all(&folder)?;
    let stem = project.file_stem().map_or_else(
        || "project".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    let extension = project
        .extension()
        .map_or_else(|| "vproj".to_owned(), |s| s.to_string_lossy().into_owned());
    let mut name = version_file_name(&stem, seconds, &extension);
    if !note.is_empty() {
        name = name.replace(&format!(".{extension}"), &format!(" {note}.{extension}"));
    }
    let path = folder.join(name);
    std::fs::write(&path, bytes)?;
    prune(&folder, &stem, KEPT_VERSIONS)?;
    Ok(path)
}

/// Every kept version of `project`, newest first.
pub fn list_versions(project: &Path) -> Vec<Version> {
    let folder = versions_folder(project);
    let stem = project
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut versions: Vec<Version> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let when = name.strip_prefix(&format!("{stem} "))?;
            // "2026-09-14 01-32-05[ note]": the time with colons back in.
            let (date, rest) = when.split_at(when.len().min(10));
            let time = rest.trim_start().get(..8)?.replace('-', ":");
            let note = rest.trim_start().get(8..).unwrap_or("").trim();
            let label = if note.is_empty() {
                format!("{date} {time}")
            } else {
                format!("{date} {time} {note}")
            };
            Some(Version { path, label })
        })
        .collect();
    versions.sort_by(|a, b| b.path.cmp(&a.path));
    versions
}

fn prune(folder: &Path, stem: &str, keep: usize) -> std::io::Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{stem} ")))
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(keep);
    for old in files.into_iter().take(excess) {
        std::fs::remove_file(old)?;
    }
    Ok(())
}

/// Seconds since 1970, now.
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_named_sortably_in_utc() {
        assert_eq!(
            version_file_name("trip", 0, "vproj"),
            "trip 1970-01-01 00-00-00.vproj"
        );
        // 2026-09-14 01:32:05 UTC.
        assert_eq!(
            version_file_name("trip", 1_789_349_525, "vproj"),
            "trip 2026-09-14 01-32-05.vproj"
        );
        // A leap day.
        assert_eq!(
            version_file_name("x", 951_782_400, "vproj"),
            "x 2000-02-29 00-00-00.vproj"
        );
    }

    #[test]
    fn the_folder_sits_beside_the_project() {
        assert_eq!(
            versions_folder(Path::new("/work/trip.vproj")),
            PathBuf::from("/work/trip versions")
        );
    }
}
