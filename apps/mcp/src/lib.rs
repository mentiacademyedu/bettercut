//! bettercut as an MCP server: AI assistants editing through the same
//! undoable commands the interface uses.
//!
//! The Model Context Protocol over standard input and output is newline-
//! delimited JSON-RPC 2.0, and a server needs only four methods of it —
//! `initialize`, `ping`, `tools/list` and `tools/call` — so it is written
//! here directly on `serde_json` rather than through an SDK and an async
//! runtime. [`Server::handle_line`] takes one message and gives back the
//! reply, if there is one, which is also how the tests drive it.
//!
//! The editing is the editor's own (`bettercut_editor_core::Editor`): every
//! change an assistant makes is a command on the history, undoable like a
//! click, saved in the same project file the app opens.

pub mod live;
mod prompts;
mod tools;

use serde_json::{Value, json};

/// The protocol revisions this server speaks, newest first. A client asking
/// for one of these gets it; anything else is answered with the newest.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What a client is told the server is for, once, at the start.
const INSTRUCTIONS: &str = "bettercut is a desktop video editor. Open or create a project \
(open_project / new_project), import media, place it on the timeline, cut and title it, \
then export. If the person has the bettercut app open, attach_to_app edits the project \
in its window instead, live; then get_selection says which clips \"this\" means. \
describe_project gives the clip ids every edit takes, and preview_frame shows the result \
— look before saying it is done. Times are in seconds. Every edit is one undo step \
(history lists them; attached, the person's own edits are there too). Nothing is written \
to the project file until save_project. For a long export, pass wait false and follow \
export_status. Edits that belong together go in one batch, so one undo takes them back.";

/// What a compact server adds to its instructions.
const COMPACT_INSTRUCTIONS: &str = " Only the common tools are listed: find_tools finds the \
others by what they do (speed ramps, green screen, voice effects, beats, stickers and more), \
and use_tool runs one.";

/// Serve one client on standard input and output until it goes away: what
/// `bettercut-mcp` does, and `bettercut --mcp` (for an AppImage, which has
/// one program to run). `--compact` lists only the common tools, for
/// clients that take only so many.
pub fn serve_stdio() {
    use std::io::{BufRead, Write};
    let mut server = if std::env::args().any(|arg| arg == "--compact") {
        Server::compact()
    } else {
        Server::new()
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break; // the client went away
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line)
            && writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
}

/// One assistant's session: at most one open project.
#[derive(Default)]
pub struct Server {
    session: tools::Session,
    /// Listing only the common tools; see [`Server::compact`].
    compact: bool,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// A server that lists only the common tools, and `find_tools` and
    /// `use_tool` for the rest: for clients that take only so many tools
    /// (Cursor's limit is about 40, for every server together). Every tool
    /// can still be called by name.
    pub fn compact() -> Self {
        Self {
            compact: true,
            ..Self::default()
        }
    }

    /// A server that looks for the app's window in `file` (see [`live`])
    /// rather than the usual place: for tests.
    pub fn with_live_file(file: std::path::PathBuf) -> Self {
        Self {
            session: tools::Session::with_live_file(file),
            compact: false,
        }
    }

    /// Edits the open project could not write to its autosave journal; see
    /// `tools::Session::autosave_failures`.
    pub fn autosave_failures(&self) -> u32 {
        self.session.autosave_failures()
    }

    /// Where the open project, rebuilt from its crash-recovery data, differs
    /// from the project as it is; see `tools::Session::recovery_differs`.
    pub fn recovery_differs(&self) -> Option<String> {
        self.session.recovery_differs()
    }

    /// Where the open project, saved and loaded again, differs from itself;
    /// see `tools::Session::file_round_trip_differs`.
    pub fn file_round_trip_differs(&self) -> Option<String> {
        self.session.file_round_trip_differs()
    }

    /// See `tools::Session::undo_redo_all_differs`. For tests.
    pub fn undo_redo_all_differs(&mut self) -> Option<String> {
        self.session.undo_redo_all_differs()
    }

