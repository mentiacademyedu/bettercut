//! The editor's own small files — settings, saved looks, title styles,
//! export presets, shortcuts — read and written in one place, so the web
//! build can keep them too: there, each is kept in the browser's local
//! storage under its path rather than on a disk the page does not have.

use std::path::Path;

use bettercut_editor_core::project_format::Project;

/// Keep `project` in this browser now, beside every other project kept
/// there, with the files it uses. Whether it was: on the desktop, never —
/// a project there is kept by saving it.
#[cfg(target_arch = "wasm32")]
pub fn keep_project(project: &Project) -> bool {
    let json = match bettercut_editor_core::project_format::to_json(project) {
        Ok(json) => json,
        Err(err) => {
            tracing::error!(%err, "could not keep the project in this browser");
            return false;
        }
    };
    let paths = project
        .media
        .iter()
        .filter(|asset| asset.path.starts_with("/browser"))
        .map(|asset| asset.path.to_string_lossy().into_owned())
        .collect();
    bettercut_editor_core::media::browser::keep_project(&String::from_utf8_lossy(&json), paths);
    true
}

#[cfg(not(target_arch = "wasm32"))]
pub fn keep_project(_project: &Project) -> bool {
    false
}

/// The text kept at `file`.
#[cfg(not(target_arch = "wasm32"))]
pub fn read(file: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(file)
}

/// Keep `text` at `file`, making its folder when there is none.
#[cfg(not(target_arch = "wasm32"))]
pub fn write(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)
}

#[cfg(target_arch = "wasm32")]
pub fn read(file: &Path) -> std::io::Result<String> {
    bettercut_editor_core::media::browser::setting(&file.to_string_lossy())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
}

#[cfg(target_arch = "wasm32")]
pub fn write(file: &Path, text: &str) -> std::io::Result<()> {
    if bettercut_editor_core::media::browser::keep_setting(&file.to_string_lossy(), text) {
        Ok(())
    } else {
        Err(std::io::Error::other("this browser would not keep it"))
    }
}
