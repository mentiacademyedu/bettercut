//! Every title property, swept like the clip ones (`every_clip_property.rs`):
//! set, undo, redo, crash and recover, each time exactly.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::ClipId;
use bettercut_editor_core::{Editor, TextProperty, recover};
use serde_json::{Value, json};

const NAMES: &[&str] = &[
    "content",
    "style",
    "position",
    "scale",
    "rotation",
    "opacity",
    "animation",
    "motion_blur",
    "shape",
    "counter",
    "highlight",
];

fn project(editor: &Editor) -> Value {
    serde_json::to_value(editor.project()).unwrap()
}

fn setup() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Every title property");
    let title = editor.add_text("A title").unwrap();
    (editor, title)
}

/// Change the first number inside `value` — or, with none, the first flag,
/// or the first string. Numbers first: a string is often a choice's name,
/// and a changed name is no longer a choice.
fn nudge(value: &mut Value) -> bool {
    nudge_kind(value, 0) || nudge_kind(value, 1) || nudge_kind(value, 2)
}

fn nudge_kind(value: &mut Value, kind: u8) -> bool {
    match value {
        Value::Number(_) if kind != 0 => false,
        Value::Bool(_) if kind != 1 => false,
        Value::String(_) if kind != 2 => false,
        Value::Array(items) => items.iter_mut().any(|v| nudge_kind(v, kind)),
        Value::Object(map) => map.values_mut().any(|v| nudge_kind(v, kind)),
        _ => nudge_leaf(value),
    }
}

fn nudge_leaf(value: &mut Value) -> bool {
    match value {
        // A whole number stays whole (a colour channel, a count), one step
        // away; a fraction moves further.
        Value::Number(n) if n.is_u64() => {
            let x = n.as_u64().unwrap_or(0);
            *value = json!(if x > 0 { x - 1 } else { 1 });
            true
        }
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            *value = json!(x * 0.5 + 7.0);
            true
        }
        Value::Bool(b) => {
            *b = !*b;
            true
        }
        Value::String(s) => {
            s.push('!');
            true
        }
        _ => false,
    }
}

#[test]
fn every_title_property_undoes_redoes_and_recovers_exactly() {
    let mut checked = Vec::new();
    let mut unbuilt = Vec::new();
    for name in NAMES {
        let (probe, title) = setup();
        let current = serde_json::to_value(probe.text_clip(title).unwrap()).unwrap();
        let mut values = vec![
            json!("Other words"),
            json!(0.37),
            json!(true),
            json!({ "x": 0.21, "y": -0.13 }),
            json!({ "r": 255, "g": 200, "b": 0, "a": 255 }),
        ];
        // The title's own value under the same name — or its style, for
        // "style" — nudged.
        for field in [*name, "text"] {
            if let Some(found) = current.get(field) {
                let mut nudged = found.clone();
                if nudge(&mut nudged) {
                    values.push(nudged);
                }
            }
        }
        let built: Vec<TextProperty> = values
            .into_iter()
            .filter_map(|value| {
                serde_json::from_value(json!({ "property": name, "value": value })).ok()
            })
            .collect();
        let mut done = false;
        for property in built {
            let (mut editor, title) = setup();
            let before = project(&editor);
            if editor.set_text_property(title, property, false).is_err() {
                continue;
            }
            let after = project(&editor);
            if after == before {
                continue;
            }
            editor.undo().unwrap();
            assert_eq!(project(&editor), before, "{name}: undo did not restore");
            editor.redo().unwrap();
            assert_eq!(
                project(&editor),
                after,
                "{name}: redo did not bring it back"
            );
            let back = recover(editor.recovery_paths().clone()).expect("recoverable");
            assert_eq!(back.failed, 0, "{name}: did not replay");
            assert_eq!(
                serde_json::to_value(&back.project).unwrap(),
                after,
                "{name}: crash recovery differs"
            );
            back.discard();
            checked.push(*name);
            done = true;
            break;
        }
        if !done {
            unbuilt.push(*name);
        }
    }
    eprintln!("checked {} title properties: {checked:?}", checked.len());
    eprintln!("not built from a simple value: {unbuilt:?}");
    assert!(checked.len() >= 8, "only {} were checked", checked.len());
}
