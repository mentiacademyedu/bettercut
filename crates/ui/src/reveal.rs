//! Show a file in the system's file browser, selected: where the footage
//! actually is, for copying it, checking its date, or finding the rest of
//! the card beside it.

use std::ffi::OsString;
use std::path::Path;

/// The program and arguments that reveal `path`, on this platform. Split
/// from running it so it can be checked without opening windows.
pub fn command_for(path: &Path) -> (&'static str, Vec<OsString>) {
    if cfg!(target_os = "windows") {
        // One argument: Explorer reads `/select,` and the path together.
        let mut arg = OsString::from("/select,");
        arg.push(path.as_os_str());
        ("explorer", vec![arg])
    } else if cfg!(target_os = "macos") {
        (
            "open",
            vec![OsString::from("-R"), path.as_os_str().to_owned()],
        )
    } else {
        // No portable "select" on Linux: the folder is the honest best.
        let folder = path
            .parent()
            .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf);
        ("xdg-open", vec![folder.into_os_string()])
    }
}

/// Open the file browser on `path`. The browser is left to itself: this
/// returns as soon as it has been asked.
pub fn show_in_folder(path: &Path) -> Result<(), String> {
    let (program, args) = command_for(path);
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the file browser ({program}): {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_names_the_file() {
        let (program, args) = command_for(Path::new("C:/media/clip.mp4"));
        assert!(!program.is_empty());
        let joined: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            joined.iter().any(|a| a.contains("media")),
            "the path is not in the arguments: {joined:?}"
        );
        if cfg!(target_os = "windows") {
            assert_eq!(program, "explorer");
            assert!(joined[0].starts_with("/select,"), "{joined:?}");
        }
    }
}
