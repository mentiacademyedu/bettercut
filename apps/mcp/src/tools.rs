//! The tools an assistant can call, and the project they act on.
//!
//! Each tool is a thin wrapper over one editor operation, so what it does is
//! exactly what the same action in the app does — and is undone the same way.
//! Times are seconds, the unit an assistant reasons in; they are turned into
//! the editor's exact ticks at the millisecond.

use std::path::{Path, PathBuf};

use bettercut_editor_core::filters::Filter;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::project_format::Project;
use bettercut_editor_core::timeline::{AnimatedParameter, Movement, TransitionKind};
use bettercut_editor_core::{ClipProperty, Editor, EventReceiver, TextProperty, TrimEdge};
use serde_json::{Value, json};

use crate::live;

/// The open project, if there is one.
#[derive(Default)]
pub struct Session {
    open: Option<(Editor, EventReceiver)>,
    /// The app's window, while attached: then edits go there instead.
    app: Option<live::Link>,
    /// Where to look for the window; `None` for the usual place.
    live_file: Option<PathBuf>,
}

/// What a tool hands back: words, or a picture with a line about it.
pub enum Reply {
    Text(String),
    Image { png: Vec<u8>, caption: String },
}

impl From<String> for Reply {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

/// One tool: its name, what it is for, and the arguments it takes.
struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "attach_to_app",
        description: "Edit the project open in the bettercut app's window instead of one of \
                      this session's own: every tool after this acts there, live, on screen and \
                      in the app's undo history (export renders a copy, so the window stays \
                      usable). The app must be running. detach_from_app to stop.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "detach_from_app",
        description: "Stop editing in the app's window; tools act on this session's own \
                      project again.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "new_project",
        description: "Create a new, empty project and save it at `path` (a .vproj file). \
                      Optionally set the picture size and frame rate (default 1920x1080 at 30).",
        schema: || {
            object(
                json!({
                    "path": { "type": "string", "description": "Where to save the .vproj file" },
                    "width": { "type": "integer", "minimum": 16 },
                    "height": { "type": "integer", "minimum": 16 },
                    "fps": { "type": "integer", "minimum": 1, "maximum": 240 }
                }),
                &["path"],
            )
        },
    },
    Tool {
        name: "open_project",
        description: "Open an existing bettercut project (.vproj).",
        schema: || object(json!({ "path": { "type": "string" } }), &["path"]),
    },
    Tool {
        name: "save_project",
        description: "Save the open project, or save it under a new `path`.",
        schema: || object(json!({ "path": { "type": "string" } }), &[]),
    },
    Tool {
        name: "describe_project",
        description: "The open project: its format and length, the media imported into it \
                      (with ids), and every clip on every lane (with ids, start and end).",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "import_media",
        description: "Import video, sound or photo files into the project's media. Returns \
                      each file's media id (or why it could not be imported).",
        schema: || {
            object(
                json!({ "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1 } }),
                &["paths"],
            )
        },
    },
    Tool {
        name: "add_to_timeline",
        description: "Place imported media at the end of the timeline: its picture on the \
                      first picture lane and its sound on the first sound lane. Optionally only \
                      the part from `from` to `to` seconds of the file. Returns the new clip ids.",
        schema: || {
            object(
                json!({
                    "media_id": { "type": "string" },
                    "from": { "type": "number", "minimum": 0 },
                    "to": { "type": "number", "minimum": 0 }
                }),
                &["media_id"],
            )
        },
    },
    Tool {
        name: "add_title",
        description: "Add a title (text over the picture) starting at `at` seconds.",
        schema: || {
            object(
                json!({
                    "text": { "type": "string", "minLength": 1 },
                    "at": { "type": "number", "minimum": 0 }
                }),
                &["text", "at"],
            )
        },
    },
    Tool {
        name: "set_transform",
        description: "Move, size or turn a picture clip or a title. `x`/`y` place its centre as \
                      an offset from the middle of the frame, in fractions of the frame: 0.5 is \
                      the right (x) or bottom (y) edge, -0.5 the left or top. `scale` 1 is its \
                      normal size; `rotation` is in degrees, clockwise. Leave out what should \
                      stay. One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "scale": { "type": "number", "exclusiveMinimum": 0 },
                    "rotation": { "type": "number" }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "style_title",
        description: "Change a title's words or look: `text`, `size` (pixels at the project's \
                      height), `color` (\"#rrggbb\"), `bold`, `italic`, an `outline` colour \
                      (\"none\" removes it) and a `background` box colour (\"none\" removes \
                      it). Leave out what should stay. One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "text": { "type": "string", "minLength": 1 },
                    "size": { "type": "number", "minimum": 4, "maximum": 1000 },
                    "color": { "type": "string" },
                    "bold": { "type": "boolean" },
                    "italic": { "type": "boolean" },
                    "outline": { "type": "string" },
                    "background": { "type": "string" }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "adjust_colour",
        description: "Correct a picture clip's colour. `brightness`, `contrast` and \
                      `saturation` are 1 for unchanged (0–2; saturation 0 is black and \
                      white); `temperature` is 0 for unchanged, -1 cooler to 1 warmer. Leave \
                      out what should stay. One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "brightness": { "type": "number", "minimum": 0, "maximum": 2 },
                    "contrast": { "type": "number", "minimum": 0, "maximum": 2 },
                    "saturation": { "type": "number", "minimum": 0, "maximum": 2 },
                    "temperature": { "type": "number", "minimum": -1, "maximum": 1 }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "set_movement",
        description: "Give a picture clip a slow camera move over its length — the \"Ken \
                      Burns\" look for photos: `movement` is zoom in, zoom out, pan left, pan \
                      right, pan up, pan down, or none to take it off. `strength` is gentle, \
                      normal (default) or strong.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "movement": { "type": "string" },
                    "strength": { "type": "string", "enum": ["gentle", "normal", "strong"] }
                }),
                &["clip_id", "movement"],
            )
        },
    },
    Tool {
        name: "animate_title",
        description: "How a title arrives and leaves: `intro` and `outro` are fade, slide up, \
                      slide down, slide right, slide left, pop, bounce, spin, typewriter, or \
                      none; `duration` is each one's length in seconds (default 0.5). \
                      `looping` keeps it moving in between: pulse, wiggle, spin, float, or \
                      none. Leave out what should stay. One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "intro": { "type": "string" },
                    "outro": { "type": "string" },
                    "duration": { "type": "number", "exclusiveMinimum": 0 },
                    "looping": { "type": "string" }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "animate",
        description: "Animate one property of a clip over time with keyframes, replacing any \
                      animation it had. `property` is opacity, x, y, scale, rotation, \
                      brightness, contrast, saturation, temperature or blur for a picture clip \
                      (x/y/scale/rotation as in set_transform), or volume or pan for a sound \
                      clip. `keys` are `{at, value, easing?}` with `at` in timeline seconds \
                      inside the clip; easing is linear (default), hold, ease in, ease out or \
                      ease in-out, and shapes the way to the next key. An empty `keys` list \
                      takes the animation off. One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "property": { "type": "string" },
                    "keys": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "at": { "type": "number", "minimum": 0 },
                                "value": { "type": "number" },
                                "easing": { "type": "string" }
                            },
                            "required": ["at", "value"]
                        }
                    }
                }),
                &["clip_id", "property", "keys"],
            )
        },
    },
    Tool {
        name: "list_templates",
        description: "The templates there are to start an edit from — bettercut's own and the \
                      person's — each with its slots: the shots, photos, sound and words it \
                      asks for.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "apply_template",
        description: "Build a template on the timeline at `at` seconds (default: the end of \
                      the edit). `fills` maps each slot id to a media_id (for a video, image, \
                      logo or audio slot) or to the words (for a text slot); a text slot left \
                      out keeps its default words, a media slot left out is skipped. One undo \
                      step.",
        schema: || {
            object(
                json!({
                    "template_id": { "type": "string" },
                    "at": { "type": "number", "minimum": 0 },
                    "fills": { "type": "object", "additionalProperties": { "type": "string" } }
                }),
                &["template_id"],
            )
        },
    },
    Tool {
        name: "normalise_volume",
        description: "Level a sound clip so its loudest moment sits just under full volume — \
                      the fix for \"this one is too quiet\" (or too loud). Reads the whole \
                      sound first, so a long file takes a moment.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "duck_under_voice",
        description: "Dip a sound clip (music, usually) wherever another sound clip has \
                      someone speaking over it, and bring it back up between. The dips are \
                      volume keys on the clip. Reads the other clips' sound first.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "split_at_scenes",
        description: "Find where the shot changes inside a picture clip — a long recording \
                      with several scenes — and cut the clip there, so each scene is its own \
                      clip. With `preview` true nothing is cut: the times are listed. Reads \
                      the footage, so a long clip takes a while. One undo step.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "preview": { "type": "boolean" } }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "mark_beats",
        description: "Put a marker on every beat of a music clip, to cut to the rhythm \
                      (split_clip, move_clip and trim_clip line up with markers). Says the \
                      tempo it found.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "remove_silences",
        description: "Cut the pauses out of a clip with someone talking, closing each gap — \
                      the jump-cut edit. Works on a sound clip or a picture clip with sound; \
                      the picture is cut with it. `threshold_db` is how quiet counts as \
                      silence (default -34), `shortest` the shortest pause to cut in seconds \
                      (default 0.6), `padding` the breath kept each side (default 0.12). With \
                      `preview` true nothing is cut: the pauses found are listed. One undo \
                      step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "threshold_db": { "type": "number", "minimum": -42, "maximum": -6 },
                    "shortest": { "type": "number", "minimum": 0.1 },
                    "padding": { "type": "number", "minimum": 0 },
                    "preview": { "type": "boolean" }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "enhance_voice",
        description: "Clean up a spoken recording in one step: less hiss and rumble, a little \
                      presence, quiet and loud words brought closer, sharp \"s\" sounds \
                      softened. For a sound clip.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "mute_clip",
        description: "Silence a clip's sound (`muted` true, the default) or bring it back \
                      (false). A picture clip's own sound is muted with it.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "muted": { "type": "boolean" } }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "get_selection",
        description: "When attached to the app's window: the clips the person has selected \
                      there, and where the playhead is — what \"this clip\" and \"here\" mean \
                      when they ask.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "select_clips",
        description: "When attached to the app's window: select these clips there, to show \
                      the person which ones you mean (an empty list clears the selection).",
        schema: || {
            object(
                json!({ "clip_ids": { "type": "array", "items": { "type": "string" } } }),
                &["clip_ids"],
            )
        },
    },
    Tool {
        name: "set_playhead",
        description: "Move the playhead to `at` seconds — in the app's window, where the \
                      person is looking, when attached.",
        schema: || object(json!({ "at": { "type": "number", "minimum": 0 } }), &["at"]),
    },
    Tool {
        name: "add_colour",
        description: "Add a clip that is all one colour (\"#rrggbb\"), or a gradient from \
                      `color` at the top to `to` at the bottom, at `at` seconds — a \
                      background for a title card, say. It goes under anything already \
                      there.",
        schema: || {
            object(
                json!({
                    "color": { "type": "string" },
                    "to": { "type": "string" },
                    "at": { "type": "number", "minimum": 0 }
                }),
                &["color", "at"],
            )
        },
    },
    Tool {
        name: "freeze_frame",
        description: "Hold the picture of a clip at `at` seconds still for `duration` seconds \
                      (default 2), pushing what follows later.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "at": { "type": "number", "minimum": 0 },
                    "duration": { "type": "number", "exclusiveMinimum": 0 }
                }),
                &["clip_id", "at"],
            )
        },
    },
    Tool {
        name: "picture_in_picture",
        description: "Shrink a picture clip into a corner over whatever is beneath it: \
                      `corner` is top left, top right, bottom left or bottom right; `size` is \
                      small, medium (default), large — or full to fill the frame again.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "corner": { "type": "string" },
                    "size": { "type": "string" }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "copy_as_shape",
        description: "Make a copy of the edit in another shape — 9:16 for Shorts, TikTok and \
                      Reels, 1:1, 4:5, 16:9 or 21:9 — with every shot reframed to fill it, and \
                      switch to the copy, so the tools after this edit it. switch_sequence goes \
                      back.",
        schema: || object(json!({ "shape": { "type": "string" } }), &["shape"]),
    },
    Tool {
        name: "switch_sequence",
        description: "Work on another of the project's sequences (describe_project lists them).",
        schema: || {
            object(
                json!({ "sequence_id": { "type": "string" } }),
                &["sequence_id"],
            )
        },
    },
    Tool {
        name: "add_lower_third",
        description: "Introduce someone on screen: their `name` large and `role` smaller \
                      beneath, with a coloured bar, low on the left, for five seconds from \
                      `at`. `accent` is the bar's colour (\"#rrggbb\", orange by default). \
                      Returns the bar, name and role clips, which style_title and \
                      animate_title can change.",
        schema: || {
            object(
                json!({
                    "name": { "type": "string", "minLength": 1 },
                    "role": { "type": "string" },
                    "at": { "type": "number", "minimum": 0 },
                    "accent": { "type": "string" }
                }),
                &["name", "at"],
            )
        },
    },
    Tool {
        name: "add_shape",
        description: "Draw a shape over the picture from `at` seconds: rectangle, rounded \
                      rectangle, ellipse, arrow, star or speech bubble. It is a title-lane \
                      clip, so set_transform places it and style_title's `color` fills it.",
        schema: || {
            object(
                json!({
                    "shape": { "type": "string" },
                    "at": { "type": "number", "minimum": 0 }
                }),
                &["shape", "at"],
            )
        },
    },
    Tool {
        name: "add_sticker",
        description: "Drop a sticker on the picture from `at` seconds: heart, star, tick, \
                      cross, smile, frown, sun, cloud, umbrella, snowman, lightning, alarm \
                      clock, music note, telephone, aeroplane or football.",
        schema: || {
            object(
                json!({
                    "sticker": { "type": "string" },
                    "at": { "type": "number", "minimum": 0 }
                }),
                &["sticker", "at"],
            )
        },
    },
    Tool {
        name: "add_timer",
        description: "A number on screen counting `down` (default) or `up` from `at` seconds, \
                      for a countdown or a stopwatch.",
        schema: || {
            object(
                json!({
                    "at": { "type": "number", "minimum": 0 },
                    "direction": { "type": "string", "enum": ["down", "up"] }
                }),
                &["at"],
            )
        },
    },
    Tool {
        name: "add_marker",
        description: "Put a marker on the timeline at `at` seconds, optionally named with \
                      `label` — to note a beat, a chapter or a cut to make.",
        schema: || {
            object(
                json!({
                    "at": { "type": "number", "minimum": 0 },
                    "label": { "type": "string" }
                }),
                &["at"],
            )
        },
    },
    Tool {
        name: "split_clip",
        description: "Cut a clip in two at `at` seconds on the timeline.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "at": { "type": "number", "minimum": 0 } }),
                &["clip_id", "at"],
            )
        },
    },
    Tool {
        name: "delete_clip",
        description: "Remove a clip (or title). With `ripple`, everything after it on its lane \
                      moves up to close the gap.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "ripple": { "type": "boolean" } }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "set_volume",
        description: "Set a sound clip's volume: 1 is as recorded, 0 silent, up to 4. Use a \
                      clip id from the sound lanes.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "volume": { "type": "number", "minimum": 0, "maximum": 4 } }),
                &["clip_id", "volume"],
            )
        },
    },
    Tool {
        name: "set_opacity",
        description: "Set a picture clip's opacity: 1 solid, 0 invisible.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "opacity": { "type": "number", "minimum": 0, "maximum": 1 } }),
                &["clip_id", "opacity"],
            )
        },
    },
    Tool {
        name: "set_speed",
        description: "Play a clip faster or slower: 2 is twice as fast (half as long), 0.5 half \
                      speed. Its linked sound follows.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "speed": { "type": "number", "minimum": 0.1, "maximum": 10 } }),
                &["clip_id", "speed"],
            )
        },
    },
    Tool {
        name: "reverse_clip",
        description: "Play a clip backwards (or forwards again with `reversed: false`).",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "reversed": { "type": "boolean" } }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "set_fades",
        description: "Fade a clip in from its start and out to its end, in seconds (0 for none).",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "fade_in": { "type": "number", "minimum": 0 },
                        "fade_out": { "type": "number", "minimum": 0 } }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "add_transition",
        description: "Put a transition at the end of a picture clip, into the next one. \
                      Optionally its length in seconds.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "kind": { "type": "string", "enum": transition_names() },
                        "duration": { "type": "number", "minimum": 0.1 } }),
                &["clip_id", "kind"],
            )
        },
    },
    Tool {
        name: "apply_filter",
        description: "Give picture clips a filter's look (\"Original\" takes it off again).",
        schema: || {
            object(
                json!({ "clip_ids": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                        "filter": { "type": "string", "enum": filter_names() } }),
                &["clip_ids", "filter"],
            )
        },
    },
    Tool {
        name: "move_clip",
        description: "Move a clip along its lane so it starts at `start` seconds.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "start": { "type": "number", "minimum": 0 } }),
                &["clip_id", "start"],
            )
        },
    },
    Tool {
        name: "trim_clip",
        description: "Move a clip's start or end edge to `to` seconds on the timeline, showing \
                      more or less of the file.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "edge": { "type": "string", "enum": ["start", "end"] },
                        "to": { "type": "number", "minimum": 0 } }),
                &["clip_id", "edge", "to"],
            )
        },
    },
    Tool {
        name: "import_captions",
        description: "Import captions from an .srt or .vtt file onto the caption lane.",
        schema: || object(json!({ "path": { "type": "string" } }), &["path"]),
    },
    Tool {
        name: "add_captions",
        description: "Put captions (subtitles) on the caption lane from a list of lines, each \
                      `{start, end, text}` in seconds, replacing any captions already there. \
                      Overlaps are shortened, not moved. One undo step.",
        schema: || {
            object(
                json!({
                    "lines": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "start": { "type": "number", "minimum": 0 },
                                "end": { "type": "number", "minimum": 0 },
                                "text": { "type": "string", "minLength": 1 }
                            },
                            "required": ["start", "end", "text"]
                        }
                    }
                }),
                &["lines"],
            )
        },
    },
    Tool {
        name: "preview_frame",
        description: "See the edit: the frame at `at` seconds, rendered exactly as it will be \
                      exported, as a PNG image (at most `max_width` pixels wide, default 960).",
        schema: || {
            object(
                json!({ "at": { "type": "number", "minimum": 0 },
                        "max_width": { "type": "integer", "minimum": 64, "maximum": 3840 } }),
                &["at"],
            )
        },
    },
    Tool {
        name: "undo",
        description: "Undo the last edit.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "redo",
        description: "Redo the last undone edit.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "export",
        description: "Render the whole timeline to a video file at `path` (.mp4 recommended), at \
                      the project's size unless `width`/`height` are given. Waits until done.",
        schema: || {
            object(
                json!({
                    "path": { "type": "string" },
                    "width": { "type": "integer", "minimum": 16 },
                    "height": { "type": "integer", "minimum": 16 }
                }),
                &["path"],
            )
        },
    },
];

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

