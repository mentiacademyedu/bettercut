//! Reading and writing `.vproj` files (§37, §38.1).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ProjectError;
use crate::project::Project;

/// On-disk schema version. Bump whenever the serialized shape changes, and add
/// a migration step for the previous value.
pub const SCHEMA_VERSION: u32 = 1;

pub const PROJECT_EXTENSION: &str = "vproj";

/// The file envelope. The version lives outside the payload so a file can be
/// version-checked before anything inside it is parsed.
#[derive(Debug, Serialize, Deserialize)]
struct ProjectFile {
    version: u32,
    /// Informational — helps diagnose "this file was written by which build?".
    #[serde(default)]
    app_version: String,
    project: Project,
}

/// Write a project atomically (§38.1).
///
/// **Never truncate the project file in place.** A crash mid-write destroys the
/// project — this is the single most damaging failure a video editor can have,
/// because the user's work is not recoverable from anywhere else.
///
/// ```text
/// 1. write   project.vproj.tmp
/// 2. fsync   the file           (contents durable)
/// 3. fsync   the directory      (the rename itself durable, POSIX)
/// 4. rename  .tmp -> .vproj     (atomic on all target platforms)
/// ```
///
/// If any step fails, the original file is left exactly as it was.
pub fn save(project: &Project, path: impl AsRef<Path>) -> Result<(), ProjectError> {
    let path = path.as_ref();

    let file = ProjectFile {
        version: SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        project: project.clone(),
    };

    // Serialize before touching the filesystem: a serialization failure must not
    // leave a stray temp file behind.
    let json = serde_json::to_vec_pretty(&file).map_err(ProjectError::Serialize)?;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ProjectError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let tmp = temp_path_for(path);

    {
        let mut handle = fs::File::create(&tmp).map_err(|source| ProjectError::Io {
            path: tmp.clone(),
            source,
        })?;
        handle.write_all(&json).map_err(|source| ProjectError::Io {
            path: tmp.clone(),
            source,
        })?;
        // Without this, the rename can be durable while the contents are not,
        // and a power loss yields a zero-length project file.
        handle.sync_all().map_err(|source| ProjectError::Io {
            path: tmp.clone(),
            source,
        })?;
    }

    sync_parent_dir(path);

    fs::rename(&tmp, path).map_err(|source| {
        // Leave no debris if the rename fails.
        let _ = fs::remove_file(&tmp);
        ProjectError::Io {
            path: path.to_path_buf(),
            source,
        }
    })?;

    sync_parent_dir(path);

    tracing::info!(
        project = %project.id,
        clips = project.clip_count(),
        bytes = json.len(),
        "project saved"
    );

    Ok(())
}

