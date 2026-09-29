//! Templates on disk: the user's own folder, and adding a file to it
//! (Milestone 11).
//!
//! Everything read here is untrusted (§64) and goes through [`crate::parse`].
//! A folder is read file by file, and one bad file is reported and skipped —
//! never allowed to hide the good ones beside it.

use std::path::{Path, PathBuf};

use crate::validate::{MAX_FILE_BYTES, Problem};
use crate::{Template, parse, starters};

/// Files read from one folder, at most. A folder of ten thousand files is a
/// mistake, and reading all of them would stall opening the window.
pub const MAX_FILES: usize = 500;

/// What a folder held.
#[derive(Debug, Default)]
pub struct Library {
    /// In file-name order.
    pub templates: Vec<Template>,
    /// Files that are not valid templates, and why.
    pub rejected: Vec<(PathBuf, Vec<Problem>)>,
}

/// Where the user's templates live: beside the editor's other per-user data,
/// in a folder of their own.
pub fn user_dir() -> PathBuf {
    bettercut_foundation::places::data_home().join("templates")
}

/// Read one template file, size-capped before it is read (§64).
pub fn load_file(path: &Path) -> Result<Template, Vec<Problem>> {
    parse(&read_capped(path)?)
}

/// A file's text, refused before reading if it is over the size limit.
fn read_capped(path: &Path) -> Result<String, Vec<Problem>> {
    let refuse = |message: String| {
        vec![Problem {
            at: String::new(),
            message,
        }]
    };
    let size = std::fs::metadata(path)
        .map_err(|err| refuse(format!("could not read the file: {err}")))?
        .len();
    if size > MAX_FILE_BYTES as u64 {
        return Err(refuse(format!(
            "the file is {} KB; templates are limited to {} KB",
            size / 1024,
            MAX_FILE_BYTES / 1024
        )));
    }
    let bytes =
        std::fs::read(path).map_err(|err| refuse(format!("could not read the file: {err}")))?;
    String::from_utf8(bytes)
        .map_err(|_| refuse("the file is not text (templates are UTF-8 JSON)".to_owned()))
}

/// Every `.json` template in `dir`. A folder that does not exist is empty,
/// not an error: nobody has added a template yet.
pub fn load_dir(dir: &Path) -> Library {
    let mut library = Library::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return library;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        })
        .take(MAX_FILES)
        .collect();
    paths.sort();

    for path in paths {
        match load_file(&path) {
            Ok(template) => library.templates.push(template),
            Err(problems) => library.rejected.push((path, problems)),
        }
    }
    library
}

/// Why a file could not be added.
#[derive(Debug)]
pub enum InstallError {
    Invalid(Vec<Problem>),
    /// Built-in templates cannot be replaced: a starter that changed on one
    /// machine and not another is exactly the inconsistency §26 warns about.
    ClashesWithStarter(String),
    Io(std::io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(problems) => match problems.as_slice() {
                [] => write!(f, "not a valid template"),
                [only] => write!(f, "not a valid template: {only}"),
                [first, rest @ ..] => write!(
                    f,
                    "not a valid template: {first} (and {} more problem(s))",
                    rest.len()
                ),
            },
            Self::ClashesWithStarter(id) => write!(
                f,
                "a built-in template is already called `{id}`; give this one another id"
            ),
            Self::Io(err) => write!(f, "could not save the template: {err}"),
        }
    }
}

/// Check a template file and copy it into `dir` as `<id>.json`.
///
/// Checked *before* it is copied, so the folder only ever holds templates that
/// load. Named by its id rather than its original file name: the id is a
/// strict alphabet (see `validate::is_identifier`), so the name can never be a
/// path out of the folder, and adding a newer version of the same template
/// replaces the old one rather than listing both.
pub fn install(file: &Path, dir: &Path) -> Result<Template, InstallError> {
    // Read once: the text that is checked is the text that is written. Reading
    // the file again to copy it would let a file changed in between arrive
    // unchecked.
    let text = read_capped(file).map_err(InstallError::Invalid)?;
    let template = parse(&text).map_err(InstallError::Invalid)?;
    if starters().iter().any(|s| s.id == template.id) {
        return Err(InstallError::ClashesWithStarter(template.id));
    }
    std::fs::create_dir_all(dir).map_err(InstallError::Io)?;
    // The author's own text rather than a re-serialization, so their layout
    // survives.
    std::fs::write(dir.join(format!("{}.json", template.id)), text).map_err(InstallError::Io)?;
    Ok(template)
}
