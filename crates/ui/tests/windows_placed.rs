//! Every window says where it first appears. One that does not opens at the
//! very top-left, over Play, New and Open — seven did, until a screenshot of
//! each window showed it. Read from the source, so a new window is held to it
//! the day it is written.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn every_window_has_a_first_place() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut unplaced = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains("egui::Window::new(") || line.trim_start().starts_with("//") {
                continue;
            }
            // The builder chain that follows: placed by the helper, or by an
            // anchor or a position of its own.
            let chain = lines[i..lines.len().min(i + 14)].join("\n");
            let placed = line.contains("theme::placed(")
                || chain.contains(".anchor(")
                || chain.contains(".default_pos(")
                || chain.contains(".fixed_pos(")
                || chain.contains(".current_pos(");
            if !placed {
                unplaced.push(format!(
                    "{}:{}",
                    path.file_name().unwrap().to_string_lossy(),
                    i + 1
                ));
            }
        }
    }
    assert!(
        unplaced.is_empty(),
        "these windows would open over the toolbar; wrap them in theme::placed: {unplaced:?}"
    );
}
