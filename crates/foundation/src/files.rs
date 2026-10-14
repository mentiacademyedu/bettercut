//! The editor's own small files — LUTs a person imports — read and written in
//! one place, so the web build can keep them too.
//!
//! On the desktop these are files like any other. A browser has no file
//! system: there, they are kept by the page (in its own storage, read into
//! memory before the app starts) under the same paths, through a store the
//! app hands this module when it starts. This crate stays free of the
//! browser: it only calls what it was given.

use std::path::Path;

/// The page's store: read a path's bytes, and keep bytes at a path.
#[cfg(target_arch = "wasm32")]
pub struct Store {
    pub read: fn(&str) -> Option<Vec<u8>>,
    pub write: fn(&str, &[u8]) -> bool,
}

#[cfg(target_arch = "wasm32")]
static STORE: std::sync::OnceLock<Store> = std::sync::OnceLock::new();

/// Use `store` for every file read and written here from now on. Once, as the
/// web build starts.
#[cfg(target_arch = "wasm32")]
pub fn use_store(store: Store) {
    let _ = STORE.set(store);
}

/// The bytes of the file at `path`.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    #[cfg(target_arch = "wasm32")]
    return STORE
        .get()
        .and_then(|store| (store.read)(&path.to_string_lossy()))
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound));
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::read(path)
}

/// How many bytes the file at `path` holds.
pub fn len(path: &Path) -> std::io::Result<u64> {
    #[cfg(target_arch = "wasm32")]
    return read(path).map(|bytes| bytes.len() as u64);
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::metadata(path).map(|m| m.len())
}

/// Keep `bytes` at `path`, making its folder when there is none.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(target_arch = "wasm32")]
    return match STORE.get() {
        Some(store) if (store.write)(&path.to_string_lossy(), bytes) => Ok(()),
        _ => Err(std::io::Error::other("this browser would not keep it")),
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, bytes)
    }
}
