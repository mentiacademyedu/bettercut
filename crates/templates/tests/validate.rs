//! The template validator (§64, §65): every rule, and no input that panics.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{Rational, TimelineTime};
use bettercut_templates::{Element, SlotKind, is_bundle_relative, parse};
use bettercut_timeline::TransitionKind;
use serde_json::{Value, json};

fn secs(s: i64) -> TimelineTime {
    TimelineTime::from_ticks(s * 960_000)
}

/// A template that uses every feature once, and is valid.
fn good() -> Value {
    json!({
        "schema_version": 1,
        "id": "product-intro",
        "name": "Product intro",
        "category": "Promo",
        "duration": 6.0,
        "description": "Two shots and a headline.",
        "slots": [
            { "id": "shot_a", "type": "video", "label": "First shot" },
            { "id": "logo", "type": "logo", "label": "Your logo" },
            { "id": "headline", "type": "text", "label": "Headline", "default_text": "New in store" },
            { "id": "music", "type": "audio" }
        ],
        "elements": [
            { "type": "clip", "slot": "shot_a", "start": 0.0, "duration": 4.0,
              "transform": { "scale": 1.2 }, "speed": 0.5,
              "transition_out": { "kind": "fade_through_black", "duration": 0.5 } },
            { "type": "clip", "slot": "logo", "start": 4.0, "duration": 2.0, "track": 1,
              "transform": { "position": [0.3, -0.2], "rotation": 15.0 }, "opacity": 0.8 },
            { "type": "text", "slot": "headline", "start": 0.5, "duration": 3.0 },
            { "type": "text", "text": "Available now", "start": 4.0, "duration": 2.0,
              "style": { "size": 48.0, "color": { "r": 255, "g": 255, "b": 255, "a": 255 }, "line_height": 1.2 } },
            { "type": "audio", "slot": "music", "start": 0.0, "duration": 6.0, "volume": 0.7 }
        ]
    })
}

fn problems_of(value: &Value) -> Vec<String> {
    match parse(&value.to_string()) {
        Ok(_) => Vec::new(),
        Err(problems) => problems.iter().map(ToString::to_string).collect(),
    }
}

/// Assert the template is refused, and that some problem mentions `needle`.
#[track_caller]
fn refused(value: &Value, needle: &str) {
    let problems = problems_of(value);
    assert!(
        problems.iter().any(|p| p.contains(needle)),
        "expected a problem mentioning {needle:?}, got {problems:?}"
    );
}

#[test]
fn a_good_template_is_accepted_and_converted_to_ticks() {
    let template = parse(&good().to_string()).expect("the fixture is valid");

    assert_eq!(template.id, "product-intro");
    assert_eq!(template.duration, secs(6));
    assert_eq!(template.slots.len(), 4);
    assert_eq!(template.slot("logo").unwrap().kind, SlotKind::Logo);
    // An empty label falls back to the id, so the interface never shows a blank.
    assert_eq!(template.slot("music").unwrap().label, "music");

    let Element::Clip {
        start,
        duration,
        speed,
        transform,
        transition_out,
        ..
    } = &template.elements[0]
    else {
        panic!("first element is a clip");
    };
    assert_eq!(*start, TimelineTime::ZERO);
    assert_eq!(*duration, secs(4));
    // Exact, not 0.5 as a float (§74).
    assert_eq!(*speed, Rational::new(1, 2).unwrap());
    assert_eq!(transform.scale.x, 1.2);
    assert_eq!(transform.scale.y, 1.2, "scale is uniform");
    assert_eq!(
        *transition_out,
        Some((
            TransitionKind::FadeThroughBlack,
            TimelineTime::from_ticks(480_000)
        ))
    );

    // A text slot's element starts out saying the slot's default.
    let Element::Text { text, .. } = &template.elements[2] else {
        panic!("third element is text");
    };
    assert_eq!(text, "New in store");
}

#[test]
fn every_problem_is_reported_not_just_the_first() {
    let mut value = good();
    value["id"] = json!("Not An Id");
    value["name"] = json!("");
    value["elements"][0]["duration"] = json!(-1.0);
    value["elements"][4]["volume"] = json!(9.0);

    let problems = problems_of(&value);
    assert!(problems.len() >= 4, "{problems:?}");
    assert!(problems.iter().any(|p| p.starts_with("id")));
    assert!(problems.iter().any(|p| p.starts_with("name")));
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("elements[0].duration"))
    );
    assert!(problems.iter().any(|p| p.starts_with("elements[4].volume")));
}