    /// Answer one line of JSON-RPC. `None` for a notification, which takes
    /// no reply.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        // A byte-order mark is not JSON, but Windows PowerShell puts one in
        // front of anything piped to a program; tolerate it.
        let line = line.trim_start_matches('\u{feff}');
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(err) => {
                return Some(error_reply(
                    Value::Null,
                    -32700,
                    &format!("not JSON: {err}"),
                ));
            }
        };
        // Protocol revision 2025-03-26 let a client send several messages as
        // one list, answered with one list.
        if let Value::Array(messages) = message {
            let replies: Vec<Value> = messages
                .iter()
                .filter_map(|m| self.handle(m))
                .filter_map(|reply| serde_json::from_str(&reply).ok())
                .collect();
            return (!replies.is_empty())
                .then(|| serde_json::to_string(&replies).unwrap_or_default());
        }
        self.handle(&message)
    }

    fn handle(&mut self, message: &Value) -> Option<String> {
        // A notification has no id and gets no answer, whatever it says; nor
        // does an answer to a request (this server makes none).
        let id = message.get("id").cloned()?;
        message.get("method")?;
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        let reply = match method {
            "initialize" => Ok(initialize(&params, self.compact)),
            "ping" => Ok(json!({})),
            "tools/list" if self.compact => Ok(json!({ "tools": tools::list_compact() })),
            "tools/list" => Ok(json!({ "tools": tools::list() })),
            // Not offered, but some clients ask anyway; an empty list is a
            // calmer answer than an error in their log.
            "resources/list" => Ok(json!({ "resources": [] })),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "tools/call" => self.call(&params),
            "prompts/list" => Ok(json!({ "prompts": prompts::list() })),
            "prompts/get" => prompts::get(
                params.get("name").and_then(Value::as_str).unwrap_or(""),
                params.get("arguments").unwrap_or(&Value::Null),
            )
            .map_err(|message| (-32602, message)),
            other => Err((-32601, format!("no method {other}"))),
        };
        Some(match reply {
            Ok(result) => serde_json::to_string(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }))
            .unwrap_or_default(),
            Err((code, message)) => error_reply(id, code, &message),
        })
    }

    /// `tools/call`: a tool that fails says so in its result (`isError`), so
    /// the assistant reads why; only a call naming no tool is a protocol
    /// error.
    fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "tools/call needs a name".to_owned()))?;
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        if !tools::exists(name) {
            return Err((-32602, format!("no tool {name}")));
        }
        if name == "find_tools" {
            let query = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            return Ok(json!({
                "content": [{ "type": "text", "text": tools::find(query) }],
                "isError": false,
            }));
        }
        if name == "use_tool" {
            let inner = arguments.get("name").and_then(Value::as_str).unwrap_or("");
            if inner == "use_tool" || inner == "find_tools" || !tools::exists(inner) {
                return Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("no tool {inner:?}; find_tools lists them"),
                    }],
                    "isError": true,
                }));
            }
            let inner_arguments = arguments.get("arguments").cloned().unwrap_or(json!({}));
            return self.call(&json!({ "name": inner, "arguments": inner_arguments }));
        }
        Ok(match self.session.call(name, &arguments) {
            Ok(tools::Reply::Text(text)) => {
                json!({ "content": [{ "type": "text", "text": text }], "isError": false })
            }
            Ok(tools::Reply::Image { png, caption }) => json!({
                "content": [
                    { "type": "image", "data": base64(&png), "mimeType": "image/png" },
                    { "type": "text", "text": caption }
                ],
                "isError": false,
            }),
            Err(message) => {
                json!({ "content": [{ "type": "text", "text": message }], "isError": true })
            }
        })
    }
}

fn initialize(params: &Value, compact: bool) -> Value {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|v| **v == asked)
        .copied()
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": { "listChanged": false },
            "prompts": { "listChanged": false },
        },
        "serverInfo": { "name": "bettercut", "version": env!("CARGO_PKG_VERSION") },
        "instructions": if compact {
            format!("{INSTRUCTIONS}{COMPACT_INSTRUCTIONS}")
        } else {
            INSTRUCTIONS.to_owned()
        },
    })
}

fn error_reply(id: Value, code: i64, message: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .unwrap_or_default()
}

/// Standard base64, padded: how MCP carries an image in a tool result. A few
/// lines here rather than a crate for the one place it is needed.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_byte_order_mark_is_not_a_parse_error() {
        let mut server = super::Server::new();
        let reply = server
            .handle_line("\u{feff}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}")
            .unwrap_or_default();
        assert!(reply.contains("\"result\""), "{reply}");
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
