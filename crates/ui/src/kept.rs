//! The editor's own small files — settings, saved looks, title styles,
//! export presets, shortcuts — read and written in one place, so the web
//! build can keep them too: there, each is kept in the browser's local
//! storage under its path rather than on a disk the page does not have.

use std::path::Path;

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
