//! The live link: an assistant editing the project open in the app's window.
//!
//! The app listens on a port on this machine only (127.0.0.1) and writes the
//! port and a random token to `live.json` in bettercut's data folder, which
//! only this user can read. `bettercut-mcp` reads that file to connect, and
//! every request carries the token, so nothing else on the machine can drive
//! the window.
//!
//! The wire is one connection per tool call: a line of JSON asking
//! (`{"token", "tool", "args"}`), a line of JSON answering (`{"ok", "text"}`,
//! `{"ok", "caption", "png_len"}` followed by that many bytes of PNG, or
//! `{"ok": false, "error"}`). The app runs the tool on its own editor, on
//! its own thread, between two frames — so the edit lands in its undo history
//! and is on screen at once.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::ClipId;
use serde_json::{Value, json};

use crate::tools::{self, Reply};

/// The file that says where the open window listens.
pub fn live_file() -> PathBuf {
    bettercut_editor_core::foundation::places::data_home().join("live.json")
}

/// One tool call waiting for the window, and where its answer goes.
struct Request {
    tool: String,
    args: Value,
    answer: Sender<Result<Reply, String>>,
}

/// The app's end: listening, and handing calls to the window.
pub struct Listener {
    requests: Receiver<Request>,
    token: String,
    file: PathBuf,
}

impl Listener {
    /// Listen, and say where in `file`. `wake` is called when a call arrives,
    /// so a window that draws only on change draws a frame to answer it.
    pub fn start(file: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let socket = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
        let port = socket.local_addr()?.port();
        let token = uuid::Uuid::new_v4().simple().to_string();
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let said = json!({ "port": port, "token": token, "pid": std::process::id() });
        std::fs::write(&file, said.to_string())?;

        let (send, requests) = channel();
        let wake = std::sync::Arc::new(wake);
        let expected = token.clone();
        std::thread::Builder::new()
            .name("bettercut-live".to_owned())
            .spawn(move || {
                for stream in socket.incoming().flatten() {
                    let send = send.clone();
                    let wake = wake.clone();
                    let expected = expected.clone();
                    // Its own thread, so a slow export does not hold up the
                    // next caller's handshake.
                    let _ = std::thread::Builder::new()
                        .name("bettercut-live-call".to_owned())
                        .spawn(move || answer(stream, &expected, &send, &*wake));
                }
            })?;
        Ok(Self {
            requests,
            token,
            file,
        })
    }

    /// Run every call that has arrived on `editor`. `selected` is what the
    /// person has selected in the window. Returns what happened, for the
    /// window to redraw, say so, and select what the assistant pointed at.
    pub fn serve(&self, editor: &mut Editor, selected: &[ClipId]) -> Served {
        let mut served = Served::default();
        while let Ok(request) = self.requests.try_recv() {
            served.any = true;
            if !matches!(
                request.tool.as_str(),
                "describe_project"
                    | "preview_frame"
                    | "contact_sheet"
                    | "get_selection"
                    | "select_clips"
                    | "_project"
            ) {
                served.last_change = Some(request.tool.replace('_', " "));
            }
            match request.tool.as_str() {
                "get_selection" => {
                    let _ = request.answer.send(Ok(Reply::Text(
                        tools::selection(editor, selected).to_string(),
                    )));
                    continue;
                }
                "_project" => {
                    let done = serde_json::to_string(editor.project())
                        .map(Reply::Text)
                        .map_err(|e| e.to_string());
                    let _ = request.answer.send(done);
                    continue;
                }
                "select_clips" => {
                    let done = tools::clip_ids(&request.args).map(|clips| {
                        let count = clips.len();
                        served.select = Some(clips);
                        Reply::Text(format!("{count} clip(s) selected in the window"))
                    });
                    let _ = request.answer.send(done);
                    continue;
                }
                _ => {}
            }
            if request.tool == "export" {
                // Rendered off the window's thread, from a copy of the
                // project as it is now: a minute-long export must not freeze
                // the window.
                let project = editor.project().clone();
                std::thread::spawn(move || {
                    let done = tools::export_project(&project, &request.args).map(Reply::Text);
                    let _ = request.answer.send(done);
                });
                continue;
            }
            let done = tools::run_on(editor, &request.tool, &request.args);
            let _ = request.answer.send(done);
        }
        served
    }
}