/// The tools, as `tools/list` reports them.
pub fn list() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.schema)(),
            })
        })
        .collect()
}

pub fn exists(name: &str) -> bool {
    TOOLS.iter().any(|tool| tool.name == name)
}

impl Session {
    /// Run tool `name`. The text is what the assistant reads back: JSON for
    /// anything with structure, a sentence otherwise.
    pub fn call(&mut self, name: &str, args: &Value) -> Result<Reply, String> {
        match name {
            "attach_to_app" => {
                let file = self.live_file.clone().unwrap_or_else(live::live_file);
                let link = live::Link::connect(&file)?;
                let described = link.call("describe_project", &json!({}))?;
                self.app = Some(link);
                let name = match described {
                    Reply::Text(text) => serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v["project"].as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    Reply::Image { .. } => String::new(),
                };
                Ok(Reply::Text(format!(
                    "Attached to the bettercut window (project \"{name}\"): edits now happen \
                     there, live, in its undo history. detach_from_app to stop."
                )))
            }
            "detach_from_app" => Ok(Reply::Text(if self.app.take().is_some() {
                "Detached: edits go to this session's own project again".to_owned()
            } else {
                "Not attached".to_owned()
            })),
            _ if self.app.is_some() => match name {
                "new_project" | "open_project" => Err(
                    "Attached to the app's window: open projects there, or detach_from_app first"
                        .to_owned(),
                ),
                _ => self.app.as_ref().map_or_else(
                    || Err("not attached".to_owned()),
                    |app| app.call(name, args),
                ),
            },
            "new_project" => self.new_project(args).map(Reply::Text),
            "open_project" => {
                let path = path_arg(args, "path")?;
                let opened = Editor::open(&path).map_err(|e| format!("could not open: {e}"))?;
                self.open = Some(opened);
                Ok(Reply::Text(format!("Opened {}", path.display())))
            }
            "get_selection" | "select_clips" => {
                Err("only the app's window has a selection: attach_to_app first".to_owned())
            }
            _ => run_on(self.editor()?, name, args),
        }
    }

