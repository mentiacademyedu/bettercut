//! Words the user reads, checked for the one defect nothing else can see.
//!
//! A long message is split across source lines with a trailing `\`, which Rust
//! reads as "join these, dropping the indentation". Lose the backslash — an
//! editor, a formatter or a script rewriting the file — and the string still
//! compiles, still passes every test, and puts a run of spaces in the middle of
//! a sentence: a tooltip reading "Unlike                  Fill".
//!
//! It happened eleven times before this existed: tooltips, an empty-state
//! message, a warning about recovery data, two transition descriptions the
//! slideshow options display, and three test failure messages. The likely route
//! in was edits made by scripts passed through a shell, which strips a lone
//! backslash before the script ever sees it. No behavioural test notices,
//! because the text is never compared — only shown.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

/// A run this long inside a string is never prose.
///
/// Deliberately well above the widest intentional run in the interface: the
/// diagnostics panel aligns `gpu` and its value in a monospace column with
/// eight spaces, and a lost line continuation leaves fourteen or more.
const SUSPICIOUS_RUN: usize = 10;

fn workspace_root() -> PathBuf {
    // crates/ui → the workspace.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/ui sits two levels below the workspace")
        .to_path_buf()
}

fn rust_sources(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Build output and vendored code are not ours to read.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "target" || name == "vendor" || name.starts_with('.') {
                continue;
            }
            rust_sources(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            into.push(path);
        }
    }
}

/// The string literals on one line, as the text between each pair of quotes.
///
/// Deliberately simple: it does not parse Rust, so a quote inside a comment or a
/// char literal can confuse it. That errs towards finding too little, never
/// towards a false alarm on correct text, which is the right way round for a
/// check nobody wants to fight.
fn literals(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        found.push(&after[..close]);
        rest = &after[close + 1..];
    }
    found
}

/// A run of at least `run` spaces with words on both sides of it.
///
/// Interior only. A lost line continuation always leaves the gap *between* two
/// words, while leading spaces are how the diagnostics panel indents a value
/// under its label in a monospace column — deliberate, and flagging it would
/// teach people to ignore this check.
fn has_run_of_spaces(text: &str, run: usize) -> bool {
    let trimmed = text.trim_start();
    let mut count = 0;
    for ch in trimmed.chars() {
        if ch == ' ' {
            count += 1;
        } else {
            if count >= run {
                return true;
            }
            count = 0;
        }
    }
    false
}

#[test]
fn no_message_has_a_lost_line_break_in_the_middle_of_it() {
    let mut files = Vec::new();
    rust_sources(&workspace_root().join("crates"), &mut files);
    rust_sources(&workspace_root().join("apps"), &mut files);
    assert!(
        files.len() > 50,
        "found only {} source files, so the scan is looking in the wrong place",
        files.len()
    );

    let mut broken = Vec::new();
    for file in &files {
        // This file names the defect in its own documentation.
        if file.ends_with("interface_text.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            if literals(line)
                .iter()
                .any(|literal| has_run_of_spaces(literal, SUSPICIOUS_RUN))
            {
                broken.push(format!(
                    "{}:{}: {}",
                    file.strip_prefix(workspace_root())
                        .unwrap_or(file)
                        .display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        broken.is_empty(),
        "these strings have a run of spaces where a `\\` line continuation was lost:\n  {}",
        broken.join("\n  ")
    );
}

/// The check has to be able to see the defect, or its silence means nothing.
#[test]
fn the_check_recognises_a_lost_line_break() {
    let damaged = r#"text("Unlike                  Fill, nothing is enlarged")"#;
    assert!(
        literals(damaged)
            .iter()
            .any(|literal| has_run_of_spaces(literal, SUSPICIOUS_RUN))
    );

    // And leaves a deliberate monospace column alone — both the label-to-value
    // gap and a value indented under its label.
    for aligned in [
        r#"ui.monospace("gpu        unavailable")"#,
        r#"ui.monospace(format!("           {} · {}", kind, backend))"#,
    ] {
        assert!(
            !literals(aligned)
                .iter()
                .any(|literal| has_run_of_spaces(literal, SUSPICIOUS_RUN)),
            "a deliberate alignment was flagged: {aligned}"
        );
    }
}
