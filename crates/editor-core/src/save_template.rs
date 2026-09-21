//! Save the edit as a template: its timing, framing, transitions and titles
//! kept, its footage turned into slots someone else fills with their own.
//!
//! Written in the same file format a template author writes (§30), and read
//! back through the same validator before it is saved — so a template made
//! here is exactly as trustworthy as one downloaded, and one that would not
//! load is never written.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use bettercut_foundation::{MediaId, TICKS_PER_SECOND, TimelineTime};
use bettercut_timeline::{ClipMotion, Movement, Transform};

use crate::editor::Editor;
use crate::error::EditorError;

/// What saving as a template kept and what it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedTemplate {
    pub path: PathBuf,
    /// Slots for the footage, photos and music to be filled in.
    pub slots: usize,
    /// Titles and clips placed.
    pub elements: usize,
    /// Shapes, timers and other things a template file cannot describe.
    pub left_out: usize,
}

impl Editor {
    /// The sequence on screen as a template file named `name`: its JSON, and
    /// counts of what went in and what was left out.
    pub fn template_json(&self, name: &str) -> Result<(String, usize, usize, usize), EditorError> {
        let sequence = self
            .active_sequence()
            .ok_or_else(|| EditorError::TemplateNotSaved("there is no sequence".to_owned()))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(EditorError::TemplateNotSaved(
                "give the template a name".to_owned(),
            ));
        }
        let duration = sequence.duration();
        if duration <= TimelineTime::ZERO {
            return Err(EditorError::TemplateNotSaved(
                "the edit is empty — there is nothing to make a template of".to_owned(),
            ));
        }

        let seconds = |t: TimelineTime| t.ticks() as f64 / TICKS_PER_SECOND as f64;
        let token = |value: Value| value.as_str().map(str::to_owned).unwrap_or_default();

        // One slot per file, so a shot used twice is filled once.
        let mut slots: Vec<Value> = Vec::new();
        let mut slot_of_media: Vec<(MediaId, String)> = Vec::new();
        let mut slot_for = |media: MediaId, kind: &str, label: &str| -> String {
            if let Some((_, id)) = slot_of_media.iter().find(|(m, _)| *m == media) {
                return id.clone();
            }
            let number = slot_of_media.len() + 1;
            let id = format!("{kind}_{number}");
            slots.push(json!({
                "id": id,
                "type": kind,
                "label": format!("{label} {number}"),
            }));
            slot_of_media.push((media, id.clone()));
            id
        };

        let transform_json = |t: &Transform| {
            json!({
                "position": [t.position.x, t.position.y],
                "scale": t.scale.x,
                "rotation": t.rotation_degrees,
                "flip_h": t.flip_h,
                "flip_v": t.flip_v,
            })
        };
        let motion_json = |intro: Option<bettercut_timeline::Motion>,
                           outro: Option<bettercut_timeline::Motion>| {
            let one = |m: bettercut_timeline::Motion| {
                json!({
                    "kind": token(serde_json::to_value(m.kind).unwrap_or(Value::Null)),
                    "duration": seconds(m.duration),
                })
            };
            let mut animation = serde_json::Map::new();
            if let Some(m) = intro {
                animation.insert("in".to_owned(), one(m));
            }
            if let Some(m) = outro {
                animation.insert("out".to_owned(), one(m));
            }
            Value::Object(animation)
        };

        let mut elements: Vec<Value> = Vec::new();
        let mut left_out = 0;

        for (track_index, track) in sequence.video_tracks.iter().enumerate() {
            for clip in track.clips() {
                let Some(asset) = self.project().media_asset(clip.media_id) else {
                    left_out += 1;
                    continue;
                };
                // A colour clip has no file for anyone to replace.
                if asset.generated.is_some() || track_index >= 8 {
                    left_out += 1;
                    continue;
                }
                let (kind, label) = if asset.is_still() {
                    ("image", "Photo")
                } else {
                    ("video", "Shot")
                };
                let slot = slot_for(clip.media_id, kind, label);
                let mut element = serde_json::Map::new();
                element.insert("type".to_owned(), json!("clip"));
                element.insert("slot".to_owned(), json!(slot));
                element.insert("start".to_owned(), json!(seconds(clip.timeline.start)));
                element.insert(
                    "duration".to_owned(),
                    json!(seconds(clip.timeline.duration())),
                );
                element.insert("track".to_owned(), json!(track_index));
                element.insert("transform".to_owned(), transform_json(&clip.transform));
                if !clip.crop.is_none() {
                    element.insert(
                        "crop".to_owned(),
                        json!([
                            clip.crop.left,
                            clip.crop.top,
                            clip.crop.right,
                            clip.crop.bottom
                        ]),
                    );
                }
                if (clip.opacity - 1.0).abs() > 1e-4 {
                    element.insert("opacity".to_owned(), json!(clip.opacity));
                }
                if clip.speed.num() != clip.speed.den() {
                    element.insert("speed".to_owned(), json!(clip.speed.as_f64()));
                }
                if let Some(transition) = clip.transition_out {
                    element.insert(
                        "transition_out".to_owned(),
                        json!({
                            "kind": token(serde_json::to_value(transition.kind).unwrap_or(Value::Null)),
                            "duration": seconds(transition.duration),
                        }),
                    );
                }
                if let Some(movement) = self.movement_of(clip.id).filter(|m| *m != Movement::None) {
                    element.insert("movement".to_owned(), json!(movement_token(movement)));
                }
                let ClipMotion { intro, outro } = clip.motion;
                if intro.is_some() || outro.is_some() {
                    element.insert("animation".to_owned(), motion_json(intro, outro));
                }
                elements.push(Value::Object(element));
            }
        }