#[test]
fn a_newer_format_is_named_as_such() {
    let mut value = good();
    value["schema_version"] = json!(2);
    // Even with a field this version has never heard of — the version is the
    // useful thing to say, not the field.
    value["elements"][0]["wobble"] = json!(true);
    refused(&value, "made for template format 2");

    let mut value = good();
    value["schema_version"] = json!(0);
    refused(&value, "schema_version");
}

#[test]
fn unknown_fields_and_element_types_are_refused() {
    let mut value = good();
    value["elements"][0]["wobble"] = json!(true);
    refused(&value, "wobble");

    let mut value = good();
    value["elements"][1]["type"] = json!("particle_storm");
    refused(&value, "particle_storm");

    let mut value = good();
    value["slots"][0]["type"] = json!("hologram");
    refused(&value, "hologram");
}

#[test]
fn ids_are_a_strict_alphabet() {
    for bad in [
        "",
        "Upper",
        "has space",
        "../escape",
        "a/b",
        "x".repeat(65).as_str(),
    ] {
        let mut value = good();
        value["id"] = json!(bad);
        refused(&value, "id");
    }
    let mut value = good();
    value["slots"][0]["id"] = json!("..");
    refused(&value, "slots[0].id");
}

#[test]
fn slot_ids_are_unique() {
    let mut value = good();
    value["slots"][1]["id"] = json!("shot_a");
    refused(&value, "used twice");
}

#[test]
fn elements_must_name_a_slot_that_exists_and_fits() {
    let mut value = good();
    value["elements"][0]["slot"] = json!("nowhere");
    refused(&value, "no slot called `nowhere`");

    // Music on a video track.
    let mut value = good();
    value["elements"][0]["slot"] = json!("music");
    refused(&value, "is a slot for audio");

    // A picture where only words go.
    let mut value = good();
    value["elements"][2]["slot"] = json!("shot_a");
    refused(&value, "is a slot for video");

    // A text element with neither a slot nor words.
    let mut value = good();
    value["elements"][3].as_object_mut().unwrap().remove("text");
    refused(&value, "needs a `slot` or fixed `text`");
}

#[test]
fn every_slot_must_be_used() {
    let mut value = good();
    value["slots"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "orphan", "type": "image" }));
    refused(&value, "no element uses this slot");
}

#[test]
fn only_text_slots_have_default_text() {
    let mut value = good();
    value["slots"][0]["default_text"] = json!("hello");
    refused(&value, "only a text slot has default text");
}

#[test]
fn times_are_checked() {
    for (field, bad, needle) in [
        ("start", json!(-0.5), "cannot be negative"),
        ("duration", json!(0.0), "more than zero"),
        ("duration", json!(1e12), "limit"),
    ] {
        let mut value = good();
        value["elements"][0][field] = bad;
        refused(&value, needle);
    }

    let mut value = good();
    value["elements"][0]["start"] = json!(3.0); // 3 + 4 > 6
    refused(&value, "runs past the end of the template");

    let mut value = good();
    value["duration"] = json!(0.0);
    let problems = problems_of(&value);
    assert!(problems.iter().any(|p| p.starts_with("duration")));
    assert!(
        !problems.iter().any(|p| p.contains("runs past")),
        "a bad template duration is reported once, not once per element: {problems:?}"
    );
}

#[test]
fn an_element_ending_exactly_at_the_end_is_fine() {
    // 0.1 is not exact in binary; 5.9 + 0.1 must still fit a 6 s template.
    let mut value = good();
    value["elements"][4]["start"] = json!(5.9);
    value["elements"][4]["duration"] = json!(0.1);
    assert_eq!(problems_of(&value), Vec::<String>::new());
}

