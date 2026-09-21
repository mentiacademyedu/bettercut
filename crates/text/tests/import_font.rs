//! Importing your own font (`import_font_into`, `TextRenderer::add_font_file`).
//!
//! Uses a font Windows ships, and skips itself where that file is not there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{FontFamily, TextRenderer, TextStyle, import_font_into};

const SYSTEM_FONT: &str = "C:/Windows/Fonts/arial.ttf";

#[test]
fn an_imported_font_is_copied_in_and_draws_titles() {
    let source = std::path::Path::new(SYSTEM_FONT);
    if !source.exists() {
        eprintln!("no {SYSTEM_FONT}; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (copied, families) = import_font_into(source, dir.path()).unwrap();
    assert!(copied.starts_with(dir.path()) && copied.exists());
    assert!(families.iter().any(|f| f == "Arial"), "{families:?}");

    let mut renderer = TextRenderer::with_fonts(Vec::new());
    let style = TextStyle {
        family: FontFamily::Named("Arial".to_owned()),
        ..TextStyle::default()
    };
    let added = renderer.add_font_file(&copied).unwrap();
    assert_eq!(added, families);
    let bitmap = renderer.rasterize("Imported", &style).unwrap();
    assert!(bitmap.width > 50);
    assert!(renderer.families().iter().any(|f| f == "Arial"));
}

#[test]
fn a_file_that_is_not_a_font_is_refused_and_not_copied() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake.ttf");
    std::fs::write(&fake, b"not really a font").unwrap();
    let into = dir.path().join("fonts");
    assert!(import_font_into(&fake, &into).is_err());
    assert!(!into.join("fake.ttf").exists());
}
