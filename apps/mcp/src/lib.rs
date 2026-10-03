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
export_status.";

/// Serve one client on standard input and output until it goes away: what
/// `bettercut-mcp` does, and `bettercut --mcp` (for an AppImage, which has
/// one program to run).
pub fn serve_stdio() {
    use std::io::{BufRead, Write};
    let mut server = Server::new();
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
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// A server that looks for the app's window in `file` (see [`live`])
    /// rather than the usual place: for tests.
    pub fn with_live_file(file: std::path::PathBuf) -> Self {
        Self {
            session: tools::Session::with_live_file(file),
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
        // A notification has no id and gets no answer, whatever it says.
        let id = message.get("id").cloned()?;
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        let reply = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::list() })),
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

fn initialize(params: &Value) -> Value {
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
        "instructions": INSTRUCTIONS,
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
