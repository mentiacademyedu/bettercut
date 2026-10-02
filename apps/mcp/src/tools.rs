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
use bettercut_editor_core::timeline::TransitionKind;
use bettercut_editor_core::{ClipProperty, Editor, EventReceiver, TrimEdge};
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
                            "opacity": c.opacity,
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
                            "volume": c.gain, "speed": c.speed.as_f64(), "reversed": c.reversed })
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
                            "text": c.text })
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