    /// Look for the app's window in `file` rather than the usual place.
    pub fn with_live_file(file: PathBuf) -> Self {
        Self {
            live_file: Some(file),
            ..Self::default()
        }
    }

    fn editor(&mut self) -> Result<&mut Editor, String> {
        self.open
            .as_mut()
            .map(|(editor, _)| editor)
            .ok_or_else(|| "No project is open: call open_project or new_project first".to_owned())
    }

    fn new_project(&mut self, args: &Value) -> Result<String, String> {
        let path = path_arg(args, "path")?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_owned();
        let (mut editor, events) = Editor::new_project(name);
        let width = u32_arg(args, "width")?.unwrap_or(1920);
        let height = u32_arg(args, "height")?.unwrap_or(1080);
        let fps = u32_arg(args, "fps")?.unwrap_or(30);
        let rate = bettercut_editor_core::foundation::FrameRate::new(i64::from(fps), 1)
            .ok_or("that frame rate is not one bettercut can use")?;
        editor
            .set_sequence_format(
                bettercut_editor_core::timeline::Resolution::new(width, height),
                rate,
            )
            .map_err(|e| e.to_string())?;
        editor
            .save_as(&path)
            .map_err(|e| format!("could not save: {e}"))?;
        self.open = Some((editor, events));
        Ok(format!(
            "Created {} ({width}x{height} at {fps} fps)",
            path.display()
        ))
    }
}

/// Run an editing tool on `editor`: the MCP server's own project, or the
/// project open in the app's window. Everything but new_project and
/// open_project.
pub fn run_on(editor: &mut Editor, name: &str, args: &Value) -> Result<Reply, String> {
    if name == "preview_frame" {
        return preview_frame(editor, args);
    }
    run_text(editor, name, args).map(Reply::Text)
}