#[test]
fn numbers_outside_the_editors_own_ranges_are_rejected_not_clamped() {
    let cases = [
        ("/elements/0/opacity", json!(1.5), "opacity"),
        ("/elements/0/speed", json!(50.0), "speed"),
        ("/elements/0/speed", json!(0.0), "speed"),
        ("/elements/0/transform/scale", json!(0.0), "scale"),
        ("/elements/0/transform/scale", json!(11.0), "scale"),
        (
            "/elements/1/transform/position",
            json!([0.0, 11.0]),
            "position[1]",
        ),
        ("/elements/1/transform/rotation", json!(-1e9), "rotation"),
        ("/elements/1/track", json!(8), "track"),
        ("/elements/4/volume", json!(-0.1), "volume"),
        (
            "/elements/0/transition_out/duration",
            json!(0.001),
            "shortest transition",
        ),
        ("/elements/0/transition_out/kind", json!("spin"), "`spin`"),
        ("/elements/3/style/size", json!(4000.0), "style"),
        ("/elements/3/style/line_height", json!(10.0), "style"),
    ];
    for (pointer, bad, needle) in cases {
        let mut value = good();
        set(&mut value, pointer, bad);
        refused(&value, needle);
    }
}

/// Set the field at `pointer`, adding it if its parent object lacks it.
fn set(value: &mut Value, pointer: &str, new: Value) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let parent = value
        .pointer_mut(parent)
        .unwrap_or_else(|| panic!("no parent for {pointer}"));
    parent.as_object_mut().unwrap().insert(key.to_owned(), new);
}

/// The name a kind is written as in a template, from the model itself.
fn token<T: serde::Serialize>(kind: T) -> String {
    serde_json::to_value(kind)
        .expect("a kind serializes")
        .as_str()
        .expect("a kind is a plain name")
        .to_owned()
}

/// A template can give a *shot* an entrance, not only a title. The
/// presets are the same ones, so a template author who has animated a caption
/// already knows how.
#[test]
fn a_shot_can_arrive_and_leave() {
    use bettercut_timeline::MotionKind;

    let mut value = good();
    value["elements"][0]["animation"] = json!({
        "in": { "kind": "slide_right", "duration": 0.5 },
        "out": { "kind": "spin", "duration": 0.4 }
    });

    let template = parse(&value.to_string()).expect("valid");
    let Element::Clip { motion, .. } = &template.elements[0] else {
        panic!("the first element is a clip");
    };
    assert_eq!(
        motion.intro.expect("an entrance").kind,
        MotionKind::SlideRight
    );
    assert_eq!(motion.outro.expect("an exit").kind, MotionKind::Spin);
}

/// And the one motion a picture may not have. A shot has no letters to reveal,
/// so a template asking for a typewriter is an author who would never work out
/// why nothing happens — better to say so at validation.
#[test]
fn a_shot_cannot_be_typed_out_letter_by_letter() {
    let mut value = good();
    value["elements"][0]["animation"] = json!({
        "in": { "kind": "typewriter", "duration": 0.5 }
    });
    refused(&value, "typewriter");

    // Still allowed on a title, which is what it was made for.
    let mut value = good();
    value["elements"][3]["animation"] = json!({
        "in": { "kind": "typewriter", "duration": 0.5 }
    });
    parse(&value.to_string()).expect("a title may still type itself in");
}

/// Every transition the editor has, expressible in a template.
///
/// The validator used to carry its own list of two while the editor grew to
/// seven, so a template asking for a zoom was told a zoom is not a transition.
/// Walking the model's own list is what stops that returning: a kind added
/// tomorrow fails here until a template can name it.
#[test]
fn every_transition_kind_can_be_written_in_a_template() {
    use bettercut_timeline::TransitionKind;

    for kind in TransitionKind::ALL {
        let mut value = good();
        value["elements"][0]["transition_out"] = json!({ "kind": token(kind), "duration": 0.5 });

        let template = parse(&value.to_string())
            .unwrap_or_else(|problems| panic!("{} was refused: {problems:?}", token(kind)));
        let Element::Clip {
            transition_out: Some((written, _)),
            ..
        } = &template.elements[0]
        else {
            panic!("the first element should carry a transition");
        };
        assert_eq!(*written, kind, "a template changed the kind");
    }
}

/// And every motion a title can have, for the same reason — three of them had
/// been added to the editor and never reached the format.
#[test]
fn every_text_motion_can_be_written_in_a_template() {
    use bettercut_timeline::MotionKind;

    for kind in MotionKind::FOR_TEXT {
        let mut value = good();
        value["elements"][3]["animation"] = json!({
            "in": { "kind": token(kind), "duration": 0.5 }
        });

        let template = parse(&value.to_string())
            .unwrap_or_else(|problems| panic!("{} was refused: {problems:?}", token(kind)));
        let Element::Text { animation, .. } = &template.elements[3] else {
            panic!("fourth element is text");
        };
        assert_eq!(animation.intro.expect("an entrance").kind, kind);
    }
}

