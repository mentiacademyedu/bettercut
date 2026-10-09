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
use bettercut_editor_core::text::TextPreset;
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
    /// Exports running (or run) in the background, oldest first.
    exports: Vec<(String, std::sync::Arc<std::sync::Mutex<ExportState>>)>,
}

/// How a background export is going.
#[derive(Debug, Clone)]
enum ExportState {
    Running { done: u64, total: u64 },
    Finished(String),
    Failed(String),
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
        description: "Add a title (text over the picture) starting at `at` seconds. `style` \
                      gives it one of the Text tab's looks (Pop, Neon, Breaking, Comic ...); \
                      left out, it is white with a black outline.",
        schema: || {
            object(
                json!({
                    "text": { "type": "string", "minLength": 1 },
                    "at": { "type": "number", "minimum": 0 },
                    "style": { "type": "string", "enum": text_style_names() }
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
                      slide down, slide right, slide left, pop, bounce, spin, zoom out, swing, \
                      typewriter, or none; `duration` is each one's length in seconds (default 0.5). \
                      `looping` keeps it moving in between: pulse, wiggle, spin, float, blink, jelly, or \
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
        name: "animate_clip",
        description: "How a picture clip arrives and leaves: `intro` and `outro` are fade,                       slide up, slide down, slide right, slide left, pop, bounce, spin,                       zoom out, swing, or none; `duration` is each one's length in seconds                       (default 0.5). Leave out what should stay. One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "intro": { "type": "string" },
                    "outro": { "type": "string" },
                    "duration": { "type": "number", "exclusiveMinimum": 0 }
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
        name: "shape_sound",
        description: "Change how a clip's sound is coloured, in one undo step; only what you \
                      pass changes. `eq` is Voice, Phone, Radio, Megaphone, Rumble, Hum 50, \
                      Hum 60 or Flat; `voice` is normal, chipmunk or deep, or `pitch` in \
                      semitones (-12 to 12, speed unchanged); `space` is dry, echo, room or \
                      hall with `space_amount` 0-100 (default 30); `robot` 0-100. A picture \
                      clip's own sound is shaped.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "eq": { "type": "string" },
                    "voice": { "type": "string", "enum": ["normal", "chipmunk", "deep"] },
                    "pitch": { "type": "number", "minimum": -12, "maximum": 12 },
                    "space": { "type": "string", "enum": ["dry", "echo", "room", "hall"] },
                    "space_amount": { "type": "number", "minimum": 0, "maximum": 100 },
                    "robot": { "type": "number", "minimum": 0, "maximum": 100 }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "add_sound_effect",
        description: "Put a sound effect at `at` seconds, on a sound lane with room: Whoosh, \
                      Pop, Click, Ding, Chime, Boom, Shutter, Riser, Laser, Coin, Buzzer or \
                      Heartbeat. Made by bettercut, no file needed. Returns the new clip id.",
        schema: || {
            object(
                json!({
                    "effect": { "type": "string", "enum": sound_effect_names() },
                    "at": { "type": "number", "minimum": 0 }
                }),
                &["effect", "at"],
            )
        },
    },
    Tool {
        name: "organise_media",
        description: "Tidy one imported file in the project's media list, in one undo step: \
                      `rename` it (the file on disk keeps its name; an empty name goes back \
                      to the file's), and file it in a `bin` (an empty bin takes it out of \
                      one). describe_project shows names and bins.",
        schema: || {
            object(
                json!({
                    "media_id": { "type": "string" },
                    "rename": { "type": "string" },
                    "bin": { "type": "string" }
                }),
                &["media_id"],
            )
        },
    },
    Tool {
        name: "remove_unused_media",
        description: "Take every imported file that no clip uses out of the project's media \
                      list, in one undo step. The files stay on disk.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "set_lane",
        description: "Change a lane, named by `lane` (as describe_project lists it) or by one \
                      of its clips' `clip_id`, in one undo step: `rename` it, `locked` so no \
                      edit touches it, `on` false to hide a picture or title lane or silence \
                      a sound lane, `solo` to hear only it, `volume` (sound lanes, 1 as \
                      recorded).",
        schema: || {
            object(
                json!({
                    "lane": { "type": "string" },
                    "clip_id": { "type": "string" },
                    "rename": { "type": "string", "minLength": 1 },
                    "locked": { "type": "boolean" },
                    "on": { "type": "boolean" },
                    "solo": { "type": "boolean" },
                    "volume": { "type": "number", "minimum": 0 }
                }),
                &[],
            )
        },
    },
    Tool {
        name: "whole_video_look",
        description: "Settings over the whole video rather than one clip, in one undo step; \
                      only what you pass changes. `bars`: cinematic black bars cutting the \
                      picture to a shape (\"2.39\", \"1.85\", \"2\", \"2.76\" or none); \
                      `progress_bar`: none, thin or thick, a bar that fills as the video \
                      plays, with `progress_color` (\"#rrggbb\") and `progress_top`; \
                      `vignette` and `grain` 0-100; `background` (\"#rrggbb\"), what shows \
                      where no picture does.",
        schema: || {
            object(
                json!({
                    "bars": { "type": "string" },
                    "progress_bar": { "type": "string", "enum": ["none", "thin", "thick"] },
                    "progress_color": { "type": "string" },
                    "progress_top": { "type": "boolean" },
                    "vignette": { "type": "number", "minimum": 0, "maximum": 100 },
                    "grain": { "type": "number", "minimum": 0, "maximum": 100 },
                    "background": { "type": "string" }
                }),
                &[],
            )
        },
    },
    Tool {
        name: "remove_range",
        description: "Take everything between `from` and `to` seconds off every unlocked \
                      lane, cutting clips that cross either end. With `close_gap` (default \
                      true) what follows moves up, markers too; false leaves the hole. One \
                      undo step.",
        schema: || {
            object(
                json!({
                    "from": { "type": "number", "minimum": 0 },
                    "to": { "type": "number", "minimum": 0 },
                    "close_gap": { "type": "boolean" }
                }),
                &["from", "to"],
            )
        },
    },
    Tool {
        name: "hold_last_frame",
        description: "Hold a picture clip's last frame still for `duration` seconds (default \
                      2) straight after it, pushing what follows later — an ending that \
                      lingers.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "duration": { "type": "number", "exclusiveMinimum": 0 }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "split_into",
        description: "Cut a clip into `parts` equal pieces (2-100), or into pieces `every` \
                      seconds long (the last one takes what is left). One undo step.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "parts": { "type": "integer", "minimum": 2, "maximum": 100 },
                    "every": { "type": "number", "exclusiveMinimum": 0 }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "close_gaps",
        description: "Close every empty stretch on the lane `clip_id` is on, sliding the \
                      clips after each gap left (their linked sound moves too). One undo \
                      step. Says how many gaps closed.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "loop_clip",
        description: "Play a clip `times` times in a row (2-20): copies follow it, pushing \
                      what comes after later. Its sound loops with it.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "times": { "type": "integer", "minimum": 2, "maximum": 20 }
                }),
                &["clip_id", "times"],
            )
        },
    },
    Tool {
        name: "boomerang",
        description: "Play a picture clip forwards then backwards straight after, the \
                      back-and-forth loop. Pushes what follows later.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "transition_every_cut",
        description: "Put the same transition on every cut of the lane `clip_id` is on \
                      (`kind` as add_transition takes, or none to take every transition off \
                      that lane). Cuts without spare footage either side are skipped and \
                      counted.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" }, "kind": { "type": "string" } }),
                &["clip_id", "kind"],
            )
        },
    },
    Tool {
        name: "fit_music",
        description: "End a music clip (a sound clip not linked to a picture) where the \
                      pictures end, with a fade out, so the song does not run on past the \
                      edit.",
        schema: || object(json!({ "clip_id": { "type": "string" } }), &["clip_id"]),
    },
    Tool {
        name: "group_clips",
        description: "Group clips so they move together (at least two), or with `ungroup` \
                      true take the groups these clips are in apart.",
        schema: || {
            object(
                json!({
                    "clip_ids": { "type": "array", "items": { "type": "string" } },
                    "ungroup": { "type": "boolean" }
                }),
                &["clip_ids"],
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
        description: "Drop a sticker on the picture from `at` seconds, by name or as the \
                      emoji itself: grin, tears of joy, heart eyes, sunglasses, wow, sad, \
                      angry, thumbs up, thumbs down, clap, thank you, strong, eyes, fire, \
                      party, hundred, sparkles, boom, idea, rocket, rainbow, speech, heart, star, tick, cross, smile, frown, sun, cloud, umbrella, \
                      snowman, lightning, alarm clock, music note, telephone, aeroplane or \
                      football.",
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
        name: "set_effect",
        description: "Put an effect on a picture clip at `amount` 0–100 (0 takes it off): \
                      blur, sharpen, vignette, glow, old film, glitch, rgb split, pixelate, \
                      zoom blur, light leak, lens flare, beat pulse (a bump on each marker), \
                      shake, strobe, sway, flicker, heartbeat, bounce, smooth skin, fisheye or poster.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "effect": { "type": "string" },
                    "amount": { "type": "number", "minimum": 0, "maximum": 100 }
                }),
                &["clip_id", "effect", "amount"],
            )
        },
    },
    Tool {
        name: "green_screen",
        description: "Make a coloured backdrop see-through on a picture clip — a green (or \
                      blue) screen — so what is on the lane beneath shows instead. `color` is \
                      the screen's colour (\"#rrggbb\", green by default); `tolerance` (0–1, \
                      default 0.12) how far from it still counts; `softness` (0–1, default \
                      0.08) the edge; `spill` (0–1, default 0.6) how much green cast is taken \
                      off the subject. `off` true takes the key away. Check with \
                      preview_frame.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "color": { "type": "string" },
                    "tolerance": { "type": "number", "minimum": 0, "maximum": 1 },
                    "softness": { "type": "number", "minimum": 0, "maximum": 1 },
                    "spill": { "type": "number", "minimum": 0, "maximum": 1 },
                    "off": { "type": "boolean" }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "crop",
        description: "Cut the edges off a picture clip's source: `left`, `top`, `right` and \
                      `bottom` are fractions of the picture to take off each side (0 keeps \
                      the edge; 0.1 is a tenth). What is left is fitted to the frame like a \
                      new picture. All zero takes the crop off.",
        schema: || {
            object(
                json!({
                    "clip_id": { "type": "string" },
                    "left": { "type": "number", "minimum": 0, "maximum": 0.95 },
                    "top": { "type": "number", "minimum": 0, "maximum": 0.95 },
                    "right": { "type": "number", "minimum": 0, "maximum": 0.95 },
                    "bottom": { "type": "number", "minimum": 0, "maximum": 0.95 }
                }),
                &["clip_id"],
            )
        },
    },
    Tool {
        name: "batch",
        description: "Make several edits as one: `calls` is a list of `{tool, args}`, run in \
                      order, and everything they change becomes ONE undo step named `label` — \
                      so the person takes your whole change back with one undo. Stops at the \
                      first that fails, keeping (as one step) what was done before it. Each \
                      call's answer comes back in order. Not for export, preview_frame, \
                      contact_sheet or another batch.",
        schema: || {
            object(
                json!({
                    "label": { "type": "string" },
                    "calls": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "tool": { "type": "string" },
                                "args": { "type": "object" }
                            },
                            "required": ["tool"]
                        }
                    }
                }),
                &["calls"],
            )
        },
    },
    Tool {
        name: "chapters",
        description: "The YouTube chapter list made from the markers (`0:00 Intro`, one a \
                      line, named after each marker), ready to paste into a description — \
                      and anything YouTube would refuse (too few chapters, one too short).",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "export_captions",
        description: "Write the captions to a subtitle file at `path`: .srt or .vtt, by the \
                      ending — for YouTube, or a player that shows subtitles.",
        schema: || object(json!({ "path": { "type": "string" } }), &["path"]),
    },
    Tool {
        name: "history",
        description: "The steps undo would take back, newest first (the last 20), and the \
                      steps redo would bring back — attached to the window, the person's own \
                      edits are in it too, so look before undoing.",
        schema: || object(json!({}), &[]),
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
        description: "Remove a clip (or title); a shot's own sound goes with it. With \
                      `ripple`, everything after it on its lanes moves up to close the gap.",
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
        name: "speed_ramp",
        description: "Ramp a clip's speed along a preset curve: montage (slow, a fast rush, \
                      slow), hero (quick, a slow-motion moment, quick), bullet (eases into deep \
                      slow motion and out), jump cut (a sudden fast skip), flash in (starts \
                      fast) or flash out (races away at the end). The clip becomes pieces, \
                      each at its own speed; its linked sound follows. Returns the pieces.",
        schema: || {
            object(
                json!({ "clip_id": { "type": "string" },
                        "ramp": { "type": "string", "enum": speed_ramp_names() } }),
                &["clip_id", "ramp"],
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
                      Optionally its length in seconds. When the clips have no footage past \
                      the cut (placed whole), they overlap to make room and the edit gets \
                      that much shorter; describe_project shows the new times.",
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
        name: "contact_sheet",
        description: "See the whole edit at a glance: `count` frames (default 8, 2–16) spread \
                      evenly through it, each about `tile_width` pixels wide (default 320), in \
                      one picture, four to a row. The caption gives each frame's time.",
        schema: || {
            object(
                json!({
                    "count": { "type": "integer", "minimum": 2, "maximum": 16 },
                    "tile_width": { "type": "integer", "minimum": 96, "maximum": 640 }
                }),
                &[],
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
        description: "Render the whole timeline to a file at `path`; its ending picks the kind: \
                      .mp4 a video; .gif a looping animation (15 fps, 480 wide unless given); \
                      .wav the sound alone; .mov a ProRes master for editing elsewhere; .webm a \
                      video with a see-through background. At the project's size unless \
                      `width`/`height` are given. Waits until done — \
                      or, with `wait` false, starts it in the background and answers at once \
                      (export_status says how far it has got; keep this session open until it \
                      finishes).",
        schema: || {
            object(
                json!({
                    "path": { "type": "string" },
                    "width": { "type": "integer", "minimum": 16 },
                    "height": { "type": "integer", "minimum": 16 },
                    "wait": { "type": "boolean" }
                }),
                &["path"],
            )
        },
    },
    Tool {
        name: "export_status",
        description: "How the exports started with `wait` false are going: each one's file, \
                      and how far along it is, or that it finished (or why it failed).",
        schema: || object(json!({}), &[]),
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
            // Background exports run here, attached or not: the window only
            // hands over a copy of its project.
            "export" if args.get("wait").and_then(Value::as_bool) == Some(false) => {
                self.export_in_background(args)
            }
            "export_status" => Ok(Reply::Text(self.export_status())),
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

    /// Start an export on its own thread, from a copy of the project as it is
    /// now — the window's, when attached.
    fn export_in_background(&mut self, args: &Value) -> Result<Reply, String> {
        let path = path_arg(args, "path")?.display().to_string();
        let project: Project = match &self.app {
            Some(app) => match app.call("_project", &json!({}))? {
                Reply::Text(text) => serde_json::from_str(&text)
                    .map_err(|e| format!("could not read the window's project: {e}"))?,
                Reply::Image { .. } => return Err("the window sent a picture".to_owned()),
            },
            None => self.editor()?.project().clone(),
        };
        let sequence = project.active().ok_or("the project has no sequence")?;
        if sequence.duration().ticks() <= 0 {
            return Err("The timeline is empty: there is nothing to export".to_owned());
        }
        let state = std::sync::Arc::new(std::sync::Mutex::new(ExportState::Running {
            done: 0,
            total: 0,
        }));
        let progress = std::sync::Arc::clone(&state);
        let args = args.clone();
        std::thread::spawn(move || {
            let result = export_project_with(&project, &args, &mut |p| {
                if let Ok(mut state) = progress.lock() {
                    *state = ExportState::Running {
                        done: p.frames_done,
                        total: p.frames_total,
                    };
                }
            });
            if let Ok(mut state) = progress.lock() {
                *state = match result {
                    Ok(summary) => ExportState::Finished(summary),
                    Err(err) => ExportState::Failed(err),
                };
            }
        });
        self.exports.push((path.clone(), state));
        Ok(Reply::Text(format!(
            "Exporting {path} in the background: export_status says how far it has got"
        )))
    }

    fn export_status(&self) -> String {
        let all: Vec<Value> = self
            .exports
            .iter()
            .map(|(path, state)| {
                let state = state
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or(ExportState::Failed("the export thread stopped".to_owned()));
                match state {
                    ExportState::Running { done, total } => json!({
                        "path": path,
                        "state": "running",
                        "frames_done": done,
                        "frames_total": total,
                    }),
                    ExportState::Finished(summary) => json!({
                        "path": path,
                        "state": "finished",
                        "summary": serde_json::from_str::<Value>(&summary).unwrap_or(Value::Null),
                    }),
                    ExportState::Failed(err) => {
                        json!({ "path": path, "state": "failed", "error": err })
                    }
                }
            })
            .collect();
        Value::Array(all).to_string()
    }

    /// Look for the app's window in `file` rather than the usual place.
    pub fn with_live_file(file: PathBuf) -> Self {
        Self {
            live_file: Some(file),
            ..Self::default()
        }
    }

    /// How many edits this session's own project failed to write to its
    /// autosave journal — what a crash would lose. Zero, or something
    /// cannot be recovered.
    pub fn autosave_failures(&self) -> u32 {
        self.open
            .as_ref()
            .map_or(0, |(editor, _)| editor.autosave_failures())
    }

    /// Rebuild the open project from its crash-recovery data, as the app
    /// would after a crash, and say where it differs from the project as it
    /// is — `None` when they agree or nothing is open. For tests: every edit
    /// must come back.
    pub fn recovery_differs(&self) -> Option<String> {
        let (editor, _) = self.open.as_ref()?;
        // Saved since the last edit: the work is on disk, not in recovery
        // (whose folder beside a saved project another session may use).
        if !editor.is_dirty() {
            return None;
        }
        let Some(recovered) = bettercut_editor_core::recover(editor.recovery_paths().clone())
        else {
            return Some("unsaved edits, but nothing to recover".to_owned());
        };
        if recovered.failed > 0 {
            return Some(format!("{} edit(s) would not replay", recovered.failed));
        }
        let now = serde_json::to_value(editor.project()).ok()?;
        let back = serde_json::to_value(&recovered.project).ok()?;
        first_difference(&now, &back, "")
    }

    /// Write the open project to a file and read it back, and say where
    /// what comes back differs — `None` when it is the same. For tests: a
    /// project file must hold everything an edit made.
    pub fn file_round_trip_differs(&self) -> Option<String> {
        let (editor, _) = self.open.as_ref()?;
        let file = std::env::temp_dir().join(format!(
            "bettercut-round-trip-{}-{}.vproj",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let saved = bettercut_editor_core::project_format::save(editor.project(), &file);
        let loaded = saved.and_then(|()| bettercut_editor_core::project_format::load(&file));
        let _ = std::fs::remove_file(&file);
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(err) => return Some(format!("could not save and load: {err}")),
        };
        let now = serde_json::to_value(editor.project()).ok()?;
        let back = serde_json::to_value(&loaded).ok()?;
        first_difference(&now, &back, "")
    }

    /// Undo every step, redo every step, and say where the project then
    /// differs from before — `None` when it is the same. For tests, at the
    /// end of a session: every command's undo must be its exact opposite,
    /// in any company.
    pub fn undo_redo_all_differs(&mut self) -> Option<String> {
        let (editor, _) = self.open.as_mut()?;
        let before = serde_json::to_value(editor.project()).ok()?;
        let mut steps = 0;
        while editor.undo().is_ok() {
            steps += 1;
        }
        for step in 0..steps {
            if let Err(err) = editor.redo() {
                return Some(format!("redo {} of {steps} failed: {err}", step + 1));
            }
        }
        let after = serde_json::to_value(editor.project()).ok()?;
        first_difference(&before, &after, "")
            .map(|d| format!("after undoing and redoing {steps} steps: {d}"))
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
    if name == "contact_sheet" {
        return contact_sheet(editor, args);
    }
    if name == "batch" {
        return batch(editor, args).map(Reply::Text);
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
        "animate_clip" => animate_clip(editor, args),
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
        "shape_sound" => shape_sound(editor, args),
        "remove_range" => remove_range(editor, args),
        "whole_video_look" => whole_video_look(editor, args),
        "set_lane" => set_lane(editor, args),
        "add_sound_effect" => {
            use bettercut_editor_core::media::GeneratedSound;
            let wanted = str_arg(args, "effect")?;
            let sound = GeneratedSound::EFFECTS
                .into_iter()
                .find(|s| plain(&s.name()) == plain(wanted))
                .ok_or_else(|| {
                    let names: Vec<String> =
                        GeneratedSound::EFFECTS.iter().map(|s| s.name()).collect();
                    format!("no effect {wanted:?}; one of {}", names.join(", "))
                })?;
            editor.set_playhead(time_arg(args, "at")?);
            let length = timeline_time(sound.natural_length().unwrap_or(1.0));
            let clip = editor
                .add_generated_sound(sound, length)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": clip.to_string() }).to_string())
        }
        "organise_media" => {
            let media = MediaId::from_uuid(id_arg(args, "media_id")?);
            if editor.project().media_asset(media).is_none() {
                return Err("no imported file with that media_id".to_owned());
            }
            let depth = editor.undo_depth();
            let mut said = Vec::new();
            if let Some(name) = args.get("rename").and_then(Value::as_str) {
                editor
                    .rename_media(media, name)
                    .map_err(|e| e.to_string())?;
                said.push(format!(
                    "called {:?}",
                    editor
                        .project()
                        .media_asset(media)
                        .map_or("", |m| m.display_name())
                ));
            }
            if let Some(bin) = args.get("bin").and_then(Value::as_str) {
                if let Err(error) = editor.set_media_bin(media, bin) {
                    for _ in 0..editor.undo_depth().saturating_sub(depth) {
                        let _ = editor.undo();
                    }
                    return Err(error.to_string());
                }
                said.push(if bin.trim().is_empty() {
                    "in no bin".to_owned()
                } else {
                    format!("in the {:?} bin", bin.trim())
                });
            }
            if said.is_empty() {
                return Err("nothing to change: pass rename or bin".to_owned());
            }
            let steps = editor.undo_depth().saturating_sub(depth);
            editor.merge_last_steps(steps, "Organise Media");
            Ok(format!("Now {}", said.join(", ")))
        }
        "remove_unused_media" => {
            let removed = editor.remove_unused_media().map_err(|e| e.to_string())?;
            Ok(if removed == 0 {
                "Every file is in use; nothing removed".to_owned()
            } else {
                format!("{removed} unused files taken out of the project")
            })
        }
        "hold_last_frame" => {
            let clip = clip_arg(args)?;
            let duration = timeline_time(seconds(args, "duration")?.unwrap_or(2.0).max(0.04));
            let held = editor
                .hold_last_frame(clip, duration)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "clip_id": held.to_string() }).to_string())
        }
        "split_into" => {
            let clip = clip_arg(args)?;
            let made = match (u32_arg(args, "parts")?, seconds(args, "every")?) {
                (Some(parts), None) => editor.split_into_parts(clip, parts),
                (None, Some(every)) => editor.split_every(clip, timeline_time(every)),
                _ => return Err("give either parts or every".to_owned()),
            }
            .map_err(|e| e.to_string())?;
            Ok(format!("{made} cuts made"))
        }
        "close_gaps" => {
            let clip = clip_arg(args)?;
            let track = editor.track_of(clip).ok_or("no clip with that id")?;
            let closed = editor.close_all_gaps(track).map_err(|e| e.to_string())?;
            Ok(format!("{closed} gaps closed"))
        }
        "loop_clip" => {
            let clip = clip_arg(args)?;
            let times = u32_arg(args, "times")?.ok_or("times is required")?;
            let copies = editor.loop_clip(clip, times).map_err(|e| e.to_string())?;
            Ok(
                json!({ "clip_ids": copies.iter().map(ToString::to_string).collect::<Vec<_>>() })
                    .to_string(),
            )
        }
        "boomerang" => {
            let clip = clip_arg(args)?;
            let back = editor.boomerang(clip).map_err(|e| e.to_string())?;
            Ok(json!({ "reversed_clip_id": back.to_string() }).to_string())
        }
        "transition_every_cut" => {
            let clip = clip_arg(args)?;
            let track = editor.track_of(clip).ok_or("no clip with that id")?;
            let kind = str_arg(args, "kind")?;
            if plain(kind) == "none" {
                let removed = editor
                    .remove_every_transition(track)
                    .map_err(|e| e.to_string())?;
                return Ok(format!("{removed} transitions taken off"));
            }
            let kind = transition_named(kind)?;
            let (applied, skipped) = editor
                .transition_every_cut(track, kind)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "{} on {applied} cuts; {skipped} skipped for want of footage",
                kind.label()
            ))
        }
        "fit_music" => {
            let clip = clip_arg(args)?;
            let end = editor.fit_music(clip).map_err(|e| e.to_string())?;
            Ok(format!(
                "Music now ends at {:.3} s, fading out",
                end.as_seconds_f64()
            ))
        }
        "group_clips" => {
            let clips = args
                .get("clip_ids")
                .and_then(Value::as_array)
                .ok_or("clip_ids is required")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .and_then(|s| uuid::Uuid::parse_str(s).ok())
                        .map(ClipId::from_uuid)
                        .ok_or_else(|| "clip_ids holds something that is not an id".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if args.get("ungroup").and_then(Value::as_bool) == Some(true) {
                let parted = editor.ungroup_clips(&clips).map_err(|e| e.to_string())?;
                if parted == 0 {
                    return Err("none of those clips is in a group".to_owned());
                }
                return Ok(format!("{parted} groups taken apart"));
            }
            let grouped = editor.group_clips(&clips).map_err(|e| e.to_string())?;
            Ok(format!("{grouped} clips now move as one group"))
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
        "set_effect" => {
            let clip = clip_arg(args)?;
            if editor.video_clip(clip).is_none() {
                return Err("that is not a picture clip".to_owned());
            }
            use bettercut_editor_core::effects::{EFFECTS, NamedEffect};
            let wanted = str_arg(args, "effect")?;
            let effect = NamedEffect::named(wanted).ok_or_else(|| {
                let names: Vec<String> = EFFECTS.iter().map(|e| e.name.to_lowercase()).collect();
                format!("no effect {:?}; one of {}", plain(wanted), names.join(", "))
            })?;
            let label = effect.name.to_lowercase();
            let amount = number(args, "amount")?.clamp(0.0, 100.0) as f32;
            editor
                .set_clip_property(clip, effect.at(amount), false)
                .map_err(|e| e.to_string())?;
            Ok(if amount == 0.0 {
                format!("{label} off")
            } else {
                format!("{label} at {amount}")
            })
        }
        "green_screen" => {
            use bettercut_editor_core::timeline::ChromaKey;
            let clip = clip_arg(args)?;
            if editor.video_clip(clip).is_none() {
                return Err("that is not a picture clip".to_owned());
            }
            if args.get("off").and_then(Value::as_bool) == Some(true) {
                editor
                    .set_clip_property(clip, ClipProperty::ChromaKey(None), false)
                    .map_err(|e| e.to_string())?;
                return Ok("Key off".to_owned());
            }
            let mut key = ChromaKey::default();
            if let Some(colour) = args.get("color").and_then(Value::as_str) {
                let c = colour_arg(colour)?;
                key.color = [
                    f32::from(c.r) / 255.0,
                    f32::from(c.g) / 255.0,
                    f32::from(c.b) / 255.0,
                ];
            }
            if let Some(v) = optional_number(args, "tolerance")? {
                key.tolerance = v.clamp(0.0, 1.0);
            }
            if let Some(v) = optional_number(args, "softness")? {
                key.softness = v.clamp(0.0, 1.0);
            }
            if let Some(v) = optional_number(args, "spill")? {
                key.spill = v.clamp(0.0, 1.0);
            }
            editor
                .set_clip_property(clip, ClipProperty::ChromaKey(Some(key)), false)
                .map_err(|e| e.to_string())?;
            Ok("Keyed: put what should show through on the lane beneath".to_owned())
        }
        "crop" => {
            let clip = clip_arg(args)?;
            let now = editor
                .video_clip(clip)
                .ok_or("that is not a picture clip")?
                .crop;
            let edge = |name: &str, was: f32| -> Result<f32, String> {
                Ok(optional_number(args, name)?.unwrap_or(was))
            };
            let crop = bettercut_editor_core::timeline::Crop {
                left: edge("left", now.left)?,
                top: edge("top", now.top)?,
                right: edge("right", now.right)?,
                bottom: edge("bottom", now.bottom)?,
            }
            .clamped();
            editor
                .set_clip_property(clip, ClipProperty::Crop(crop), false)
                .map_err(|e| e.to_string())?;
            Ok(if crop.is_none() {
                "Crop off".to_owned()
            } else {
                format!(
                    "Cropped: left {:.2}, top {:.2}, right {:.2}, bottom {:.2}",
                    crop.left, crop.top, crop.right, crop.bottom
                )
            })
        }
        "chapters" => {
            let sequence = editor
                .active_sequence()
                .ok_or("the project has no sequence")?;
            let list = bettercut_editor_core::timeline::chapters::chapter_list(
                &sequence.markers,
                sequence.duration(),
            );
            Ok(json!({
                "text": list.text,
                "count": list.count,
                "problems": list.problems.iter().map(|p| p.describe()).collect::<Vec<_>>(),
            })
            .to_string())
        }
        "export_captions" => {
            let path = path_arg(args, "path")?;
            let written = editor.export_captions(&path).map_err(|e| e.to_string())?;
            Ok(format!("{written} captions written to {}", path.display()))
        }
        "history" => {
            let (done, undone) = editor.history_steps();
            let undo: Vec<&String> = done.iter().rev().take(20).collect();
            Ok(json!({ "undo": undo, "redo": undone }).to_string())
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
            let style = match args.get("style").and_then(Value::as_str) {
                Some(name) => Some(text_style_named(name)?),
                None => None,
            };
            editor.set_playhead(at);
            let depth = editor.undo_depth();
            let clip = editor.add_text(text).map_err(|e| e.to_string())?;
            if let Some(preset) = style {
                // The look over the title's own size and font, as the Text tab
                // gives it: one undo step with the title.
                if let Some(look) = editor.text_clip(clip).map(|t| preset.applied_to(&t.style)) {
                    editor
                        .set_text_property(clip, TextProperty::Style(Box::new(look)), false)
                        .map_err(|e| e.to_string())?;
                }
                let steps = editor.undo_depth().saturating_sub(depth);
                editor.merge_last_steps(steps, &format!("Add {} Text", preset.label()));
            }
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
        "speed_ramp" => {
            use bettercut_editor_core::SpeedRamp;
            let clip = clip_arg(args)?;
            let wanted = str_arg(args, "ramp")?;
            let ramp = SpeedRamp::ALL
                .into_iter()
                .find(|r| plain(r.label()) == plain(wanted))
                .ok_or_else(|| {
                    format!("no ramp {wanted:?}; one of {}", speed_ramp_names().join(", "))
                })?;
            let pieces = editor
                .apply_speed_ramp(clip, ramp)
                .map_err(|e| e.to_string())?;
            let ids: Vec<String> = pieces.iter().map(ToString::to_string).collect();
            Ok(json!({ "clip_ids": ids }).to_string())
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
            let before = editor_length(editor);
            match duration {
                // No footage past the cut: overlap at the length asked for,
                // rather than at the house length and then clamping to it.
                Some(duration) if editor.transition_overlap(clip, kind).is_some() => {
                    editor
                        .overlap_into_transition(clip, kind, timeline_time(duration))
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    editor
                        .set_transition(clip, kind)
                        .map_err(|e| e.to_string())?;
                    if let Some(duration) = duration {
                        editor
                            .set_transition_duration(clip, timeline_time(duration))
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            let shorter = before - editor_length(editor);
            Ok(if shorter > 0.0005 {
                format!(
                    "{} added; the clips had no footage past the cut, so they overlap to make \
                     room and the edit is {shorter:.2} s shorter",
                    kind.label()
                )
            } else {
                format!("{} added", kind.label())
            })
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
    use bettercut_editor_core::Command;
    editor.track_of(clip).ok_or("no clip with that id")?;
    let sequence = editor
        .active_sequence()
        .map(|s| s.id)
        .ok_or("the project has no sequence")?;
    // A shot's own sound goes with it, on its own lane, as Delete in the app
    // takes both: otherwise a ripple would leave the sound out of sync.
    let commands = editor
        .linked_with(clip)
        .into_iter()
        .filter_map(|clip| {
            let track = editor.track_of(clip)?;
            Some(if ripple {
                Command::RippleDeleteClip {
                    sequence,
                    track,
                    clip,
                }
            } else {
                Command::RemoveClip {
                    sequence,
                    track,
                    clip,
                }
            })
        })
        .collect();
    let label = if ripple { "Ripple Delete" } else { "Delete" };
    editor
        .dispatch_group(label, commands)
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

/// Several edits run in order and folded into one undo step.
fn batch(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .filter(|l| !l.trim().is_empty())
        .unwrap_or("Assistant's changes")
        .to_owned();
    let calls = args
        .get("calls")
        .and_then(Value::as_array)
        .ok_or("calls must be a list of {tool, args}")?;
    let depth = editor.undo_depth();
    let mut answers = Vec::new();
    let mut failed = None;
    for (i, call) in calls.iter().enumerate() {
        let tool = call.get("tool").and_then(Value::as_str).unwrap_or("");
        let call_args = call.get("args").cloned().unwrap_or_else(|| json!({}));
        let refused = match tool {
            "batch" | "export" | "preview_frame" | "contact_sheet" => {
                Some(format!("{tool} cannot be part of a batch"))
            }
            _ if !exists(tool) => Some(format!("no tool {tool:?}")),
            // Session-level tools, which do not act on an open project.
            "new_project" | "open_project" | "attach_to_app" | "detach_from_app"
            | "export_status" | "get_selection" | "select_clips" => {
                Some(format!("{tool} cannot be part of a batch"))
            }
            _ => None,
        };
        let answer = match refused {
            Some(reason) => Err(reason),
            None => run_text(editor, tool, &call_args),
        };
        match answer {
            Ok(text) => answers.push(json!({ "tool": tool, "ok": true, "answer": text })),
            Err(err) => {
                answers.push(json!({ "tool": tool, "ok": false, "error": err }));
                failed = Some(i);
                break;
            }
        }
    }
    let steps = editor.undo_depth().saturating_sub(depth);
    editor.merge_last_steps(steps, &label);
    let summary = json!({
        "undo_step": if steps > 0 { Value::String(label) } else { Value::Null },
        "results": answers,
    });
    match failed {
        Some(i) => Err(format!(
            "call {} of {} failed; the ones before it were kept as one step: {summary}",
            i + 1,
            calls.len()
        )),
        None => Ok(summary.to_string()),
    }
}

/// Frames spread through the edit, tiled into one picture: the export's own
/// contact sheet (`bettercut_export::contact_sheet`), handed over as an image.
fn contact_sheet(editor: &mut Editor, args: &Value) -> Result<Reply, String> {
    let count = u32_arg(args, "count")?.unwrap_or(8).clamp(2, 16);
    let tile_width = u32_arg(args, "tile_width")?.unwrap_or(320).clamp(96, 640);
    let sequence = editor
        .active_sequence()
        .ok_or("the project has no sequence")?;
    let length = sequence.duration();
    if length.ticks() <= 0 {
        return Err("The timeline is empty: there is nothing to see".to_owned());
    }
    let file = std::env::temp_dir().join(format!("bettercut-mcp-sheet-{}.png", std::process::id()));
    let range = bettercut_editor_core::timeline::TimelineRange::new(TimelineTime::ZERO, length)
        .map_err(|e| e.to_string())?;
    let mut settings = bettercut_export::SheetSettings::of(file.clone(), range);
    settings.tiles = count;
    settings.columns = count.min(4);
    settings.tile_width = tile_width;
    let times: Vec<String> = settings
        .instants()
        .iter()
        .map(|t| format!("{:.2}", t.as_seconds_f64()))
        .collect();
    bettercut_export::contact_sheet(
        editor.project(),
        sequence,
        &settings,
        &mut |_| {},
        &bettercut_editor_core::media::NeverCancelled,
    )
    .map_err(|e| format!("could not make the sheet: {e}"))?;
    let png = std::fs::read(&file).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&file);
    Ok(Reply::Image {
        png: png?,
        caption: format!(
            "{count} frames, left to right and down, at {} s",
            times.join(", ")
        ),
    })
}

/// Render `project`'s timeline as the export tool asks. Takes the project
/// alone so the app can export a copy away from its window's thread.
pub fn export_project(project: &Project, args: &Value) -> Result<String, String> {
    export_project_with(project, args, &mut |_| {})
}

/// [`export_project`], telling `on_progress` how far it has got.
fn export_project_with(
    project: &Project,
    args: &Value,
    on_progress: &mut dyn FnMut(bettercut_export::ExportProgress),
) -> Result<String, String> {
    let path = path_arg(args, "path")?;
    let width = u32_arg(args, "width")?;
    let height = u32_arg(args, "height")?;
    let sequence = project.active().ok_or("the project has no sequence")?;
    if sequence.duration().ticks() <= 0 {
        return Err("The timeline is empty: there is nothing to export".to_owned());
    }
    let mut settings = bettercut_export::ExportSettings::for_sequence(path.clone(), sequence);
    let ending = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ending.as_str() {
        "gif" => {
            settings.gif = true;
            if let Some(rate) = bettercut_editor_core::foundation::FrameRate::new(15, 1) {
                settings.frame_rate = rate;
            }
            // Small, unless asked otherwise: a GIF is a preview to share.
            let full = settings.resolution;
            let wide = 480.min(full.width.max(2));
            let high = (u64::from(full.height) * u64::from(wide) / u64::from(full.width.max(1)))
                .max(2) as u32;
            settings.resolution =
                bettercut_editor_core::timeline::Resolution::new(wide & !1, high & !1);
        }
        "wav" => settings.sound_only = true,
        "mov" => settings.prores = true,
        "webm" => settings.transparent = true,
        _ => {}
    }
    if let (Some(w), Some(h)) = (width, height) {
        settings.resolution = bettercut_editor_core::timeline::Resolution::new(w, h);
    }
    let summary = bettercut_export::export(
        project,
        sequence,
        &settings,
        on_progress,
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

/// Where two JSON values first differ, as a path, with both sides.
fn first_difference(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys()) {
                let (l, r) = (x.get(key), y.get(key));
                if l != r {
                    return first_difference(
                        l.unwrap_or(&Value::Null),
                        r.unwrap_or(&Value::Null),
                        &format!("{at}/{key}"),
                    );
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (l, r))| first_difference(l, r, &format!("{at}/{i}"))),
        _ if a == b => None,
        _ => Some(format!("{at}: now {a}, recovered {b}")),
    }
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

fn animate_clip(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::timeline::{Motion, MotionKind};
    let clip = clip_arg(args)?;
    let mut motion = editor
        .video_clip(clip)
        .ok_or("that is not a picture clip: use an id from picture_lanes")?
        .motion;
    let before = motion;
    let duration = timeline_time(seconds(args, "duration")?.unwrap_or(0.5).max(0.05));
    let kind = |name: &str| -> Result<Option<Motion>, String> {
        if plain(name) == "none" {
            return Ok(None);
        }
        one_of(&MotionKind::ALL, MotionKind::label, name, "animation")
            .map(|kind| Some(Motion::new(kind, duration)))
    };
    if let Some(name) = args.get("intro").and_then(Value::as_str) {
        motion.intro = kind(name)?;
    }
    if let Some(name) = args.get("outro").and_then(Value::as_str) {
        motion.outro = kind(name)?;
    }
    if motion == before {
        return Err("nothing to change: give intro or outro".to_owned());
    }
    editor
        .set_clip_property(clip, ClipProperty::Motion(motion), false)
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

/// A lane's name, lock, on/off, solo and volume, as one undo step.
fn set_lane(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::TrackFlag;
    let sequence = editor
        .active_sequence()
        .ok_or("the project has no sequence")?;
    // (id, name, sound lane's gain and pan)
    let lanes: Vec<_> = sequence
        .video_tracks
        .iter()
        .map(|t| (t.id, t.name.clone(), None))
        .chain(
            sequence
                .audio_tracks
                .iter()
                .map(|t| (t.id, t.name.clone(), Some((t.gain, t.pan)))),
        )
        .chain(
            sequence
                .text_tracks
                .iter()
                .map(|t| (t.id, t.name.clone(), None)),
        )
        .collect();
    let (track, name, mix) = match (
        args.get("lane").and_then(Value::as_str),
        args.get("clip_id"),
    ) {
        (Some(wanted), _) => lanes
            .into_iter()
            .find(|(_, name, _)| plain(name) == plain(wanted))
            .ok_or_else(|| format!("no lane called {wanted:?}"))?,
        (None, Some(_)) => {
            let track = editor
                .track_of(clip_arg(args)?)
                .ok_or("no clip with that id")?;
            lanes
                .into_iter()
                .find(|(id, _, _)| *id == track)
                .ok_or("that clip is not on a picture, sound or title lane")?
        }
        (None, None) => return Err("give lane or clip_id".to_owned()),
    };
    let volume = optional_number(args, "volume")?;
    if volume.is_some() && mix.is_none() {
        return Err(format!(
            "{name} is not a sound lane: volume is for sound lanes"
        ));
    }
    let depth = editor.undo_depth();
    let mut said = Vec::new();
    let outcome = (|| -> Result<(), String> {
        let flags = [
            ("locked", TrackFlag::Locked, "locked", "unlocked"),
            ("on", TrackFlag::Enabled, "on", "off"),
            ("solo", TrackFlag::Solo, "solo", "not solo"),
        ];
        // Unlock first and lock last, so a locked lane can still be renamed.
        if args.get("locked").and_then(Value::as_bool) == Some(false) {
            editor
                .set_track_flag(track, TrackFlag::Locked, false)
                .map_err(|e| e.to_string())?;
            said.push("unlocked".to_owned());
        }
        if let Some(new_name) = args.get("rename").and_then(Value::as_str) {
            editor
                .rename_track(track, new_name)
                .map_err(|e| e.to_string())?;
            said.push(format!("renamed {new_name:?}"));
        }
        if let (Some(volume), Some((_, pan))) = (volume, mix) {
            editor
                .set_track_mix(track, volume, pan, false)
                .map_err(|e| e.to_string())?;
            said.push(format!("volume {volume}"));
        }
        for (key, flag, yes, no) in flags {
            let Some(value) = args.get(key).and_then(Value::as_bool) else {
                continue;
            };
            if matches!(flag, TrackFlag::Locked) && !value {
                continue;
            }
            editor
                .set_track_flag(track, flag, value)
                .map_err(|e| e.to_string())?;
            said.push(if value { yes } else { no }.to_owned());
        }
        Ok(())
    })();
    let steps = editor.undo_depth().saturating_sub(depth);
    if let Err(error) = outcome {
        for _ in 0..steps {
            let _ = editor.undo();
        }
        return Err(error);
    }
    if said.is_empty() {
        return Err("nothing to change: pass rename, locked, on, solo or volume".to_owned());
    }
    editor.merge_last_steps(steps, "Change Lane");
    Ok(format!("{name}: {}", said.join(", ")))
}

/// The sequence's own look — bars, progress bar, vignette, grain, background —
/// as one undo step.
fn whole_video_look(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::timeline::MAX_PROGRESS_BAR;
    let master = editor
        .active_sequence()
        .ok_or("the project has no sequence")?
        .master;
    let mut properties = Vec::new();
    let mut said = Vec::new();
    if let Some(bars) = args.get("bars").and_then(Value::as_str) {
        let shape = if plain(bars) == "none" || plain(bars) == "off" {
            0.0
        } else {
            let ratio = bars.split(':').next().unwrap_or("").trim();
            ratio
                .parse::<f32>()
                .ok()
                .filter(|r| r.is_finite() && *r > 1.0 && *r <= 4.0)
                .ok_or_else(|| {
                    format!("bars is a shape wider than 1 like \"2.39\", or none — not {bars:?}")
                })?
        };
        properties.push(ClipProperty::Bars(shape));
        said.push(if shape == 0.0 {
            "no bars".to_owned()
        } else {
            format!("bars at {shape}:1")
        });
    }
    let mut bar = master.progress_bar;
    let mut bar_changed = false;
    if let Some(size) = args.get("progress_bar").and_then(Value::as_str) {
        bar.height = match plain(size).as_str() {
            "none" | "off" => 0.0,
            "thin" => MAX_PROGRESS_BAR * 0.3,
            "thick" => MAX_PROGRESS_BAR,
            _ => return Err(format!("progress_bar is none, thin or thick, not {size:?}")),
        };
        bar_changed = true;
        said.push(format!("progress bar {}", size.to_lowercase()));
    }
    if let Some(colour) = args.get("progress_color").and_then(Value::as_str) {
        let c = colour_arg(colour)?;
        bar.colour = [c.r, c.g, c.b];
        bar_changed = true;
    }
    if let Some(top) = args.get("progress_top").and_then(Value::as_bool) {
        bar.top = top;
        bar_changed = true;
    }
    if bar_changed {
        properties.push(ClipProperty::ProgressBar(bar));
    }
    if let Some(v) = optional_number(args, "vignette")? {
        let v = v.clamp(0.0, 100.0);
        properties.push(ClipProperty::Vignette(v / 100.0));
        said.push(format!("vignette {v}"));
    }
    if let Some(v) = optional_number(args, "grain")? {
        let v = v.clamp(0.0, 100.0);
        properties.push(ClipProperty::Grain(v / 100.0));
        said.push(format!("grain {v}"));
    }
    if let Some(colour) = args.get("background").and_then(Value::as_str) {
        let c = colour_arg(colour)?;
        properties.push(ClipProperty::Background([
            f32::from(c.r) / 255.0,
            f32::from(c.g) / 255.0,
            f32::from(c.b) / 255.0,
        ]));
        said.push(format!("background {colour}"));
    }
    if properties.is_empty() {
        return Err(
            "nothing to change: pass bars, progress_bar, progress_color, \
                    progress_top, vignette, grain or background"
                .to_owned(),
        );
    }
    let depth = editor.undo_depth();
    for property in properties {
        if let Err(error) = editor.set_sequence_value(property, false) {
            for _ in 0..editor.undo_depth().saturating_sub(depth) {
                let _ = editor.undo();
            }
            return Err(error.to_string());
        }
    }
    let steps = editor.undo_depth().saturating_sub(depth);
    editor.merge_last_steps(steps, "Whole Video Look");
    if said.is_empty() {
        said.push("progress bar restyled".to_owned());
    }
    Ok(format!("Whole video: {}", said.join(", ")))
}

/// Lift or extract a stretch given in seconds: marks it, takes it out, puts
/// the person's own marks back, and folds all of that into one undo step.
fn remove_range(editor: &mut Editor, args: &Value) -> Result<String, String> {
    let from = time_arg(args, "from")?;
    let to = time_arg(args, "to")?;
    if to <= from {
        return Err("to must be after from".to_owned());
    }
    let close = args
        .get("close_gap")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let sequence = editor
        .active_sequence()
        .ok_or("the project has no sequence")?;
    let (id, marks) = (sequence.id, (sequence.mark_in, sequence.mark_out));
    let depth = editor.undo_depth();
    let taken = (|| {
        editor.set_mark_in(from)?;
        editor.set_mark_out(to)?;
        let taken = if close {
            editor.extract_marked()?
        } else {
            editor.lift_marked()?
        };
        let now = editor.active_sequence().map(|s| (s.mark_in, s.mark_out));
        if now != Some(marks) {
            editor.dispatch(bettercut_editor_core::Command::SetInOut {
                sequence: id,
                mark_in: marks.0,
                mark_out: marks.1,
            })?;
        }
        Ok::<usize, bettercut_editor_core::EditorError>(taken)
    })();
    let steps = editor.undo_depth().saturating_sub(depth);
    let taken = match taken {
        Ok(taken) => taken,
        Err(error) => {
            for _ in 0..steps {
                let _ = editor.undo();
            }
            return Err(error.to_string());
        }
    };
    let label = if close { "Remove Range" } else { "Lift Range" };
    editor.merge_last_steps(steps, label);
    Ok(format!(
        "{taken} clips or parts of clips taken out{}",
        if close { ", gap closed" } else { "" }
    ))
}

/// The sound's equaliser, voice, room and robot, as one undo step.
fn shape_sound(editor: &mut Editor, args: &Value) -> Result<String, String> {
    use bettercut_editor_core::timeline::{ClipEq, ClipSpace, SpaceKind};
    let sound = sound_clip(editor, args)?;
    let mut properties = Vec::new();
    let mut said = Vec::new();
    if let Some(name) = args.get("eq").and_then(Value::as_str) {
        let eq = if plain(name) == "flat" || plain(name) == "off" {
            ClipEq::default()
        } else {
            ClipEq::PRESETS
                .iter()
                .find(|(label, _, _)| plain(label) == plain(name))
                .map(|(_, _, eq)| *eq)
                .ok_or_else(|| {
                    let names: Vec<&str> = ClipEq::PRESETS.iter().map(|(l, _, _)| *l).collect();
                    format!("no eq {name:?}; one of {}, or Flat", names.join(", "))
                })?
        };
        properties.push(ClipProperty::Eq(eq));
        said.push(format!("eq {name}"));
    }
    let pitch = match args.get("voice").and_then(Value::as_str) {
        None => optional_number(args, "pitch")?,
        Some(voice) => Some(match plain(voice).as_str() {
            "normal" => 0.0,
            "chipmunk" => 7.0,
            "deep" => -5.0,
            _ => return Err(format!("voice is normal, chipmunk or deep, not {voice:?}")),
        }),
    };
    if let Some(semitones) = pitch {
        let semitones = (semitones.clamp(-12.0, 12.0) * 10.0).round() / 10.0;
        properties.push(ClipProperty::Pitch(semitones));
        said.push(format!("pitch {semitones:+} semitones"));
    }
    if let Some(name) = args.get("space").and_then(Value::as_str) {
        let kind = one_of(&SpaceKind::ALL, SpaceKind::label, name, "space")?;
        let amount = optional_number(args, "space_amount")?.unwrap_or(30.0);
        let space = ClipSpace {
            kind,
            mix: amount / 100.0,
        }
        .clamped();
        properties.push(ClipProperty::Space(space));
        said.push(if space.is_dry() {
            "dry".to_owned()
        } else {
            format!(
                "{} at {}",
                kind.label().to_lowercase(),
                amount.clamp(0.0, 100.0)
            )
        });
    }
    if let Some(robot) = optional_number(args, "robot")? {
        let robot = robot.clamp(0.0, 100.0);
        properties.push(ClipProperty::Robot(robot));
        said.push(format!("robot {robot}"));
    }
    if properties.is_empty() {
        return Err("nothing to change: pass eq, voice, pitch, space or robot".to_owned());
    }
    editor
        .set_clip_properties(sound.id, properties, "Shape Sound")
        .map_err(|e| e.to_string())?;
    Ok(format!("Sound: {}", said.join(", ")))
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

/// A picture clip's effects in `set_effect`'s names and 0–100 amounts —
/// only those that are on.
fn effects_of(clip: &bettercut_editor_core::timeline::VideoClip) -> Value {
    let on: serde_json::Map<String, Value> = bettercut_editor_core::effects::EFFECTS
        .iter()
        .filter_map(|effect| {
            let amount = f64::from(effect.amount_on(clip));
            (amount.abs() > 1e-6).then(|| {
                (
                    effect.name.to_lowercase(),
                    json!((amount * 10.0).round() / 10.0),
                )
            })
        })
        .collect();
    Value::Object(on)
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
                "bin": m.bin,
                "kind": format!("{:?}", m.kind).to_lowercase(),
                "duration": seconds_of(m.duration.ticks()),
                "width": m.width,
                "height": m.height,
                // An iPhone Live Photo's moment, as video: import_media it to use it.
                "live_photo_video": m.live_video.as_ref().map(|p| p.display().to_string()),
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
                "lane": track.name, "locked": track.locked, "on": track.enabled,
                "clips": track.clips().iter().map(|c| {
                    let (start, end) = span(c.timeline.start, c.timeline.end);
                    json!({ "clip_id": c.id.to_string(), "start": start, "end": end,
                            "media": name_of(c.media_id),
                            "speed": c.speed.as_f64(), "reversed": c.reversed,
                            "opacity": c.opacity, "place": place(&c.transform),
                            "animated": animated_names(editor, c.id),
                            "effects": effects_of(c),
                            "green_screen": c.chroma_key.is_some(),
                            "cropped": !c.crop.is_none(),
                            "movement": editor.movement_of(c.id)
                                .filter(|m| *m != Movement::None).map(Movement::label),
                            "filter": editor.filter_of(c.id).map(|f| f.label()),
                            "intro": c.motion.intro.map(|m| m.kind.label()),
                            "outro": c.motion.outro.map(|m| m.kind.label()),
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
                "lane": track.name, "locked": track.locked, "on": track.enabled,
                "volume": track.gain,
                "clips": track.clips().iter().map(|c| {
                    let (start, end) = span(c.timeline.start, c.timeline.end);
                    json!({ "clip_id": c.id.to_string(), "start": start, "end": end,
                            "media": name_of(c.media_id),
                            "volume": c.gain, "speed": c.speed.as_f64(), "reversed": c.reversed,
                            "eq": (!c.eq.is_flat()).then(|| bettercut_editor_core::timeline::ClipEq::PRESETS
                                .iter().find(|(_, _, eq)| eq.clamped() == c.eq.clamped())
                                .map_or("custom", |(label, _, _)| *label)),
                            "pitch": c.pitch,
                            "space": (!c.space.is_dry()).then(|| json!({
                                "kind": c.space.kind.label(),
                                "amount": (c.space.mix * 100.0).round(),
                            })),
                            "robot": c.robot,
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
                "lane": track.name, "locked": track.locked, "on": track.enabled,
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
            "whole_video": {
                "bars": (sequence.master.bars > 1.0).then_some(sequence.master.bars),
                "progress_bar": sequence.master.progress_bar.is_visible().then(|| {
                    let bar = sequence.master.progress_bar;
                    json!({
                        "color": format!("#{:02x}{:02x}{:02x}",
                            bar.colour[0], bar.colour[1], bar.colour[2]),
                        "top": bar.top,
                    })
                }),
                "vignette": (sequence.master.vignette * 100.0).round(),
                "grain": (sequence.master.grain * 100.0).round(),
            },
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

/// How long the sequence on screen runs, in seconds.
fn editor_length(editor: &Editor) -> f64 {
    editor
        .active_sequence()
        .map_or(0.0, |s| s.duration().as_seconds_f64())
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

fn sound_effect_names() -> Vec<String> {
    bettercut_editor_core::media::GeneratedSound::EFFECTS
        .iter()
        .map(|s| s.name().to_lowercase())
        .collect()
}

fn speed_ramp_names() -> Vec<String> {
    bettercut_editor_core::SpeedRamp::ALL
        .iter()
        .map(|r| r.label().to_lowercase())
        .collect()
}

fn text_style_names() -> Vec<&'static str> {
    TextPreset::ALL.iter().map(|p| p.label()).collect()
}

fn text_style_named(name: &str) -> Result<TextPreset, String> {
    TextPreset::ALL
        .into_iter()
        .find(|p| p.label().eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| {
            format!(
                "no text style called {name:?}; try {}",
                text_style_names().join(", ")
            )
        })
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