fn run_text(editor: &mut Editor, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "save_project" => {
            match args.get("path").and_then(Value::as_str) {
                Some(path) => editor.save_as(path),
                None => editor.save(),
            }
            .map_err(|e| format!("could not save: {e}"))?;
            Ok("Saved".to_owned())
        }
        "describe_project" => Ok(describe(editor).to_string()),
        "import_media" => import_media(editor, args),
        "add_to_timeline" => add_to_timeline(editor, args),
        "set_transform" => set_transform(editor, args),
        "style_title" => style_title(editor, args),
        "adjust_colour" => adjust_colour(editor, args),
        "set_movement" => {
            use bettercut_editor_core::timeline::MovementStrength;
            let clip = clip_arg(args)?;
            let movement = one_of(
                &Movement::ALL,
                Movement::label,
                str_arg(args, "movement")?,
                "movement",
            )?;
            let strength = match args.get("strength").and_then(Value::as_str) {
                None => MovementStrength::Normal,
                Some(name) => one_of(
                    &MovementStrength::ALL,
                    MovementStrength::label,
                    name,
                    "strength",
                )?,
            };
            if editor.video_clip(clip).is_none() {
                return Err("that is not a picture clip".to_owned());
            }
            editor
                .set_movement_at(clip, movement, strength)
                .map_err(|e| e.to_string())?;
            Ok(format!("{} ({})", movement.label(), strength.label()))
        }
        "animate_title" => animate_title(editor, args),
        "animate" => animate(editor, args),
        "list_templates" => Ok(Value::Array(
            templates()
                .iter()
                .map(|t| {
                    json!({
                        "template_id": t.id,
                        "name": t.name,
                        "category": t.category,
                        "description": t.description,
                        "duration": seconds_of(t.duration.ticks()),
                        "slots": t.slots.iter().map(|slot| json!({
                            "id": slot.id,
                            "kind": format!("{:?}", slot.kind).to_lowercase(),
                            "label": slot.label,
                            "default_text": slot.default_text,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )
        .to_string()),
        "apply_template" => apply_template(editor, args),
        "normalise_volume" => {
            let clip = sound_clip(editor, args)?;
            let waveform = waveform_of(editor, clip.media_id)?;
            let gain = bettercut_playback::normalise(&clip, &waveform)
                .ok_or("this clip is already at a good level, or too quiet to raise")?;
            editor
                .set_clip_property(clip.id, ClipProperty::Gain(gain), false)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "Level set to {:+.1} dB",
                20.0 * gain.max(1e-6).log10()
            ))
        }
        "duck_under_voice" => {
            let music = sound_clip(editor, args)?;
            let others: Vec<_> = editor
                .active_sequence()
                .map(|s| {
                    s.audio_tracks
                        .iter()
                        .flat_map(|t| t.clips().iter())
                        .filter(|o| o.id != music.id && o.timeline.overlaps(music.timeline))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let mut speech = Vec::new();
            for other in &others {
                let waveform = waveform_of(editor, other.media_id)?;
                speech.extend(bettercut_playback::speech_ranges(
                    other,
                    &waveform,
                    bettercut_playback::SpeechSettings::default(),
                ));
            }
            if speech.is_empty() {
                return Err("nothing is speaking over this clip".to_owned());
            }
            let points = bettercut_playback::duck_envelope(
                &music,
                &speech,
                bettercut_playback::DuckSettings::default(),
            );
            let keys = editor
                .set_gain_envelope(music.id, &points, false)
                .map_err(|e| e.to_string())?;
            if keys == 0 {
                return Err("nothing to duck under on this clip".to_owned());
            }
            Ok(format!(
                "Ducked under the voice ({} speaking parts)",
                speech.len()
            ))
        }
        "split_at_scenes" => {
            use bettercut_jobs::{JobContext, Task};
            let clip = clip_arg(args)?;
            let picture = editor
                .video_clip(clip)
                .ok_or("that is not a picture clip")?
                .clone();
            let asset = editor
                .project()
                .media
                .iter()
                .find(|m| m.id == picture.media_id)
                .ok_or("the clip's file is not in the project")?
                .clone();
            let (mut job, report) = bettercut_playback::SceneJob::new(
                &asset,
                picture.source.start,
                picture.source.end,
                bettercut_playback::SceneSettings::default(),
                2,
            )
            .ok_or("only a video clip has scenes to find")?;
            job.run(&JobContext::detached())
                .map_err(|e| format!("could not read the footage: {e}"))?;
            let cuts = report.cuts().unwrap_or_default();
            let at = bettercut_playback::scene_job::timeline_cuts(&picture, &cuts);
            let listed: Vec<f64> = at.iter().map(|t| t.as_seconds_f64()).collect();
            if args
                .get("preview")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(json!({ "scene_changes": listed }).to_string());
            }
            if at.is_empty() {
                return Err("no change of shot inside this clip".to_owned());
            }
            let made = editor.split_clip_at(clip, &at).map_err(|e| e.to_string())?;
            Ok(json!({ "cuts": made, "scene_changes": listed }).to_string())
        }
        "mark_beats" => {
            let sound = sound_clip(editor, args)?;
            let waveform = waveform_of(editor, sound.media_id)?;
            let (beats, bpm) = bettercut_playback::beat_markers(&sound, &waveform)
                .ok_or("no steady beat found in this clip")?;
            let added = editor.add_markers(&beats).map_err(|e| e.to_string())?;
            Ok(format!("{added} beat markers, about {bpm:.0} BPM"))
        }
        "remove_silences" => {
            let sound = sound_clip(editor, args)?;
            let defaults = bettercut_playback::SilenceSettings::default();
            let settings = bettercut_playback::SilenceSettings {
                threshold_db: optional_number(args, "threshold_db")?
                    .map_or(defaults.threshold_db, |db| db.clamp(-42.0, -6.0)),
                shortest: seconds(args, "shortest")?
                    .map_or(defaults.shortest, |s| timeline_time(s.max(0.1))),
                padding: seconds(args, "padding")?.map_or(defaults.padding, timeline_time),
            };
            let waveform = waveform_of(editor, sound.media_id)?;
            let ranges = bettercut_playback::silent_ranges(&sound, &waveform, settings);
            let listed: Vec<Value> = ranges
                .iter()
                .map(|r| {
                    json!({
                        "start": r.start.as_seconds_f64(),
                        "end": r.end.as_seconds_f64(),
                    })
                })
                .collect();
            if args
                .get("preview")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(json!({ "pauses": listed }).to_string());
            }
            if ranges.is_empty() {
                return Err("no pauses long and quiet enough to cut".to_owned());
            }
            let cut = editor
                .remove_ranges(sound.id, &ranges)
                .map_err(|e| e.to_string())?;
            let seconds: f64 = ranges
                .iter()
                .map(|r| (r.end - r.start).as_seconds_f64())
                .sum();
            Ok(format!("{cut} pauses cut, {seconds:.1} s shorter"))
        }
        "enhance_voice" => {
            let clip = sound_clip(editor, args)?;
            editor.enhance_voice(clip.id).map_err(|e| e.to_string())?;
            Ok("Voice enhanced".to_owned())
        }
        "mute_clip" => {
            let clip = clip_arg(args)?;
            let muted = args.get("muted").and_then(Value::as_bool).unwrap_or(true);
            let changed = editor.set_muted(clip, muted).map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("that clip has no sound to mute, or is already so".to_owned());
            }
            Ok(if muted { "Muted" } else { "Unmuted" }.to_owned())
        }
        "set_playhead" => {
            let at = time_arg(args, "at")?;
            editor.set_playhead(at);
            Ok(format!(
                "Playhead at {:.3} s",
                editor.playhead().as_seconds_f64()
            ))
        }
        "get_selection" | "select_clips" => {
            Err("only the app's window has a selection: attach_to_app first".to_owned())
        }
        "add_colour" => {
            let top = colour_arg(str_arg(args, "color")?)?;
            let bottom = match args.get("to").and_then(Value::as_str) {
                Some(to) => colour_arg(to)?,
                None => top,
            };
            editor.set_playhead(time_arg(args, "at")?);
            let clip = editor
                .add_colour_clip(bettercut_editor_core::media::Generated::Colour {
                    top: [top.r, top.g, top.b],
                    bottom: [bottom.r, bottom.g, bottom.b],
                })
                .map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": clip.to_string() }).to_string())
        }
        "freeze_frame" => {
            let clip = clip_arg(args)?;
            let duration = timeline_time(seconds(args, "duration")?.unwrap_or(2.0).max(0.04));
            editor.set_playhead(time_arg(args, "at")?);
            let held = editor
                .freeze_frame(clip, duration)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": held.to_string() }).to_string())
        }
        "picture_in_picture" => {
            use bettercut_editor_core::{PipCorner, PipSize};
            let clip = clip_arg(args)?;
            if editor.video_clip(clip).is_none() {
                return Err("that is not a picture clip".to_owned());
            }
            let size = args.get("size").and_then(Value::as_str).unwrap_or("medium");
            if plain(size) == "full" {
                editor
                    .reset_to_full_frame(clip)
                    .map_err(|e| e.to_string())?;
                return Ok("Full frame".to_owned());
            }
            let size = one_of(&PipSize::ALL, PipSize::label, size, "size")?;
            let corner = match args.get("corner").and_then(Value::as_str) {
                None => PipCorner::BottomRight,
                Some(name) => one_of(&PipCorner::ALL, PipCorner::label, name, "corner")?,
            };
            editor
                .apply_picture_in_picture(clip, corner, size)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "{} inset, {}",
                size.label(),
                corner.label().to_lowercase()
            ))
        }
        "copy_as_shape" => {
            let wanted = str_arg(args, "shape")?;
            let shape = bettercut_editor_core::SHAPES
                .into_iter()
                .find(|s| plain(s.label) == plain(wanted) || plain(s.file_suffix) == plain(wanted))
                .ok_or_else(|| {
                    let labels: Vec<&str> = bettercut_editor_core::SHAPES
                        .iter()
                        .map(|s| s.label)
                        .collect();
                    format!("no shape {wanted:?}; one of {}", labels.join(", "))
                })?;
            let from = editor
                .active_sequence()
                .map(|s| s.id)
                .ok_or("the project has no sequence")?;
            let copy = editor
                .copy_sequence_as(from, shape)
                .map_err(|e| e.to_string())?;
            editor.switch_sequence(copy);
            Ok(json!({ "sequence_id": copy.to_string(), "shape": shape.label }).to_string())
        }
        "switch_sequence" => {
            let id = id_arg(args, "sequence_id")?;
            let target = editor
                .sequence_list()
                .into_iter()
                .find(|(s, _)| s.to_string() == id.to_string())
                .ok_or("no sequence with that id")?;
            editor.switch_sequence(target.0);
            Ok(format!("Working on {}", target.1))
        }
        "add_lower_third" => {
            let name = str_arg(args, "name")?.to_owned();
            let role = args
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let accent = match args.get("accent").and_then(Value::as_str) {
                Some(colour) => {
                    let c = colour_arg(colour)?;
                    [c.r, c.g, c.b]
                }
                None => bettercut_editor_core::lower_third::LOWER_THIRD_ACCENT,
            };
            editor.set_playhead(time_arg(args, "at")?);
            let clips = editor
                .add_lower_third(&name, &role, accent)
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "clip_ids": clips.iter().map(ToString::to_string).collect::<Vec<_>>() })
                    .to_string(),
            )
        }
        "add_shape" => {
            use bettercut_editor_core::text::ShapeKind;
            let kind = one_of(
                &ShapeKind::ALL,
                ShapeKind::label,
                str_arg(args, "shape")?,
                "shape",
            )?;
            editor.set_playhead(time_arg(args, "at")?);
            let clip = editor.add_shape(kind).map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": clip.to_string() }).to_string())
        }
        "add_sticker" => {
            use bettercut_editor_core::stickers::STICKERS;
            let wanted = str_arg(args, "sticker")?;
            let (sticker, _) = STICKERS
                .iter()
                .find(|(glyph, name)| *glyph == wanted.trim() || plain(name) == plain(wanted))
                .ok_or_else(|| {
                    let names: Vec<&str> = STICKERS.iter().map(|(_, name)| *name).collect();
                    format!("no sticker {wanted:?}; one of {}", names.join(", "))
                })?;
            editor.set_playhead(time_arg(args, "at")?);
            let clip = editor.add_sticker(sticker).map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": clip.to_string() }).to_string())
        }
        "add_timer" => {
            use bettercut_editor_core::timeline::CountDirection;
            let direction = match args.get("direction").and_then(Value::as_str) {
                None | Some("down") => CountDirection::Down,
                Some("up") => CountDirection::Up,
                Some(other) => return Err(format!("direction is down or up, not {other:?}")),
            };
            editor.set_playhead(time_arg(args, "at")?);
            let clip = editor.add_counter(direction).map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": clip.to_string() }).to_string())
        }
        "add_marker" => {
            let label = args.get("label").and_then(Value::as_str).unwrap_or("");
            let at = editor
                .add_named_marker(time_arg(args, "at")?, label)
                .map_err(|e| e.to_string())?;
            Ok(format!("Marker at {:.3} s", at.as_seconds_f64()))
        }
        "add_title" => {
            let text = str_arg(args, "text")?.to_owned();
            let at = time_arg(args, "at")?;
            editor.set_playhead(at);
            let clip = editor.add_text(text).map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": clip.to_string() }).to_string())
        }
        "split_clip" => {
            let clip = clip_arg(args)?;
            let at = time_arg(args, "at")?;
            let made = editor
                .split_clip_at(clip, &[at])
                .map_err(|e| e.to_string())?;
            if made == 0 {
                return Err("that time is not inside the clip".to_owned());
            }
            Ok("Split".to_owned())
        }
        "delete_clip" => delete_clip(editor, args),
        "set_volume" => {
            let clip = clip_arg(args)?;
            let volume = number(args, "volume")?;
            if editor.audio_clip(clip).is_none() {
                return Err("that is not a sound clip: use an id from sound_lanes".to_owned());
            }
            editor
                .set_clip_property(clip, ClipProperty::Gain(volume as f32), false)
                .map_err(|e| e.to_string())?;
            Ok(format!("Volume {volume}"))
        }
        "set_opacity" => {
            let clip = clip_arg(args)?;
            let opacity = number(args, "opacity")?;
            editor
                .set_clip_property(clip, ClipProperty::Opacity(opacity as f32), false)
                .map_err(|e| e.to_string())?;
            Ok(format!("Opacity {opacity}"))
        }
        "set_speed" => {
            let clip = clip_arg(args)?;
            let speed = number(args, "speed")?;
            let rate = bettercut_editor_core::foundation::Rational::new(
                (speed * 100.0).round() as i64,
                100,
            )
            .filter(|_| speed > 0.0)
            .ok_or("speed must be more than 0")?;
            editor
                .set_clip_speed(clip, rate, false)
                .map_err(|e| e.to_string())?;
            Ok(format!("Speed {speed}x"))
        }
        "reverse_clip" => {
            let clip = clip_arg(args)?;
            let on = args
                .get("reversed")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            editor.set_reversed(clip, on).map_err(|e| e.to_string())?;
            Ok(if on { "Reversed" } else { "Forwards" }.to_owned())
        }
        "set_fades" => {
            let clip = clip_arg(args)?;
            let fade_in = seconds(args, "fade_in")?.unwrap_or(0.0);
            let fade_out = seconds(args, "fade_out")?.unwrap_or(0.0);
            editor
                .set_clip_fades(clip, timeline_time(fade_in), timeline_time(fade_out), false)
                .map_err(|e| e.to_string())?;
            Ok("Fades set".to_owned())
        }
        "add_transition" => {
            let clip = clip_arg(args)?;
            let kind = transition_named(str_arg(args, "kind")?)?;
            let duration = seconds(args, "duration")?;
            editor
                .set_transition(clip, kind)
                .map_err(|e| e.to_string())?;
            if let Some(duration) = duration {
                editor
                    .set_transition_duration(clip, timeline_time(duration))
                    .map_err(|e| e.to_string())?;
            }
            Ok(format!("{} added", kind.label()))
        }
        "apply_filter" => {
            let filter = filter_named(str_arg(args, "filter")?)?;
            let clips = args
                .get("clip_ids")
                .and_then(Value::as_array)
                .ok_or("clip_ids must be a list")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .and_then(|t| uuid::Uuid::parse_str(t).ok())
                        .map(ClipId::from_uuid)
                        .ok_or_else(|| "clip_ids holds something that is not an id".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let took = editor
                .apply_filter(filter, clips)
                .map_err(|e| e.to_string())?;
            if took == 0 {
                return Err("none of those are picture clips".to_owned());
            }
            Ok(format!("{} on {took} clip(s)", filter.label()))
        }
        "move_clip" => {
            let clip = clip_arg(args)?;
            let start = time_arg(args, "start")?;
            let track = editor.track_of(clip).ok_or("no clip with that id")?;
            editor
                .move_clip(track, track, clip, start)
                .map_err(|e| e.to_string())?;
            Ok("Moved".to_owned())
        }
        "trim_clip" => {
            let clip = clip_arg(args)?;
            let edge = match str_arg(args, "edge")? {
                "start" => TrimEdge::Start,
                "end" => TrimEdge::End,
                _ => return Err("edge is start or end".to_owned()),
            };
            let to = time_arg(args, "to")?;
            let track = editor.track_of(clip).ok_or("no clip with that id")?;
            editor
                .trim_clip(track, clip, edge, to)
                .map_err(|e| e.to_string())?;
            Ok("Trimmed".to_owned())
        }
        "add_captions" => {
            use bettercut_editor_core::captions::{CaptionSegment, tidy};
            let lines = args
                .get("lines")
                .and_then(Value::as_array)
                .ok_or("lines must be a list of {start, end, text}")?;
            let segments = lines
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    let text = line
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("line {} has no text", i + 1))?;
                    Ok(CaptionSegment {
                        start: time_arg(line, "start")?,
                        end: time_arg(line, "end")?,
                        text: text.to_owned(),
                        words: Vec::new(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let given = segments.len();
            let segments = tidy(segments);
            if segments.is_empty() {
                return Err(
                    "none of those lines can be shown: each needs words and an end after its start"
                        .to_owned(),
                );
            }
            let count = editor
                .replace_captions(segments, format!("Add {given} Captions"))
                .map_err(|e| e.to_string())?;
            Ok(if count < given {
                format!(
                    "{count} captions added; {} could not be shown",
                    given - count
                )
            } else {
                format!("{count} captions added")
            })
        }
        "import_captions" => {
            let path = path_arg(args, "path")?;
            let count = editor.import_captions(&path).map_err(|e| e.to_string())?;
            Ok(format!("{count} captions imported"))
        }
        "undo" => {
            editor.undo().map_err(|e| e.to_string())?;
            Ok("Undone".to_owned())
        }
        "redo" => {
            editor.redo().map_err(|e| e.to_string())?;
            Ok("Redone".to_owned())
        }
        "export" => export_project(editor.project(), args),
        other => Err(format!("no tool {other}")),
    }
}

fn import_media(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let paths = args
        .get("paths")
        .and_then(Value::as_array)
        .ok_or("paths must be a list of files")?;
    let results: Vec<Value> = paths
        .iter()
        .map(|path| {
            let Some(path) = path.as_str() else {
                return json!({ "error": "not a path" });
            };
            match editor.import_file(Path::new(path)) {
                Ok(id) => json!({ "path": path, "media_id": id.to_string() }),
                Err(err) => json!({ "path": path, "error": err.to_string() }),
            }
        })
        .collect();
    Ok(Value::Array(results).to_string())
}

fn add_to_timeline(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let media = id_arg(args, "media_id").map(MediaId::from_uuid)?;
    let range = match (seconds(args, "from")?, seconds(args, "to")?) {
        (None, None) => None,
        (from, to) => Some((
            media_time(from.unwrap_or(0.0)),
            media_time(to.unwrap_or(f64::MAX / 2.0)),
        )),
    };
    let clips = editor
        .place_media_range(media, range)
        .map_err(|e| e.to_string())?;
    Ok(
        json!({ "clip_ids": clips.iter().map(ToString::to_string).collect::<Vec<_>>() })
            .to_string(),
    )
}

fn delete_clip(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let clip = clip_arg(args)?;
    let ripple = args.get("ripple").and_then(Value::as_bool).unwrap_or(false);
    if editor.text_clip(clip).is_some() {
        editor.remove_text(clip).map_err(|e| e.to_string())?;
        return Ok("Deleted the title".to_owned());
    }
    let track = editor.track_of(clip).ok_or("no clip with that id")?;
    if ripple {
        editor.ripple_delete(track, clip)
    } else {
        editor.remove_clip(track, clip)
    }
    .map_err(|e| e.to_string())?;
    Ok("Deleted".to_owned())
}

fn preview_frame(editor: &mut Editor, args: &Value) -> Result<Reply, String> {
    let at = time_arg(args, "at")?;
    let max_width = u32_arg(args, "max_width")?.unwrap_or(960);
    let sequence = editor
        .active_sequence()
        .ok_or("the project has no sequence")?;
    let (size, rgba) = bettercut_export::render_still(
        editor.project(),
        sequence,
        at,
        &bettercut_editor_core::media::NeverCancelled,
    )
    .map_err(|e| format!("could not render that frame: {e}"))?;
    // Smaller for the assistant, keeping the shape.
    let (size, rgba) = if size.width > max_width {
        let height =
            (u64::from(size.height) * u64::from(max_width) / u64::from(size.width)).max(1) as u32;
        let to = bettercut_editor_core::timeline::Resolution::new(max_width, height);
        (to, bettercut_export::shrink(&rgba, size, to))
    } else {
        (size, rgba)
    };
    let file = std::env::temp_dir().join(format!("bettercut-mcp-frame-{}.png", std::process::id()));
    bettercut_export::write_png(&file, size, &rgba).map_err(|e| e.to_string())?;
    let png = std::fs::read(&file).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&file);
    Ok(Reply::Image {
        png: png?,
        caption: format!(
            "The frame at {:.3} s, {}x{}",
            at.as_seconds_f64(),
            size.width,
            size.height
        ),
    })
}