        for track in &sequence.text_tracks {
            for clip in track.clips() {
                if clip.shape.is_some() || clip.counter.is_some() || clip.text.trim().is_empty() {
                    left_out += 1;
                    continue;
                }
                let mut element = serde_json::Map::new();
                element.insert("type".to_owned(), json!("text"));
                element.insert("text".to_owned(), json!(clip.text));
                element.insert("start".to_owned(), json!(seconds(clip.timeline.start)));
                element.insert(
                    "duration".to_owned(),
                    json!(seconds(clip.timeline.duration())),
                );
                element.insert("transform".to_owned(), transform_json(&clip.transform));
                element.insert(
                    "style".to_owned(),
                    serde_json::to_value(&clip.style).unwrap_or(Value::Null),
                );
                if clip.animation.intro.is_some() || clip.animation.outro.is_some() {
                    element.insert(
                        "animation".to_owned(),
                        motion_json(clip.animation.intro, clip.animation.outro),
                    );
                }
                elements.push(Value::Object(element));
            }
        }

        for (track_index, track) in sequence.audio_tracks.iter().enumerate() {
            for clip in track.clips() {
                // A shot's own sound comes with the shot someone puts in.
                if clip.link.is_some() || track_index >= 8 {
                    left_out += usize::from(clip.link.is_none());
                    continue;
                }
                let slot = slot_for(clip.media_id, "audio", "Music");
                elements.push(json!({
                    "type": "audio",
                    "slot": slot,
                    "start": seconds(clip.timeline.start),
                    "duration": seconds(clip.timeline.duration()),
                    "track": track_index,
                    "volume": clip.gain,
                    "fade_in": seconds(clip.fade_in),
                    "fade_out": seconds(clip.fade_out),
                }));
            }
        }

        let (slot_count, element_count) = (slots.len(), elements.len());
        let file = json!({
            "schema_version": bettercut_templates::SCHEMA_VERSION,
            "id": template_id(name),
            "name": name,
            "category": "My templates",
            "duration": seconds(duration),
            "description": format!("Saved from \"{}\"", sequence.name),
            "slots": slots,
            "elements": elements,
        });
        let text = serde_json::to_string_pretty(&file)
            .map_err(|e| EditorError::TemplateNotSaved(e.to_string()))?;
        // Read back through the validator: a template that would not load is
        // not written.
        bettercut_templates::parse(&text).map_err(|problems| {
            EditorError::TemplateNotSaved(
                problems
                    .iter()
                    .take(3)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        Ok((text, slot_count, element_count, left_out))
    }

    /// Save the sequence on screen as a template named `name` into `folder`,
    /// under a file name of its own.
    pub fn save_as_template(
        &self,
        name: &str,
        folder: &Path,
    ) -> Result<SavedTemplate, EditorError> {
        let (text, slots, elements, left_out) = self.template_json(name)?;
        std::fs::create_dir_all(folder).map_err(|e| EditorError::Io(e.to_string()))?;
        let stem = template_id(name);
        let path = (1..)
            .map(|n| {
                if n == 1 {
                    folder.join(format!("{stem}.json"))
                } else {
                    folder.join(format!("{stem}-{n}.json"))
                }
            })
            .find(|path| !path.exists())
            .unwrap_or_else(|| folder.join(format!("{stem}.json")));
        std::fs::write(&path, text).map_err(|e| EditorError::Io(e.to_string()))?;
        Ok(SavedTemplate {
            path,
            slots,
            elements,
            left_out,
        })
    }
}

/// A template id from its name: lowercase letters and digits, words joined by
/// hyphens, at most 64 characters.
fn template_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let id = id
        .trim_end_matches('-')
        .chars()
        .take(64)
        .collect::<String>();
    if id.is_empty() {
        "my-template".to_owned()
    } else {
        id
    }
}

fn movement_token(movement: Movement) -> &'static str {
    match movement {
        Movement::None => "none",
        Movement::ZoomIn => "zoom_in",
        Movement::ZoomOut => "zoom_out",
        Movement::PanLeft => "pan_left",
        Movement::PanRight => "pan_right",
        Movement::PanUp => "pan_up",
        Movement::PanDown => "pan_down",
    }
}
