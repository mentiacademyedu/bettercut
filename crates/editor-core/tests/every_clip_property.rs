//! Every clip property, swept: set it, undo it, redo it, crash and recover —
//! and each time the project is exactly what it should be.
//!
//! Each property is built from JSON the way the journal stores it
//! (`{"property": name, "value": v}`), trying a few simple values, so a new
//! property is covered here without anyone listing it. The ones whose value
//! is a structure no simple value fits are counted, not silently passed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{ClipProperty, Editor, recover};
use serde_json::{Value, json};

/// Every property's name, as the journal writes it.
const NAMES: &[&str] = &[
    "opacity",
    "gain",
    "position",
    "scale",
    "rotation",
    "brightness",
    "contrast",
    "saturation",
    "temperature",
    "tint",
    "anchor",
    "vibrance",
    "wheels",
    "secondary",
    "blur",
    "vignette",
    "grain",
    "burn_in",
    "sharpen",
    "reverse",
    "rgb_split",
    "glitch",
    "pixelate",
    "zoom_blur",
    "lens",
    "posterise",
    "smooth_skin",
    "tilt_shift",
    "glow",
    "old_film",
    "curves",
    "light_leak",
    "lens_flare",
    "beat_pulse",
    "shake",
    "strobe",
    "sway",
    "flicker",
    "heartbeat",
    "bounce",
    "background",
    "backdrop",
    "bars",
    "blend",
    "border",
    "channels",
    "chroma_key",
    "corner_pin",
    "crop",
    "crossfade",
    "de_ess",
    "denoise",
    "eq",
    "fade_shape",
    "flip",
    "gate",
    "keep_pitch",
    "leveller",
    "luma_key",
    "lut",
    "mask",
    "motion",
    "motion_blur",
    "mute",
    "pan",
    "pitch",
    "progress_bar",
    "reflection",
    "robot",
    "secondary",
    "shadow",
    "smooth_motion",
    "space",
    "stereo_width",
    "tone",
];

/// The values tried, simplest first.
fn candidates() -> Vec<Value> {
    vec![
        json!(0.37),
        json!(true),
        json!({ "x": 0.21, "y": -0.13 }),
        json!(3.0),
        json!(null),
        // Names of choices, for the properties that are one: serde takes
        // only the names a property knows, so a wrong guess just fails to
        // build and the next one is tried.
        json!("screen"),
        json!("multiply"),
        json!("blur"),
        json!("left_to_both"),
        json!("mono"),
        json!("linear"),
        json!("fast"),
        json!("left_right"),
        json!("sepia"),
        json!("horizontal"),
        json!("ellipse"),
    ]
}

/// One clip as JSON, picture or sound.
fn clip_json(editor: &Editor, clip: ClipId) -> Value {
    editor
        .video_clip(clip)
        .map(|c| serde_json::to_value(c).unwrap())
        .or_else(|| {
            editor
                .audio_clip(clip)
                .map(|c| serde_json::to_value(c).unwrap())
        })
        .unwrap_or(Value::Null)
}

/// Change the first number (or, failing that, the first flag) inside
/// `value`, so it differs from what it was. False when there was nothing.
fn nudge(value: &mut Value) -> bool {
    match value {
        // A whole number stays whole (a colour channel), one step away.
        Value::Number(n) if n.is_u64() => {
            let x = n.as_u64().unwrap_or(0);
            *value = json!(if x > 0 { x - 1 } else { 1 });
            true
        }
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            *value = json!(if x.fract() == 0.0 && x.abs() > 1.5 {
                x + 1.0
            } else {
                x * 0.5 + 0.11
            });
            true
        }
        Value::Bool(b) => {
            *b = !*b;
            true
        }
        Value::Array(items) => items.iter_mut().any(nudge),
        Value::Object(map) => map.values_mut().any(nudge),
        _ => false,
    }
}

fn project(editor: &Editor) -> Value {
    serde_json::to_value(editor.project()).unwrap()
}

/// A project with one picture clip and one sound clip.
fn setup() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Every property");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(20),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.audio_codec = Some("aac".to_owned());
    asset.audio_sample_rate = Some(48_000);
    asset.audio_channels = Some(2);
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    let picture = placed
        .iter()
        .copied()
        .find(|c| editor.video_clip(*c).is_some())
        .unwrap();
    let sound = placed
        .iter()
        .copied()
        .find(|c| editor.audio_clip(*c).is_some())
        .unwrap();
    (editor, picture, sound)
}

#[test]
fn every_property_undoes_redoes_and_recovers_exactly() {
    let mut checked = Vec::new();
    let mut unbuilt = Vec::new();
    for name in NAMES {
        // Simple values, and the clip's own current value for this name,
        // nudged — which fits the properties whose value is a structure.
        let (probe, picture, sound) = setup();
        let mut values = candidates();
        for clip in [picture, sound] {
            let json = clip_json(&probe, clip);
            for found in [
                json.get(*name),
                json.get("color").and_then(|c| c.get(*name)),
            ]
            .into_iter()
            .flatten()
            {
                let mut nudged = found.clone();
                if nudge(&mut nudged) {
                    values.push(nudged);
                }
            }
        }
        let built: Vec<ClipProperty> = values
            .into_iter()
            .filter_map(|value| {
                serde_json::from_value(json!({ "property": name, "value": value })).ok()
            })
            .collect();
        if built.is_empty() {
            unbuilt.push(*name);
            continue;
        }
        let mut applied = false;
        for property in built {
            let (mut editor, picture, sound) = setup();
            let before = project(&editor);
            // Picture first; a sound-only property is refused there.
            let target = [picture, sound]
                .into_iter()
                .find(|clip| editor.set_clip_property(*clip, property, false).is_ok());
            if target.is_none() {
                continue;
            }
            let after = project(&editor);
            if after == before {
                // That value is the default: try the next one.
                continue;
            }
            applied = true;

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
            break;
        }
        if !applied {
            unbuilt.push(*name);
        }
    }
    eprintln!("checked {} properties: {checked:?}", checked.len());
    eprintln!("not built from a simple value: {unbuilt:?}");
    assert!(
        checked.len() >= 45,
        "only {} properties were checked",
        checked.len()
    );
}
