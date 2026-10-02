//! The tools an assistant can call, and the project they act on.
//!
//! Each tool is a thin wrapper over one editor operation, so what it does is
//! exactly what the same action in the app does — and is undone the same way.
//! Times are seconds, the unit an assistant reasons in; they are turned into
//! the editor's exact ticks at the millisecond.

use std::path::{Path, PathBuf};

use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::{Editor, EventReceiver};
use serde_json::{Value, json};

/// The open project, if there is one.
#[derive(Default)]
pub struct Session {
    open: Option<(Editor, EventReceiver)>,
}

/// One tool: its name, what it is for, and the arguments it takes.
struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

const TOOLS: &[Tool] = &[
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
    pub fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "new_project" => self.new_project(args),
            "open_project" => {
                let path = path_arg(args, "path")?;
                let opened = Editor::open(&path).map_err(|e| format!("could not open: {e}"))?;
                self.open = Some(opened);
                Ok(format!("Opened {}", path.display()))
            }
            "save_project" => {
                let editor = self.editor()?;
                match args.get("path").and_then(Value::as_str) {
                    Some(path) => editor.save_as(path),
                    None => editor.save(),
                }
                .map_err(|e| format!("could not save: {e}"))?;
                Ok("Saved".to_owned())
            }
            "describe_project" => Ok(describe(self.editor()?).to_string()),
            "import_media" => self.import_media(args),
            "add_to_timeline" => self.add_to_timeline(args),
            "add_title" => {
                let text = str_arg(args, "text")?.to_owned();
                let at = time_arg(args, "at")?;
                let editor = self.editor()?;
                editor.set_playhead(at);
                let clip = editor.add_text(text).map_err(|e| e.to_string())?;
                Ok(json!({ "clip_id": clip.to_string() }).to_string())
            }
            "split_clip" => {
                let clip = clip_arg(args)?;
                let at = time_arg(args, "at")?;
                let made = self
                    .editor()?
                    .split_clip_at(clip, &[at])
                    .map_err(|e| e.to_string())?;
                if made == 0 {
                    return Err("that time is not inside the clip".to_owned());
                }
                Ok("Split".to_owned())
            }
            "delete_clip" => self.delete_clip(args),
            "undo" => {
                self.editor()?.undo().map_err(|e| e.to_string())?;
                Ok("Undone".to_owned())
            }
            "redo" => {
                self.editor()?.redo().map_err(|e| e.to_string())?;
                Ok("Redone".to_owned())
            }
            "export" => self.export(args),
            other => Err(format!("no tool {other}")),
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

    fn import_media(&mut self, args: &Value) -> Result<String, String> {
        let paths = args
            .get("paths")
            .and_then(Value::as_array)
            .ok_or("paths must be a list of files")?;
        let editor = self.editor()?;
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

    fn add_to_timeline(&mut self, args: &Value) -> Result<String, String> {
        let media = id_arg(args, "media_id").map(MediaId::from_uuid)?;
        let range = match (seconds(args, "from")?, seconds(args, "to")?) {
            (None, None) => None,
            (from, to) => Some((
                media_time(from.unwrap_or(0.0)),
                media_time(to.unwrap_or(f64::MAX / 2.0)),
            )),
        };
        let clips = self
            .editor()?
            .place_media_range(media, range)
            .map_err(|e| e.to_string())?;
        Ok(
            json!({ "clip_ids": clips.iter().map(ToString::to_string).collect::<Vec<_>>() })
                .to_string(),
        )
    }

    fn delete_clip(&mut self, args: &Value) -> Result<String, String> {
        let clip = clip_arg(args)?;
        let ripple = args.get("ripple").and_then(Value::as_bool).unwrap_or(false);
        let editor = self.editor()?;
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

    fn export(&mut self, args: &Value) -> Result<String, String> {
        let path = path_arg(args, "path")?;
        let width = u32_arg(args, "width")?;
        let height = u32_arg(args, "height")?;
        let editor = self.editor()?;
        let project = editor.project();
        let sequence = editor
            .active_sequence()
            .ok_or("the project has no sequence")?;
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
        return json!({ "media": media, "sequence": null });
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
                            "media": name_of(c.media_id) })
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
                            "media": name_of(c.media_id) })
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

fn id_arg(args: &Value, name: &str) -> Result<uuid::Uuid, String> {
    let text = str_arg(args, name)?;
    uuid::Uuid::parse_str(text).map_err(|_| format!("{name} is not an id bettercut gave out"))
}

fn clip_arg(args: &Value) -> Result<ClipId, String> {
    id_arg(args, "clip_id").map(ClipId::from_uuid)
}
