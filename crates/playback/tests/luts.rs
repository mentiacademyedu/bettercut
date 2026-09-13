//! Loading colour lookup tables for drawing (`bettercut_playback::load_luts`),
//! the one loader the preview and the export share.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::path::Path;

use bettercut_foundation::LutId;
use bettercut_project_format::{LutAsset, Project};
use bettercut_timeline::CubeLut;

fn write_cube(path: &Path) {
    let mut text = String::from("LUT_3D_SIZE 2\n");
    for [r, g, b] in &CubeLut::identity(2).table {
        text.push_str(&format!("{r} {g} {b}\n"));
    }
    std::fs::write(path, text).unwrap();
}

/// Every readable table once; a missing or broken file skipped without
/// complaint but remembered, so it is not re-read on every frame; and a table
/// added to the project later is picked up on the next call.
#[test]
fn tables_are_loaded_once_and_bad_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let here = dir.path().join("here.cube");
    let broken = dir.path().join("broken.cube");
    write_cube(&here);
    std::fs::write(&broken, "not a table").unwrap();

    let mut project = Project::new("LUTs");
    let asset = |path: &Path| LutAsset {
        id: LutId::new(),
        name: "x".to_owned(),
        path: path.to_path_buf(),
    };
    let (good, bad, gone) = (
        asset(&here),
        asset(&broken),
        asset(&dir.path().join("gone.cube")),
    );
    project.luts = vec![good.clone(), bad.clone(), gone.clone()];

    let mut tried = HashSet::new();
    let mut loaded = Vec::new();
    bettercut_playback::load_luts(&project, &mut tried, |id, lut| loaded.push((id, lut.size)));
    assert_eq!(loaded, vec![(good.id, 2)]);
    assert!(tried.contains(&bad.id) && tried.contains(&gone.id));

    bettercut_playback::load_luts(&project, &mut tried, |id, _| loaded.push((id, 0)));
    assert_eq!(loaded.len(), 1, "a table was read twice");

    let later = dir.path().join("later.cube");
    write_cube(&later);
    let added = asset(&later);
    project.luts.push(added.clone());
    bettercut_playback::load_luts(&project, &mut tried, |id, lut| loaded.push((id, lut.size)));
    assert_eq!(
        loaded.last(),
        Some(&(added.id, 2)),
        "a new table was not picked up"
    );
}