#[test]
fn a_title_can_arrive_and_leave() {
    use bettercut_timeline::MotionKind;

    let mut value = good();
    value["elements"][3]["animation"] = json!({
        "in": { "kind": "typewriter", "duration": 1.0 },
        "out": { "kind": "slide_down", "duration": 0.5 }
    });
    let template = parse(&value.to_string()).expect("valid");
    let Element::Text { animation, .. } = &template.elements[3] else {
        panic!("fourth element is text");
    };
    let intro = animation.intro.expect("an entrance");
    assert_eq!(intro.kind, MotionKind::Typewriter);
    assert_eq!(intro.duration, secs(1));
    assert_eq!(
        animation.outro.expect("an exit").kind,
        MotionKind::SlideDown
    );

    for (motion, needle) in [
        (
            json!({ "in": { "kind": "wobble", "duration": 1 } }),
            "`wobble`",
        ),
        (
            json!({ "in": { "kind": "fade", "duration": 9 } }),
            "animation.in.duration",
        ),
        (json!({ "sideways": {} }), "sideways"),
    ] {
        let mut value = good();
        value["elements"][3]["animation"] = motion;
        refused(&value, needle);
    }
}

#[test]
fn a_shot_can_be_given_a_slow_move() {
    use bettercut_timeline::Movement;

    let mut value = good();
    value["elements"][0]["movement"] = json!("zoom_in");
    let template = parse(&value.to_string()).expect("valid");
    let Element::Clip { movement, .. } = &template.elements[0] else {
        panic!("first element is a clip");
    };
    assert_eq!(*movement, Movement::ZoomIn);

    let mut value = good();
    value["elements"][0]["movement"] = json!("spin");
    refused(&value, "`spin` is not a movement");
}

#[test]
fn a_style_needs_only_what_it_changes() {
    use bettercut_text::{FontFamily, TextStyle};

    let mut value = good();
    value["elements"][3]["style"] = json!({ "size": 96.0, "family": "serif" });
    let template = parse(&value.to_string()).expect("a partial style is valid");
    let Element::Text { style, .. } = &template.elements[3] else {
        panic!("fourth element is text");
    };
    assert_eq!(style.size, 96.0);
    assert_eq!(style.family, FontFamily::Serif);
    // Everything not mentioned is a new title's.
    assert_eq!(style.color, TextStyle::default().color);
    assert_eq!(style.stroke, TextStyle::default().stroke);

    let mut value = good();
    value["elements"][3]["style"] = json!({ "colour": { "r": 1, "g": 2, "b": 3, "a": 4 } });
    refused(&value, "style.colour: is not a text style setting");

    let mut value = good();
    value["elements"][3]["style"] = json!({ "weight": "heavy" });
    refused(&value, "elements[3].style");

    let mut value = good();
    value["elements"][3]["style"] = json!("big");
    refused(&value, "must be an object");
}

#[test]
fn sizes_are_bounded() {
    let mut value = good();
    let slots: Vec<Value> = (0..40)
        .map(|i| json!({ "id": format!("s{i}"), "type": "video" }))
        .collect();
    value["slots"] = json!(slots);
    refused(&value, "the limit is 32");

    let mut value = good();
    let many: Vec<Value> = (0..300).map(|_| value["elements"][0].clone()).collect();
    value["elements"] = json!(many);
    refused(&value, "the limit is 256");

    let mut value = good();
    value["elements"][3]["text"] = json!("x".repeat(3000));
    refused(&value, "too long");

    let huge = format!("{{\"pad\": \"{}\"}}", "x".repeat(2 * 1024 * 1024));
    let problems = parse(&huge).unwrap_err();
    assert!(problems[0].message.contains("limited to"), "{problems:?}");

    let mut value = good();
    value["elements"] = json!([]);
    refused(&value, "must place something");
}

