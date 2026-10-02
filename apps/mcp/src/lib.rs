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

mod tools;

use serde_json::{Value, json};

/// The protocol revisions this server speaks, newest first. A client asking
/// for one of these gets it; anything else is answered with the newest.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What a client is told the server is for, once, at the start.
const INSTRUCTIONS: &str = "bettercut is a desktop video editor. Open or create a project \
(open_project / new_project), import media, place it on the timeline, cut and title it, \
then export. Times are in seconds. Every edit can be undone (undo), and nothing is \
written to the project file until save_project.";

/// One assistant's session: at most one open project.
#[derive(Default)]
pub struct Server {
    session: tools::Session,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer one line of JSON-RPC. `None` for a notification, which takes
    /// no reply.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
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
        "capabilities": { "tools": { "listChanged": false } },
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
    fn base64_matches_the_standard() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