/// Read a project, migrating it forward if it was written by an older build.
pub fn load(path: impl AsRef<Path>) -> Result<Project, ProjectError> {
    let path = path.as_ref();

    let bytes = fs::read(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ProjectError::NotFound(path.to_path_buf())
        } else {
            ProjectError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;

    // §37 makes the project file human-readable and therefore hand-editable.
    // Windows editors routinely add a UTF-8 BOM when saving, and serde_json
    // rejects it with "expected value at line 1 column 1" — an error that gives
    // a user no idea what is wrong with a file that looks perfectly fine.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);

    // Parse as generic JSON first so the version can be checked and migrations
    // applied before the payload has to match today's `Project` shape.
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(ProjectError::Deserialize)?;

    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(ProjectError::MissingVersion)? as u32;

    if version > SCHEMA_VERSION {
        return Err(ProjectError::UnsupportedVersion {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }

    if version < SCHEMA_VERSION {
        tracing::info!(
            from = version,
            to = SCHEMA_VERSION,
            "migrating project file"
        );
        value = migrate(value, version)?;
    }

    let file: ProjectFile = serde_json::from_value(value).map_err(ProjectError::Deserialize)?;

    tracing::info!(
        project = %file.project.id,
        clips = file.project.clip_count(),
        "project loaded"
    );

    Ok(file.project)
}

/// Migrate a parsed file from `from` up to [`SCHEMA_VERSION`].
///
/// There is nothing to do yet — version 1 is the first. The function exists now
/// so that adding version 2 is an edit here rather than a redesign, and so the
/// call site in `load` is already correct.
fn migrate(mut value: serde_json::Value, from: u32) -> Result<serde_json::Value, ProjectError> {
    // Version 1 is the first schema, so today there is nothing below it to
    // migrate from. When version 2 lands, this becomes:
    //
    //     if version == 1 { value = migrate_1_to_2(value)?; version = 2; }
    //
    // and `load` already routes here, so adding a step is a local edit.
    if from != SCHEMA_VERSION {
        return Err(ProjectError::NoMigrationPath { from });
    }

    // Stamp the migrated value so the envelope parses against today's version.
    if let Some(obj) = value.as_object_mut() {
        obj.insert("version".to_owned(), SCHEMA_VERSION.into());
    }
    Ok(value)
}

/// `foo.vproj` -> `foo.vproj.tmp`, kept in the same directory so the rename
/// stays within one filesystem and therefore stays atomic.
fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// fsync the containing directory so the rename survives a power loss.
///
/// Best-effort: Windows does not support opening a directory as a file, and
/// NTFS does not need it. A failure here is not a save failure.
fn sync_parent_dir(path: &Path) {
    let Some(parent) = path.parent() else { return };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaTime, TimelineTime};
    use bettercut_media::{MediaAsset, MediaKind};
    use bettercut_timeline::{SourceRange, VideoClip};

    fn sample_project() -> Project {
        let mut p = Project::new("Test Project");
        let media = MediaAsset::new(
            MediaKind::Video,
            "C:/media/clip.mp4",
            MediaTime::from_seconds(30),
        )
        .with_video(1920, 1080, bettercut_foundation::FrameRate::NTSC_29_97);
        let media_id = p.add_media(media);

        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).expect("valid");
        let clip = VideoClip::new(media_id, TimelineTime::ZERO, source).expect("valid");
        p.active_mut().expect("has sequence").video_tracks[0]
            .insert(clip)
            .expect("no overlap");
        p
    }

    #[test]
    fn save_then_load_round_trips_exactly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.vproj");

        let original = sample_project();
        save(&original, &path).expect("save");
        let loaded = load(&path).expect("load");

        assert_eq!(loaded, original);
        assert_eq!(loaded.clip_count(), 1);
    }

    #[test]
    fn saved_file_is_human_readable_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.vproj");
        save(&sample_project(), &path).expect("save");

        let text = fs::read_to_string(&path).expect("read");
        // §37: human-readable. Pretty-printed and version-tagged.
        assert!(text.contains("\"version\": 1"), "no version field: {text}");
        assert!(text.contains('\n'), "not pretty-printed");
        assert!(text.contains("Test Project"));
    }

    /// §38.1 — the critical property. An existing project must survive a failed
    /// save, and no temp file may be left behind after a successful one.
    #[test]
    fn saving_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.vproj");

        save(&sample_project(), &path).expect("save");
        assert!(
            !temp_path_for(&path).exists(),
            "temp file survived the save"
        );

        let entries: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["test.vproj".to_owned()]);
    }

    /// Overwriting must replace atomically, never truncate-then-write.
    #[test]
    fn overwriting_replaces_the_previous_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.vproj");

        let mut first = sample_project();
        first.name = "First".to_owned();
        save(&first, &path).expect("save");

        let mut second = sample_project();
        second.name = "Second".to_owned();
        save(&second, &path).expect("save");

        assert_eq!(load(&path).expect("load").name, "Second");
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/deeper/test.vproj");
        save(&sample_project(), &path).expect("save");
        assert!(path.exists());
    }

    #[test]
    fn loading_a_newer_schema_is_refused_not_guessed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("future.vproj");
        fs::write(&path, r#"{"version": 999, "project": {}}"#).expect("write");

        assert!(matches!(
            load(&path),
            Err(ProjectError::UnsupportedVersion {
                found: 999,
                supported: 1
            })
        ));
    }

    #[test]
    fn loading_a_file_without_a_version_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bad.vproj");
        fs::write(&path, r#"{"project": {}}"#).expect("write");
        assert!(matches!(load(&path), Err(ProjectError::MissingVersion)));
    }

    #[test]
    fn loading_a_missing_file_reports_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            load(dir.path().join("nope.vproj")),
            Err(ProjectError::NotFound(_))
        ));
    }

    /// A hand-edited project saved by a Windows editor picks up a BOM. It must
    /// still load — the alternative is "expected value at line 1 column 1" on a
    /// file that looks correct in every editor the user has.
    #[test]
    fn a_utf8_bom_does_not_break_loading() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bom.vproj");
        save(&sample_project(), &path).expect("save");

        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(&fs::read(&path).expect("read"));
        fs::write(&path, &bytes).expect("write with BOM");

        let loaded = load(&path).expect("BOM-prefixed project should load");
        assert_eq!(loaded.name, "Test Project");
    }

    #[test]
    fn loading_malformed_json_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bad.vproj");
        fs::write(&path, "{ this is not json").expect("write");
        assert!(matches!(load(&path), Err(ProjectError::Deserialize(_))));
    }

    #[test]
    fn temp_path_stays_in_the_same_directory() {
        let tmp = temp_path_for(Path::new("C:/projects/my.vproj"));
        assert_eq!(tmp, PathBuf::from("C:/projects/my.vproj.tmp"));
        assert_eq!(tmp.parent(), Path::new("C:/projects/my.vproj").parent());
    }

    /// The NTSC frame rate must survive a save/load cycle exactly — an f64
    /// round trip would turn 30000/1001 into something that no longer divides
    /// the timebase (§9).
    #[test]
    fn ntsc_frame_rate_survives_persistence_exactly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ntsc.vproj");

        let mut p = Project::new("ntsc");
        p.active_mut().expect("sequence").frame_rate = bettercut_foundation::FrameRate::NTSC_29_97;
        save(&p, &path).expect("save");

        let loaded = load(&path).expect("load");
        let rate = loaded.active().expect("sequence").frame_rate;
        assert_eq!(rate.as_rational().num(), 30_000);
        assert_eq!(rate.as_rational().den(), 1001);
        assert_eq!(
            bettercut_foundation::ticks_per_frame(rate),
            Some(32_032),
            "frame rate stopped dividing the timebase after a round trip"
        );
    }
}