/// What one [`Listener::serve`] did.
#[derive(Debug, Default)]
pub struct Served {
    /// Whether any call ran, so the window draws again.
    pub any: bool,
    /// The last tool that changes something, in words ("add title"); looking
    /// at the project is not news.
    pub last_change: Option<String>,
    /// Clips the assistant asked the window to select.
    pub select: Option<Vec<ClipId>>,
}

impl Drop for Listener {
    /// The file goes with the window — unless a newer window wrote its own.
    fn drop(&mut self) {
        let ours = std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|said| said["token"] == self.token.as_str());
        if ours {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

/// One connection: read the call, check the token, wait for the window, answer.
fn answer(stream: TcpStream, token: &str, send: &Sender<Request>, wake: &(dyn Fn() + Send + Sync)) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return;
    }
    let call: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    let done = if call["token"] != token {
        Err("the app refused the connection (wrong token)".to_owned())
    } else {
        let (answer, answered) = channel();
        let request = Request {
            tool: call["tool"].as_str().unwrap_or_default().to_owned(),
            args: call.get("args").cloned().unwrap_or(Value::Null),
            answer,
        };
        if send.send(request).is_err() {
            Err("the app is closing".to_owned())
        } else {
            wake();
            answered
                .recv()
                .unwrap_or_else(|_| Err("the app is closing".to_owned()))
        }
    };
    let _ = write_reply(&mut writer, done);
}

fn write_reply(out: &mut impl Write, done: Result<Reply, String>) -> std::io::Result<()> {
    match done {
        Ok(Reply::Text(text)) => writeln!(out, "{}", json!({ "ok": true, "text": text })),
        Ok(Reply::Image { png, caption }) => {
            writeln!(
                out,
                "{}",
                json!({ "ok": true, "caption": caption, "png_len": png.len() })
            )?;
            out.write_all(&png)
        }
        Err(error) => writeln!(out, "{}", json!({ "ok": false, "error": error })),
    }?;
    out.flush()
}

/// The assistant's end: where the window listens.
pub struct Link {
    port: u16,
    token: String,
}

impl Link {
    /// Find the open window from `file`, and check it answers.
    pub fn connect(file: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|_| "bettercut is not open: start the app, then attach again".to_owned())?;
        let said: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let link = Self {
            port: said["port"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
                .ok_or("live.json has no port")?,
            token: said["token"].as_str().unwrap_or_default().to_owned(),
        };
        link.call("describe_project", &json!({})).map_err(|err| {
            if err.starts_with("could not reach") {
                "bettercut is not open (it may have closed without tidying up): \
                 start the app, then attach again"
                    .to_owned()
            } else {
                err
            }
        })?;
        Ok(link)
    }

    /// Run `tool` in the window.
    pub fn call(&self, tool: &str, args: &Value) -> Result<Reply, String> {
        let mut stream = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.port)),
            Duration::from_secs(5),
        )
        .map_err(|e| format!("could not reach the app: {e}"))?;
        let ask = json!({ "token": self.token, "tool": tool, "args": args });
        writeln!(stream, "{ask}").map_err(|e| format!("could not reach the app: {e}"))?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("the app did not answer: {e}"))?;
        let said: Value =
            serde_json::from_str(&line).map_err(|_| "the app did not answer".to_owned())?;
        if said["ok"] != true {
            return Err(said["error"].as_str().unwrap_or("failed").to_owned());
        }
        if let Some(len) = said["png_len"].as_u64() {
            let mut png = vec![0; usize::try_from(len).map_err(|e| e.to_string())?];
            reader
                .read_exact(&mut png)
                .map_err(|e| format!("the app's picture was cut short: {e}"))?;
            return Ok(Reply::Image {
                png,
                caption: said["caption"].as_str().unwrap_or_default().to_owned(),
            });
        }
        Ok(Reply::Text(
            said["text"].as_str().unwrap_or_default().to_owned(),
        ))
    }
}