#[test]
fn malformed_input_never_panics() {
    let text = good().to_string();
    let bytes = text.as_bytes();

    // Every truncation.
    for end in 0..bytes.len() {
        if let Ok(prefix) = std::str::from_utf8(&bytes[..end]) {
            let _ = parse(prefix);
        }
    }

    // A few thousand deterministic single-byte corruptions. No random crate:
    // a fixed LCG keeps any failure reproducible.
    let mut seed: u64 = 0x5eed;
    let replacements = b"\"{}[],:0-9.eE+ nulltruefalse\\";
    for _ in 0..4000 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let at = (seed >> 33) as usize % bytes.len();
        let with = replacements[(seed >> 17) as usize % replacements.len()];
        let mut corrupted = bytes.to_vec();
        corrupted[at] = with;
        if let Ok(corrupted) = String::from_utf8(corrupted) {
            let _ = parse(&corrupted);
        }
    }

    // Values of every JSON type in every field.
    let weird = [
        json!(null),
        json!(-1),
        json!(1e308),
        json!(u64::MAX),
        json!(""),
        json!([]),
        json!({}),
        json!(true),
    ];
    let mut paths = Vec::new();
    collect_pointers(&good(), String::new(), &mut paths);
    for pointer in &paths {
        for value in &weird {
            let mut template = good();
            *template.pointer_mut(pointer).unwrap() = value.clone();
            let _ = parse(&template.to_string());
        }
    }
}

fn collect_pointers(value: &Value, at: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                collect_pointers(child, format!("{at}/{key}"), out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_pointers(child, format!("{at}/{index}"), out);
            }
        }
        _ => {}
    }
    if !at.is_empty() {
        out.push(at);
    }
}

#[test]
fn bundle_paths_cannot_leave_the_bundle() {
    for good in ["preview.webp", "assets/logo.png", "a/b/c.mp4"] {
        assert!(is_bundle_relative(good), "{good}");
    }
    for bad in [
        "",
        "/etc/passwd",
        "\\\\server\\share\\x",
        "C:\\Windows\\x",
        "C:x",
        "../x",
        "assets/../../x",
        "assets/./x",
        "assets//x",
        "assets\\x",
        "nul\0byte",
    ] {
        assert!(!is_bundle_relative(bad), "{bad:?}");
    }
}

/// A template can mirror a shot, and the two axes stay apart on the way in.
///
/// `deny_unknown_fields` means a field the format does not know is a hard
/// refusal, so "the template says flip_h and the clip comes back mirrored" is
/// two claims at once: the schema accepts the key, and the value reaches the
/// transform rather than being read and dropped.
#[test]
fn a_template_can_mirror_a_shot() {
    let mut value = good();
    value["elements"][0]["transform"]["flip_h"] = json!(true);

    let template = parse(&value.to_string()).expect("a mirrored template is valid");
    let Element::Clip { transform, .. } = &template.elements[0] else {
        panic!("first element is a clip");
    };
    assert!(transform.flip_h, "the mirror did not reach the transform");
    assert!(
        !transform.flip_v,
        "a left-to-right mirror also turned the picture over"
    );

    // And unstated stays unmirrored, so every template written before the
    // mirror existed still means what it said.
    let plain = parse(&good().to_string()).expect("the fixture is valid");
    let Element::Clip { transform, .. } = &plain.elements[0] else {
        panic!("first element is a clip");
    };
    assert!(!transform.flip_h && !transform.flip_v);
}

/// A template can crop a shot, and an impossible crop is refused rather than
/// quietly trimmed to fit.
///
/// Rejecting rather than clamping is this module's rule: the author is still
/// writing the file and can be told, where a user dragging a slider is watching
/// the result and should simply be stopped at the limit.
#[test]
fn a_template_can_crop_a_shot() {
    let mut value = good();
    value["elements"][0]["crop"] = json!([0.1, 0.2, 0.05, 0.0]);

    let template = parse(&value.to_string()).expect("a cropped template is valid");
    let Element::Clip { crop, .. } = &template.elements[0] else {
        panic!("first element is a clip");
    };
    assert_eq!(crop.left, 0.1);
    assert_eq!(crop.top, 0.2);
    assert_eq!(crop.right, 0.05);
    assert_eq!(crop.bottom, 0.0);

    // Unstated is uncropped, so every template written before §22's crop
    // existed still means what it said.
    let plain = parse(&good().to_string()).expect("the fixture is valid");
    let Element::Clip { crop, .. } = &plain.elements[0] else {
        panic!("first element is a clip");
    };
    assert!(crop.is_none());
}

#[test]
fn a_crop_that_leaves_nothing_is_refused() {
    let mut value = good();
    value["elements"][0]["crop"] = json!([0.6, 0.0, 0.6, 0.0]);
    refused(&value, "crop");

    let mut value = good();
    value["elements"][0]["crop"] = json!([0.0, 0.0, 1.5, 0.0]);
    refused(&value, "crop");
}
