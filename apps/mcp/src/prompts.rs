//! Ready-made requests: the edits people ask for most, written out once so
//! an assistant's client can offer them by name — Claude Code shows them as
//! slash commands. Each is only words for the assistant to act on with the
//! tools; nothing here edits anything itself.

use serde_json::{Value, json};

/// One prompt: its name, what it is for, what it asks for, and the request.
struct Prompt {
    name: &'static str,
    description: &'static str,
    /// Each argument: name, what it is, whether it must be given.
    arguments: &'static [(&'static str, &'static str, bool)],
    /// The request, with `{name}` where each argument goes.
    text: &'static str,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "tighten_interview",
        description: "Cut the pauses out of someone talking, level the sound and name the speaker",
        arguments: &[
            ("file", "The recording, as a path", true),
            ("speaker", "Who is talking, for a lower third", false),
        ],
        text: "In bettercut, tighten this interview: {file}. If the bettercut app is open, \
               attach_to_app first so I can watch. Import it and put it on the timeline, look \
               at the pauses with remove_silences preview, then cut them. Use enhance_voice and \
               normalise_volume on the sound. If a speaker is named ({speaker}), add a lower \
               third with their name near the start. Check a frame or two with preview_frame, \
               then tell me what you changed and how much shorter it is.",
    },
    Prompt {
        name: "make_short",
        description: "A vertical 9:16 copy of the edit for Shorts, TikTok and Reels, with captions",
        arguments: &[(
            "captions",
            "What is said, if captions are wanted (or leave it out)",
            false,
        )],
        text: "In bettercut, make a vertical Short from the edit that is open (attach_to_app if \
               the app is open). Use copy_as_shape 9:16, then look at a few frames with \
               preview_frame and move anything important that the reframing cut off \
               (set_transform). If I gave the words ({captions}), add them as captions in short \
               lines timed to the cut. Keep it under 60 seconds if you can, and tell me where \
               you trimmed.",
    },
    Prompt {
        name: "cut_to_the_beat",
        description: "Cut a set of shots to the beat of a song",
        arguments: &[
            ("song", "The music, as a path", true),
            (
                "shots",
                "The clips or photos, as paths separated by commas",
                true,
            ),
        ],
        text: "In bettercut, cut a music edit (attach_to_app if the app is open). Import the \
               song {song} and the shots {shots}. Put the song on the timeline and mark_beats \
               on it. Place the shots one after another so each cut lands on a beat — every \
               beat for fast songs, every second or fourth beat for slow ones — trimming each \
               shot to fit. Add a fade at the very end. Then show me a frame from the middle \
               and say how the cuts line up.",
    },
    Prompt {
        name: "title_card",
        description: "A title card at the start: a background colour, a title and a subtitle",
        arguments: &[
            ("title", "The title", true),
            ("subtitle", "A line beneath it", false),
        ],
        text: "In bettercut, put a three-second title card at the very start of the edit \
               (attach_to_app if the app is open): a dark colour background with add_colour, \
               the title \"{title}\" large in the middle with style_title, and \"{subtitle}\" \
               smaller beneath it if given. Give the title a gentle animate_title intro, and \
               fade into the first shot. Check it with preview_frame.",
    },
];

/// The prompts, as `prompts/list` reports them.
pub fn list() -> Vec<Value> {
    PROMPTS
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "description": p.description,
                "arguments": p.arguments.iter().map(|(name, description, required)| json!({
                    "name": name,
                    "description": description,
                    "required": required,
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// `prompts/get`: the request with its arguments filled in.
pub fn get(name: &str, arguments: &Value) -> Result<Value, String> {
    let prompt = PROMPTS
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("no prompt {name}"))?;
    let mut text = prompt.text.to_owned();
    for (argument, _, required) in prompt.arguments {
        let given = arguments
            .get(*argument)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if *required && given.is_none() {
            return Err(format!("{name} needs {argument}"));
        }
        text = text.replace(&format!("{{{argument}}}"), given.unwrap_or("none given"));
    }
    Ok(json!({
        "description": prompt.description,
        "messages": [{ "role": "user", "content": { "type": "text", "text": text } }],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prompt_fills_every_argument_and_names_real_tools() {
        let tools: Vec<String> = crate::tools::list()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        for prompt in PROMPTS {
            let args: serde_json::Map<String, Value> = prompt
                .arguments
                .iter()
                .map(|(name, _, _)| ((*name).to_owned(), json!("x")))
                .collect();
            let got = get(prompt.name, &Value::Object(args)).unwrap_or_default();
            let text = got["messages"][0]["content"]["text"]
                .as_str()
                .unwrap_or_default();
            assert!(
                !text.contains('{'),
                "{}: an argument was not filled",
                prompt.name
            );
            // Every snake_case word in a prompt that looks like a tool is one.
            for word in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                if word.contains('_')
                    && word
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                {
                    assert!(
                        tools.iter().any(|t| t == word),
                        "{}: no tool {word}",
                        prompt.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_missing_required_argument_is_said() {
        let err = get("tighten_interview", &json!({}))
            .err()
            .unwrap_or_default();
        assert!(err.contains("file"), "{err}");
        assert!(get("no_such", &json!({})).is_err());
    }
}