/// Render `project`'s timeline as the export tool asks. Takes the project
/// alone so the app can export a copy away from its window's thread.
pub fn export_project(project: &Project, args: &Value) -> Result<String, String> {
    let path = path_arg(args, "path")?;
    let width = u32_arg(args, "width")?;
    let height = u32_arg(args, "height")?;
    let sequence = project.active().ok_or("the project has no sequence")?;
    if sequence.duration().ticks() <= 0 {
        return Err("The timeline is empty: there is nothing to export".to_owned());
    }
    let mut settings = bettercut_export::ExportSettings::for_sequence(path.clone(), sequence);
    if let (Some(w), Some(h)) = (width, height) {
        settings.resolution = bettercut_editor_core::timeline::Resolution::new(w, h);
    }
    let summary = bettercut_export::export(
        project,
        sequence,
        &settings,
        &mut |_| {},
        &bettercut_editor_core::media::NeverCancelled,
    )
    .map_err(|e| format!("export failed: {e}"))?;
    Ok(json!({
        "path": summary.path.display().to_string(),
        "frames": summary.frames,
        "encoder": summary.encoder,
    })
    .to_string())
}

/// A number argument that may be left out; when given, it must be finite.
fn optional_number(args: &Value, name: &str) -> Result<Option<f32>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| Some(v as f32))
            .ok_or_else(|| format!("{name} must be a number")),
    }
}

