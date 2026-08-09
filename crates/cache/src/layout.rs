//! Where cached artefacts live (§19).
//!
//! ```text
//! .cache/
//! └── media/
//!     └── MEDIA_ID/
//!         ├── thumbnails/
//!         ├── waveform.bin
//!         └── proxy/
//! ```
//!
//! §19: *"Use hashed IDs so files with identical names do not conflict."* Two
//! `interview.mp4` files from different folders are different media; keying the
//! cache by filename would serve one's proxy for the other, and the user would
//! see the wrong footage with no clue why.
//!
//! The `MediaId` is already a UUID, so it is unique by construction — but it is
//! rendered here as a plain hex string rather than the hyphenated form, because
//! that is what stays valid on every filesystem we target.

use std::path::{Path, PathBuf};

use bettercut_foundation::MediaId;

pub const MEDIA_DIR: &str = "media";
pub const THUMBNAILS_DIR: &str = "thumbnails";
pub const PROXY_DIR: &str = "proxy";
pub const WAVEFORM_FILE: &str = "waveform.bin";

/// The root of one cache tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheLayout {
    root: PathBuf,
}

impl CacheLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The default location, under the OS cache directory.
    ///
    /// Not beside the project: several projects usually share media, and a
    /// per-project cache would regenerate the same proxy once per project.
    pub fn default_location() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(std::env::temp_dir);

        Self::new(base.join("bettercut").join("cache"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn media_root(&self) -> PathBuf {
        self.root.join(MEDIA_DIR)
    }

    /// This asset's directory. The id is rendered without hyphens so the name
    /// is valid everywhere and sorts predictably.
    pub fn media_dir(&self, media: MediaId) -> PathBuf {
        self.media_root().join(media.as_uuid().simple().to_string())
    }

    pub fn thumbnails_dir(&self, media: MediaId) -> PathBuf {
        self.media_dir(media).join(THUMBNAILS_DIR)
    }

    pub fn proxy_dir(&self, media: MediaId) -> PathBuf {
        self.media_dir(media).join(PROXY_DIR)
    }

    /// The proxy file for a given height, so switching quality does not discard
    /// the other one.
    pub fn proxy_file(&self, media: MediaId, height: u32) -> PathBuf {
        self.proxy_dir(media).join(format!("{height}p.mp4"))
    }

    /// Where a proxy is written while encoding.
    ///
    /// A separate name so an interrupted encode cannot be mistaken for a
    /// finished proxy — the same reasoning as §38.1's atomic project write. A
    /// half-written proxy that looks complete would play as corrupt footage.
    pub fn proxy_temp_file(&self, media: MediaId, height: u32) -> PathBuf {
        self.proxy_dir(media).join(format!("{height}p.partial.mp4"))
    }

    pub fn waveform_file(&self, media: MediaId) -> PathBuf {
        self.media_dir(media).join(WAVEFORM_FILE)
    }

    /// Create the directories an asset needs.
    pub fn prepare(&self, media: MediaId) -> std::io::Result<()> {
        std::fs::create_dir_all(self.proxy_dir(media))?;
        std::fs::create_dir_all(self.thumbnails_dir(media))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_asset_gets_its_own_directory() {
        let layout = CacheLayout::new("C:/cache");
        let (a, b) = (MediaId::new(), MediaId::new());

        assert_ne!(
            layout.media_dir(a),
            layout.media_dir(b),
            "two assets shared a cache directory"
        );
    }

    /// §19's actual worry: two files called `interview.mp4` from different
    /// folders must not collide.
    #[test]
    fn identical_file_names_do_not_collide() {
        let layout = CacheLayout::new("C:/cache");
        let first = MediaId::new();
        let second = MediaId::new();

        // Nothing about the path derives from the file name.
        let a = layout.proxy_file(first, 720);
        let b = layout.proxy_file(second, 720);
        assert_ne!(a, b);
    }

    #[test]
    fn directory_names_contain_no_hyphens_or_separators() {
        let layout = CacheLayout::new("C:/cache");
        let media = MediaId::new();
        let dir = layout.media_dir(media);
        let name = dir
            .file_name()
            .expect("a directory name")
            .to_string_lossy()
            .into_owned();

        assert_eq!(name.len(), 32, "expected a plain 32-char hex id: {name}");
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_proxy_heights_are_separate_files() {
        let layout = CacheLayout::new("C:/cache");
        let media = MediaId::new();
        assert_ne!(
            layout.proxy_file(media, 540),
            layout.proxy_file(media, 720),
            "switching quality would overwrite the other proxy"
        );
    }

    /// A partially written proxy must never look like a finished one.
    #[test]
    fn the_temporary_proxy_name_differs_from_the_final_one() {
        let layout = CacheLayout::new("C:/cache");
        let media = MediaId::new();
        assert_ne!(
            layout.proxy_temp_file(media, 720),
            layout.proxy_file(media, 720)
        );
    }

    #[test]
    fn preparing_creates_the_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = CacheLayout::new(dir.path());
        let media = MediaId::new();

        layout.prepare(media).expect("prepare");
        assert!(layout.proxy_dir(media).is_dir());
        assert!(layout.thumbnails_dir(media).is_dir());
    }

    #[test]
    fn the_default_location_is_absolute_and_namespaced() {
        let layout = CacheLayout::default_location();
        assert!(layout.root().is_absolute(), "{:?}", layout.root());
        assert!(
            layout.root().to_string_lossy().contains("bettercut"),
            "the cache root is not namespaced: {:?}",
            layout.root()
        );
    }
}
