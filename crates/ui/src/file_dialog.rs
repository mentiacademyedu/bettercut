//! Open, save and folder dialogs.
//!
//! On the desktop these are the system's own (`rfd`). A browser has no
//! dialog that hands back a path — a page gets files dropped on it or
//! chosen through an `<input>`, and their bytes, never where they live — so
//! in the web build every dialog is answered "cancelled", and files come in
//! by being dropped on the window instead.

#[cfg(not(target_arch = "wasm32"))]
pub use rfd::FileDialog;

#[cfg(target_arch = "wasm32")]
pub use web::FileDialog;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::PathBuf;

    /// The desktop dialog's builder, every one of which is cancelled.
    #[derive(Default)]
    pub struct FileDialog;

    impl FileDialog {
        pub fn new() -> Self {
            Self
        }
        pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self {
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