/// "#rrggbb" (or "rrggbb", or "#rrggbbaa") as a colour.
fn colour_arg(text: &str) -> Result<bettercut_editor_core::text::Rgba, String> {
    let hex = text.trim().trim_start_matches('#');
    let byte = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
    };
    let bad = || format!("{text:?} is not a colour: write it as \"#rrggbb\"");
    match hex.len() {
        6 | 8 => Ok(bettercut_editor_core::text::Rgba {
            r: byte(0).ok_or_else(bad)?,
            g: byte(2).ok_or_else(bad)?,
            b: byte(4).ok_or_else(bad)?,
            a: if hex.len() == 8 {
                byte(6).ok_or_else(bad)?
            } else {
                255
            },
        }),
        _ => Err(bad()),
    }
}

fn set_transform(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let clip = clip_arg(args)?;
    let x = optional_number(args, "x")?;
    let y = optional_number(args, "y")?;
    let scale = optional_number(args, "scale")?;
    let rotation = optional_number(args, "rotation")?;
    if scale.is_some_and(|s| s <= 0.0) {
        return Err("scale must be more than 0".to_owned());
    }
    let changed;
    if let Some(title) = editor.text_clip(clip) {
        let now = title.transform;
        let mut changes = Vec::new();
        if x.is_some() || y.is_some() {
            changes.push(TextProperty::Position {
                x: x.unwrap_or(now.position.x),
                y: y.unwrap_or(now.position.y),
            });
        }
        if let Some(scale) = scale {
            changes.push(TextProperty::Scale { x: scale, y: scale });
        }
        if let Some(rotation) = rotation {
            changes.push(TextProperty::Rotation(rotation));
        }
        changed = !changes.is_empty();
        editor
            .set_text_properties(clip, changes, "Place Title")
            .map_err(|e| e.to_string())?;
    } else {
        let now = editor
            .video_clip(clip)
            .ok_or("that is not a picture clip or a title: sound has no place on screen")?
            .transform;
        let mut changes = Vec::new();
        if x.is_some() || y.is_some() {
            changes.push(ClipProperty::Position {
                x: x.unwrap_or(now.position.x),
                y: y.unwrap_or(now.position.y),
            });
        }
        if let Some(scale) = scale {
            changes.push(ClipProperty::Scale { x: scale, y: scale });
        }
        if let Some(rotation) = rotation {
            changes.push(ClipProperty::Rotation(rotation));
        }
        changed = !changes.is_empty();
        editor
            .set_clip_properties(clip, changes, "Place Clip")
            .map_err(|e| e.to_string())?;
    }
    if !changed {
        return Err("nothing to change: give x, y, scale or rotation".to_owned());
    }
    Ok("Placed".to_owned())
}

fn style_title(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::text::{Background, FontWeight, Stroke};
    let clip = clip_arg(args)?;
    let title = editor
        .text_clip(clip)
        .ok_or("that is not a title: use an id from title_lanes")?;
    let mut style = title.style.clone();
    let before = style.clone();
    if let Some(size) = optional_number(args, "size")? {
        style.size = size.clamp(4.0, 1000.0);
    }
    if let Some(color) = args.get("color").and_then(Value::as_str) {
        style.color = colour_arg(color)?;
    }
    if let Some(bold) = args.get("bold").and_then(Value::as_bool) {
        style.weight = if bold {
            FontWeight::Bold
        } else {
            FontWeight::Regular
        };
    }
    if let Some(italic) = args.get("italic").and_then(Value::as_bool) {
        style.italic = italic;
    }
    if let Some(outline) = args.get("outline").and_then(Value::as_str) {
        style.stroke = if outline.eq_ignore_ascii_case("none") {
            None
        } else {
            let color = colour_arg(outline)?;
            Some(Stroke {
                color,
                ..style.stroke.unwrap_or_default()
            })
        };
    }
    if let Some(background) = args.get("background").and_then(Value::as_str) {
        style.background = if background.eq_ignore_ascii_case("none") {
            None
        } else {
            let color = colour_arg(background)?;
            Some(Background {
                color,
                ..style.background.unwrap_or_default()
            })
        };
    }
    let mut changes = Vec::new();
    if let Some(text) = args.get("text").and_then(Value::as_str) {
        changes.push(TextProperty::Content(text.to_owned()));
    }
    if style != before {
        changes.push(TextProperty::Style(Box::new(style)));
    }
    if changes.is_empty() {
        return Err("nothing to change".to_owned());
    }
    editor
        .set_text_properties(clip, changes, "Style Title")
        .map_err(|e| e.to_string())?;
    Ok("Styled".to_owned())
}

fn adjust_colour(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let clip = clip_arg(args)?;
    if editor.video_clip(clip).is_none() {
        return Err("that is not a picture clip".to_owned());
    }
    let changes: Vec<ClipProperty> = [
        (
            "brightness",
            ClipProperty::Brightness as fn(f32) -> ClipProperty,
            0.0,
            2.0,
        ),
        ("contrast", ClipProperty::Contrast, 0.0, 2.0),
        ("saturation", ClipProperty::Saturation, 0.0, 2.0),
        ("temperature", ClipProperty::Temperature, -1.0, 1.0),
    ]
    .into_iter()
    .filter_map(|(name, make, low, high)| {
        optional_number(args, name)
            .map(|v| v.map(|v| make(v.clamp(low, high))))
            .transpose()
    })
    .collect::<Result<_, _>>()?;
    if changes.is_empty() {
        return Err(
            "nothing to change: give brightness, contrast, saturation or temperature".to_owned(),
        );
    }
    editor
        .set_clip_properties(clip, changes, "Adjust Colour")
        .map_err(|e| e.to_string())?;
    Ok("Colour adjusted".to_owned())
}

