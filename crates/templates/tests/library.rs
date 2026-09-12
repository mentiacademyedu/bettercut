//! Templates on disk: the user's folder, and adding a file to it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bettercut_templates::library::{InstallError, install, load_dir};

fn template(id: &str, name: &str) -> String {
    format!(
        r#"{{ "schema_version": 1, "id": "{id}", "name": "{name}", "category": "Mine",
             "duration": 2,
             "slots": [ {{ "id": "shot", "type": "video" }} ],
             "elements": [ {{ "type": "clip", "slot": "shot", "start": 0, "duration": 2 }} ] }}"#
    )
}

fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) {
    std::fs::write(dir.join(name), contents).unwrap();
}

#[test]
fn a_folder_that_does_not_exist_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let library = load_dir(&dir.path().join("never-made"));
    assert!(library.templates.is_empty());
    assert!(library.rejected.is_empty());
}

#[test]
fn one_bad_file_does_not_hide_the_good_ones() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "b-good.json", template("good", "Good"));
    write(dir.path(), "a-broken.json", "{ not json");
    write(dir.path(), "c-huge.json", " ".repeat(2 * 1024 * 1024));
    write(dir.path(), "d-binary.json", [0xff, 0xfe, 0x00]);
    write(dir.path(), "notes.txt", "not a template, not looked at");

    let library = load_dir(dir.path());

    let ids: Vec<&str> = library.templates.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["good"]);
    let rejected: Vec<String> = library
        .rejected
        .iter()
        .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(rejected, ["a-broken.json", "c-huge.json", "d-binary.json"]);
    assert!(
        library.rejected[1].1[0].message.contains("limited to"),
        "the oversized file is refused by size, before it is read"
    );
}

#[test]
fn installing_copies_the_file_under_its_id() {
    let source = tempfile::tempdir().unwrap();
    let library = tempfile::tempdir().unwrap();
    write(
        source.path(),
        "Downloaded (1).json",
        template("my-intro", "My Intro"),
    );

    let installed = install(&source.path().join("Downloaded (1).json"), library.path()).unwrap();
    assert_eq!(installed.name, "My Intro");
    assert!(library.path().join("my-intro.json").is_file());

    // A newer version of the same template replaces it rather than doubling.
    write(
        source.path(),
        "v2.json",
        template("my-intro", "My Intro v2"),
    );
    install(&source.path().join("v2.json"), library.path()).unwrap();
    let names: Vec<String> = load_dir(library.path())
        .templates
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["My Intro v2"]);
}

#[test]
fn an_invalid_file_is_not_installed() {
    let source = tempfile::tempdir().unwrap();
    let library = tempfile::tempdir().unwrap();
    write(
        source.path(),
        "bad.json",
        template("bad", "Bad").replace("\"duration\": 2,", "\"duration\": -2,"),
    );

    let err = install(&source.path().join("bad.json"), library.path()).unwrap_err();
    assert!(matches!(err, InstallError::Invalid(_)), "{err}");
    assert!(err.to_string().contains("duration"), "{err}");
    assert_eq!(std::fs::read_dir(library.path()).unwrap().count(), 0);
}

#[test]
fn a_starters_id_cannot_be_taken() {
    let source = tempfile::tempdir().unwrap();
    let library = tempfile::tempdir().unwrap();
    write(source.path(), "fake.json", template("meme", "Not The Meme"));

    let err = install(&source.path().join("fake.json"), library.path()).unwrap_err();
    assert!(matches!(err, InstallError::ClashesWithStarter(_)), "{err}");
}
