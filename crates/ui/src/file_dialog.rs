//! Open, save and folder dialogs.
//!
//! On the desktop these are the system's own (`rfd`). A browser has no
//! dialog that hands back a path — a page gets files dropped on it or
//! chosen through an `<input>`, and their bytes, never where they live — so
//! in the web build every dialog is answered "cancelled", and files come in
//! by being dropped on the window instead. Picking media opens the
//! browser's own picker; what is chosen arrives a moment later, the same way
//! dropped files do (`crate::file_drop`).

#[cfg(not(target_arch = "wasm32"))]
pub use rfd::FileDialog;

#[cfg(target_arch = "wasm32")]
pub use web::FileDialog;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::PathBuf;

    /// The desktop dialog's builder. Picking several files opens the
    /// browser's picker and answers "cancelled" for now; everything else is
    /// cancelled.
    #[derive(Default)]
    pub struct FileDialog {
        extensions: Vec<String>,
    }

    impl FileDialog {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn add_filter(mut self, _name: impl Into<String>, extensions: &[impl ToString]) -> Self {
            self.extensions
                .extend(extensions.iter().map(ToString::to_string));
            self
        }
        pub fn set_title(self, _title: impl Into<String>) -> Self {
            self
        }
        pub fn set_file_name(self, _name: impl Into<String>) -> Self {
            self
        }
        pub fn set_directory<P: AsRef<std::path::Path>>(self, _path: P) -> Self {
            self
        }
        pub fn pick_file(self) -> Option<PathBuf> {
            None
        }
        pub fn pick_files(self) -> Option<Vec<PathBuf>> {
            let extensions: Vec<&str> = self.extensions.iter().map(String::as_str).collect();
            bettercut_editor_core::media::browser::pick_files(&extensions);
            None
        }
        pub fn pick_folder(self) -> Option<PathBuf> {
            None
        }
        pub fn save_file(self) -> Option<PathBuf> {
            None
        }
    }
}