/// `name` as one of `all`, matched the forgiving way names are here; "none"
/// is left to the caller.
fn one_of<T: Copy + std::fmt::Debug>(
    all: &[T],
    label: fn(T) -> &'static str,
    name: &str,
    what: &str,
) -> Result<T, String> {
    let wanted = plain(name);
    all.iter()
        .copied()
        .find(|v| plain(label(*v)) == wanted || plain(&format!("{v:?}")) == wanted)
        .ok_or_else(|| {
            let names: Vec<&str> = all.iter().map(|v| label(*v)).collect();
            format!("no {what} {name:?}; one of {}", names.join(", "))
        })
}

fn animate_title(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::timeline::{LoopMotion, Motion, MotionKind};
    let clip = clip_arg(args)?;
    let mut animation = editor
        .text_clip(clip)
        .ok_or("that is not a title: use an id from title_lanes")?
        .animation;
    let before = animation;
    let duration = timeline_time(seconds(args, "duration")?.unwrap_or(0.5).max(0.05));
    let motion = |name: &str| -> Result<Option<Motion>, String> {
        if plain(name) == "none" {
            return Ok(None);
        }
        one_of(&MotionKind::FOR_TEXT, MotionKind::label, name, "animation")
            .map(|kind| Some(Motion::new(kind, duration)))
    };
    if let Some(name) = args.get("intro").and_then(Value::as_str) {
        animation.intro = motion(name)?;
    }
    if let Some(name) = args.get("outro").and_then(Value::as_str) {
        animation.outro = motion(name)?;
    }
    if let Some(name) = args.get("looping").and_then(Value::as_str) {
        animation.looping = if plain(name) == "none" {
            None
        } else {
            Some(one_of(&LoopMotion::ALL, LoopMotion::label, name, "loop")?)
        };
    }
    if animation == before {
        return Err("nothing to change: give intro, outro or looping".to_owned());
    }
    editor
        .set_text_property(clip, TextProperty::Animation(animation), false)
        .map_err(|e| e.to_string())?;
    Ok("Animated".to_owned())
}

/// The properties `animate` takes, by name: what each drives, and whether it
/// is a sound clip's.
const ANIMATED: &[(&str, &[AnimatedParameter], bool)] = &[
    ("opacity", &[AnimatedParameter::Opacity], false),
    ("x", &[AnimatedParameter::PositionX], false),
    ("y", &[AnimatedParameter::PositionY], false),
    (
        "scale",
        &[AnimatedParameter::ScaleX, AnimatedParameter::ScaleY],
        false,
    ),
    ("rotation", &[AnimatedParameter::Rotation], false),
    ("brightness", &[AnimatedParameter::Brightness], false),
    ("contrast", &[AnimatedParameter::Contrast], false),
    ("saturation", &[AnimatedParameter::Saturation], false),
    ("temperature", &[AnimatedParameter::Temperature], false),
    ("blur", &[AnimatedParameter::Blur], false),
    ("volume", &[AnimatedParameter::Gain], true),
    ("pan", &[AnimatedParameter::Pan], true),
];

fn animate(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::timeline::{Interpolation, Keyframe};
    let clip = clip_arg(args)?;
    let name = str_arg(args, "property")?;
    let wanted = plain(name);
    let (label, parameters, sound) = ANIMATED
        .iter()
        .find(|(label, _, _)| *label == wanted)
        .ok_or_else(|| {
            let names: Vec<&str> = ANIMATED.iter().map(|(label, _, _)| *label).collect();
            format!(
                "no property {name:?} to animate; one of {}",
                names.join(", ")
            )
        })?;
    // Where the clip is, and how its timeline maps to the source time keys
    // are kept in.
    let (range, to_source): (_, Box<dyn Fn(TimelineTime) -> MediaTime>) = if *sound {
        let sound = editor
            .audio_clip(clip)
            .ok_or_else(|| format!("{label} is a sound clip's: use an id from sound_lanes"))?
            .clone();
        (sound.timeline, Box::new(move |t| sound.source_time_at(t)))
    } else {
        let picture = editor
            .video_clip(clip)
            .ok_or_else(|| format!("{label} is a picture clip's: use an id from picture_lanes"))?
            .clone();
        (
            picture.timeline,
            Box::new(move |t| picture.source_time_at(t)),
        )
    };
    let easings = [
        Interpolation::Linear,
        Interpolation::Hold,
        Interpolation::EaseIn,
        Interpolation::EaseOut,
        Interpolation::EaseInOut,
    ];
    let keys = args
        .get("keys")
        .and_then(Value::as_array)
        .ok_or("keys must be a list of {at, value}")?
        .iter()
        .map(|key| {
            let at = time_arg(key, "at")?;
            if !range.contains(at) {
                return Err(format!(
                    "a key at {:.3} s is outside the clip, which runs {:.3}–{:.3} s",
                    at.as_seconds_f64(),
                    range.start.as_seconds_f64(),
                    range.end.as_seconds_f64()
                ));
            }
            let value = key
                .get("value")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or("each key needs a number value")? as f32;
            let easing = match key.get("easing").and_then(Value::as_str) {
                None => Interpolation::Linear,
                Some(easing) => one_of(&easings, Interpolation::label, easing, "easing")?,
            };
            Ok(Keyframe::new(to_source(at), value, easing))
        })
        .collect::<Result<Vec<_>, String>>()?;
    editor
        .replace_keyframes(clip, parameters, &keys, &format!("Animate {label}"))
        .map_err(|e| e.to_string())?;
    Ok(if keys.is_empty() {
        format!("{label} no longer animated")
    } else {
        format!("{label} animated with {} keys", keys.len())
    })
}

/// bettercut's own templates, then the person's.
fn templates() -> Vec<bettercut_editor_core::templates::Template> {
    use bettercut_editor_core::templates::{library, starters};
    let mut all = starters::starters();
    all.extend(library::load_dir(&library::user_dir()).templates);
    all
}

fn apply_template(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::SlotFill;
    use bettercut_editor_core::templates::SlotKind;
    let wanted = str_arg(args, "template_id")?;
    let template = templates()
        .into_iter()
        .find(|t| t.id == wanted || plain(&t.name) == plain(wanted))
        .ok_or_else(|| format!("no template {wanted:?}: list_templates names them"))?;
    let at = match seconds(args, "at")? {
        Some(at) => timeline_time(at),
        None => editor
            .active_sequence()
            .map(|s| s.duration())
            .unwrap_or_default(),
    };
    let mut fills = std::collections::HashMap::new();
    if let Some(given) = args.get("fills").and_then(Value::as_object) {
        for (slot_id, value) in given {
            let slot = template.slot(slot_id).ok_or_else(|| {
                let ids: Vec<&str> = template.slots.iter().map(|s| s.id.as_str()).collect();
                format!(
                    "{} has no slot {slot_id:?}; its slots are {}",
                    template.name,
                    ids.join(", ")
                )
            })?;
            let value = value
                .as_str()
                .ok_or_else(|| format!("the fill for {slot_id} must be a string"))?;
            let fill = if slot.kind == SlotKind::Text {
                SlotFill::Text(value.to_owned())
            } else {
                let id = uuid::Uuid::parse_str(value)
                    .map_err(|_| format!("the fill for {slot_id} must be a media_id"))?;
                SlotFill::Media(MediaId::from_uuid(id))
            };
            fills.insert(slot_id.clone(), fill);
        }
    }
    let applied = editor
        .apply_template(&template, &fills, at)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "template": template.name,
        "clip_ids": applied.clips.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "unfilled_slots": applied.unfilled,
        "shortened_slots": applied.shortened,
    })
    .to_string())
}

/// The sound clip `clip_id` names, or why not. A picture clip's id gives
/// its linked sound, since that is what an assistant means.
fn sound_clip(
    editor: &Editor,
    args: &Value,
) -> Result<bettercut_editor_core::timeline::AudioClip, String> {
    let id = clip_arg(args)?;
    if let Some(sound) = editor.audio_clip(id) {
        return Ok(sound.clone());
    }
    editor
        .linked_with(id)
        .into_iter()
        .find_map(|other| editor.audio_clip(other).cloned())
        .ok_or_else(|| "that clip has no sound: use an id from sound_lanes".to_owned())
}

/// A file's waveform: the app's cached one when it has made it, otherwise
/// read now. Not written to the app's cache: that is the app's to fill.
fn waveform_of(editor: &Editor, media: MediaId) -> Result<bettercut_cache::Waveform, String> {
    let file = bettercut_cache::CacheLayout::default_location().waveform_file(media);
    if let Ok(waveform) = bettercut_cache::Waveform::read(&file) {
        return Ok(waveform);
    }
    let asset = editor
        .project()
        .media
        .iter()
        .find(|m| m.id == media)
        .ok_or("a clip's file is not in the project")?;
    bettercut_playback::analyse_waveform(asset, 2)
        .map_err(|e| format!("could not read the sound of {}: {e}", asset.file_name))
}

/// What is selected in the window, for `get_selection`.
pub fn selection(editor: &Editor, selected: &[ClipId]) -> Value {
    let clips: Vec<Value> = selected
        .iter()
        .map(|id| {
            let (kind, name) = if let Some(c) = editor.video_clip(*id) {
                ("picture", media_name(editor, c.media_id))
            } else if let Some(c) = editor.audio_clip(*id) {
                ("sound", media_name(editor, c.media_id))
            } else if let Some(c) = editor.text_clip(*id) {
                ("title", c.text.clone())
            } else {
                ("other", String::new())
            };
            json!({ "clip_id": id.to_string(), "kind": kind, "name": name })
        })
        .collect();
    json!({
        "selected": clips,
        "playhead": editor.playhead().as_seconds_f64(),
    })
}

fn media_name(editor: &Editor, media: MediaId) -> String {
    editor
        .project()
        .media
        .iter()
        .find(|m| m.id == media)
        .map_or_else(String::new, |m| m.display_name().to_owned())
}

/// `clip_ids` as ids, for `select_clips` and `apply_filter`.
pub fn clip_ids(args: &Value) -> Result<Vec<ClipId>, String> {
    args.get("clip_ids")
        .and_then(Value::as_array)
        .ok_or("clip_ids must be a list")?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|t| uuid::Uuid::parse_str(t).ok())
                .map(ClipId::from_uuid)
                .ok_or_else(|| "clip_ids holds something that is not an id".to_owned())
        })
        .collect()
}

/// Which of `animate`'s properties have keys on `clip`.
fn animated_names(editor: &Editor, clip: ClipId) -> Vec<&'static str> {
    ANIMATED
        .iter()
        .filter(|(_, parameters, _)| {
            parameters
                .iter()
                .any(|p| !editor.keyframes_of(clip, *p).is_empty())
        })
        .map(|(label, _, _)| *label)
        .collect()
}

/// Where a clip sits, in `set_transform`'s terms.
fn place(transform: &bettercut_editor_core::timeline::Transform) -> Value {
    json!({
        "x": transform.position.x,
        "y": transform.position.y,
        "scale": transform.scale.x,
        "rotation": transform.rotation_degrees,
    })
}

/// The project as an assistant needs to see it to edit it.
fn describe(editor: &Editor) -> Value {
    let project = editor.project();
    let media: Vec<Value> = project
        .media
        .iter()
        .map(|m| {
            json!({
                "media_id": m.id.to_string(),
                "name": m.display_name(),
                "kind": format!("{:?}", m.kind).to_lowercase(),
                "duration": seconds_of(m.duration.ticks()),
                "width": m.width,
                "height": m.height,
            })
        })
        .collect();
    let Some(sequence) = editor.active_sequence() else {
        return json!({ "project": project.name, "media": media, "sequence": null });
    };
    let name_of = |id: MediaId| {
        project
            .media_asset(id)
            .map_or("(missing)".to_owned(), |m| m.display_name().to_owned())
    };
    let span = |start: TimelineTime, end: TimelineTime| {
        (seconds_of(start.ticks()), seconds_of(end.ticks()))
    };
    let video: Vec<Value> = sequence
        .video_tracks
        .iter()
        .map(|track| {
            json!({
                "lane": track.name,
                "clips": track.clips().iter().map(|c| {
                    let (start, end) = span(c.timeline.start, c.timeline.end);
                    json!({ "clip_id": c.id.to_string(), "start": start, "end": end,
                            "media": name_of(c.media_id),
                            "speed": c.speed.as_f64(), "reversed": c.reversed,
                            "opacity": c.opacity, "place": place(&c.transform),
                            "animated": animated_names(editor, c.id),
                            "movement": editor.movement_of(c.id)
                                .filter(|m| *m != Movement::None).map(Movement::label),
                            "filter": editor.filter_of(c.id).map(|f| f.label()),
                            "transition": c.transition_out.map(|t| json!({
                                "kind": t.kind.label(),
                                "duration": seconds_of(t.duration.ticks()),
                            })) })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let audio: Vec<Value> = sequence
        .audio_tracks
        .iter()
        .map(|track| {
            json!({
                "lane": track.name,
                "clips": track.clips().iter().map(|c| {
                    let (start, end) = span(c.timeline.start, c.timeline.end);
                    json!({ "clip_id": c.id.to_string(), "start": start, "end": end,
                            "media": name_of(c.media_id),
                            "volume": c.gain, "speed": c.speed.as_f64(), "reversed": c.reversed,
                            "animated": animated_names(editor, c.id) })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let titles: Vec<Value> = sequence
        .text_tracks
        .iter()
        .map(|track| {
            json!({
                "lane": track.name,
                "clips": track.clips().iter().map(|c| {
                    let (start, end) = span(c.timeline.start, c.timeline.end);
                    json!({ "clip_id": c.id.to_string(), "start": start, "end": end,
                            "text": c.text, "place": place(&c.transform),
                            "size": c.style.size,
                            "intro": c.animation.intro.map(|m| m.kind.label()),
                            "outro": c.animation.outro.map(|m| m.kind.label()),
                            "looping": c.animation.looping.map(|m| m.label()),
                            "color": format!("#{:02x}{:02x}{:02x}",
                                c.style.color.r, c.style.color.g, c.style.color.b) })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let rate = sequence.frame_rate.as_rational();
    json!({
        "project": project.name,
        "path": editor.path().map(|p| p.display().to_string()),
        "sequence": {
            "name": sequence.name,
            "width": sequence.resolution.width,
            "height": sequence.resolution.height,
            "fps": rate.num() as f64 / rate.den() as f64,
            "duration": seconds_of(sequence.duration().ticks()),
        },
        "media": media,
        "picture_lanes": video,
        "sound_lanes": audio,
        "title_lanes": titles,
        "sequences": editor.sequence_list().into_iter().map(|(id, name)| json!({
            "sequence_id": id.to_string(), "name": name,
        })).collect::<Vec<_>>(),
        "markers": sequence.markers.iter().map(|m| json!({
            "at": seconds_of(m.time.ticks()), "label": m.label,
        })).collect::<Vec<_>>(),
    })
}

fn seconds_of(ticks: i64) -> f64 {
    (TimelineTime::from_ticks(ticks).as_seconds_f64() * 1000.0).round() / 1000.0
}

fn str_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} is required"))
}

fn path_arg(args: &Value, name: &str) -> Result<PathBuf, String> {
    str_arg(args, name).map(PathBuf::from)
}

fn u32_arg(args: &Value, name: &str) -> Result<Option<u32>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("{name} must be a whole number")),
    }
}

fn seconds(args: &Value, name: &str) -> Result<Option<f64>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|s| s.is_finite() && *s >= 0.0)
            .map(Some)
            .ok_or_else(|| format!("{name} must be a number of seconds, 0 or more")),
    }
}

fn time_arg(args: &Value, name: &str) -> Result<TimelineTime, String> {
    let s = seconds(args, name)?.ok_or_else(|| format!("{name} is required"))?;
    Ok(TimelineTime::from_millis((s * 1000.0).round() as i64))
}

fn media_time(s: f64) -> MediaTime {
    MediaTime::from_millis((s.min(1e9) * 1000.0).round() as i64)
}

fn number(args: &Value, name: &str) -> Result<f64, String> {
    seconds(args, name)?.ok_or_else(|| format!("{name} is required"))
}

fn timeline_time(s: f64) -> TimelineTime {
    TimelineTime::from_millis((s.min(1e9) * 1000.0).round() as i64)
}

/// A name as an assistant might write it — "Black and white", "B&W",
/// "fade-through-black" — reduced to letters and digits for matching.
fn plain(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn transition_names() -> Vec<&'static str> {
    TransitionKind::ALL.iter().map(|k| k.label()).collect()
}

fn filter_names() -> Vec<&'static str> {
    Filter::ALL.iter().map(|f| f.label()).collect()
}

fn transition_named(name: &str) -> Result<TransitionKind, String> {
    let wanted = plain(name);
    TransitionKind::ALL
        .into_iter()
        .find(|k| plain(k.label()) == wanted || plain(&format!("{k:?}")) == wanted)
        .ok_or_else(|| {
            format!(
                "no transition {name}; one of {}",
                transition_names().join(", ")
            )
        })
}

fn filter_named(name: &str) -> Result<Filter, String> {
    let wanted = plain(name);
    Filter::ALL
        .into_iter()
        .find(|f| plain(f.label()) == wanted || plain(&format!("{f:?}")) == wanted)
        .ok_or_else(|| format!("no filter {name}; one of {}", filter_names().join(", ")))
}

fn id_arg(args: &Value, name: &str) -> Result<uuid::Uuid, String> {
    let text = str_arg(args, name)?;
    uuid::Uuid::parse_str(text).map_err(|_| format!("{name} is not an id bettercut gave out"))
}

fn clip_arg(args: &Value) -> Result<ClipId, String> {
    id_arg(args, "clip_id").map(ClipId::from_uuid)
}
