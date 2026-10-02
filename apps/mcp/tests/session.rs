//! An assistant's session, end to end, through the protocol alone: the
//! handshake, then a whole edit — new project, import, place, title, split,
//! delete, undo, save, reopen, export.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_mcp::Server;
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/media/tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

struct Client {
    server: Server,
    next: u64,
}

impl Client {
    fn new() -> Self {
        Self {
            server: Server::new(),
            next: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let reply = self
            .server
            .handle_line(&line.to_string())
            .expect("a request is answered");
        let reply: Value = serde_json::from_str(&reply).expect("the reply is JSON");
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], id, "the reply is to this request");
        reply
    }

    /// Call a tool; the text it returns, and whether it is an error.
    fn tool(&mut self, name: &str, arguments: Value) -> (String, bool) {
        let reply = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &reply["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        (text, result["isError"].as_bool().unwrap_or(true))
    }

    fn ok(&mut self, name: &str, arguments: Value) -> String {
        let (text, is_error) = self.tool(name, arguments);
        assert!(!is_error, "{name} failed: {text}");
        text
    }
}

#[test]
fn the_handshake_offers_the_tools() {
    let mut client = Client::new();
    let init = client.request(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": { "name": "test", "version": "1" } }),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "bettercut");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // A version it does not know is answered with the newest it does.
    let other = client.request("initialize", json!({ "protocolVersion": "1999-01-01" }));
    assert_eq!(
        other["result"]["protocolVersion"],
        bettercut_mcp::PROTOCOL_VERSIONS[0]
    );

    // Notifications get no reply at all.
    assert!(
        client
            .server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none()
    );

    let list = client.request("tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for wanted in [
        "new_project",
        "import_media",
        "add_to_timeline",
        "export",
        "undo",
    ] {
        assert!(
            names.contains(&wanted),
            "{wanted} is not offered: {names:?}"
        );
    }
    for tool in list["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
    }
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
}

#[test]
fn protocol_mistakes_are_answered_as_errors() {
    let mut client = Client::new();
    assert_eq!(
        client.request("no/such/method", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        client.request("tools/call", json!({ "name": "no_such_tool" }))["error"]["code"],
        -32602
    );
    let garbled = client.server.handle_line("{not json").expect("answered");
    assert!(garbled.contains("-32700"), "{garbled}");
    // A tool that cannot run says why, as a tool result.
    let (text, is_error) = client.tool("describe_project", json!({}));
    assert!(is_error);
    assert!(text.contains("open_project"), "{text}");
}

#[test]
fn an_assistant_makes_an_edit_and_exports_it() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("edit.vproj").display().to_string();
    let mut client = Client::new();

    client.ok(
        "new_project",
        json!({ "path": project, "width": 640, "height": 360, "fps": 30 }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("ntsc-2997.mp4"), dir.join("missing.mp4").display().to_string()] }),
    ))
    .unwrap();
    let media = imported[0]["media_id"]
        .as_str()
        .expect("imported")
        .to_owned();
    assert!(imported[1]["error"].is_string(), "a missing file says so");

    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": media, "from": 0.0, "to": 2.0 }),
    ))
    .unwrap();
    let clips = placed["clip_ids"].as_array().unwrap();
    assert_eq!(clips.len(), 2, "picture and sound");
    let picture = clips[0].as_str().unwrap().to_owned();

    client.ok(
        "add_title",
        json!({ "text": "Hello from an assistant", "at": 0.5 }),
    );

    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert_eq!(described["sequence"]["width"], 640);
    // The two seconds asked for; the title (3 s from 0.5 s) runs past it.
    let picture_end = described["picture_lanes"][0]["clips"][0]["end"]
        .as_f64()
        .unwrap();
    assert!(
        (picture_end - 2.0).abs() < 0.05,
        "the clip ends at {picture_end} s"
    );
    let length = described["sequence"]["duration"].as_f64().unwrap();
    assert!((length - 3.5).abs() < 0.05, "the timeline is {length} s");
    assert_eq!(
        described["title_lanes"][0]["clips"][0]["text"],
        "Hello from an assistant"
    );

    // Split, then take it back.
    client.ok("split_clip", json!({ "clip_id": picture, "at": 1.0 }));
    let split: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert_eq!(
        split["picture_lanes"][0]["clips"].as_array().unwrap().len(),
        2
    );
    client.ok("undo", json!({}));
    let back: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert_eq!(
        back["picture_lanes"][0]["clips"].as_array().unwrap().len(),
        1
    );

    // A split outside the clip is refused, and says so.
    let (_, outside) = client.tool("split_clip", json!({ "clip_id": picture, "at": 30.0 }));
    assert!(outside);

    client.ok("save_project", json!({}));

    // Opened fresh, the edit is all there.
    let mut again = Client::new();
    again.ok("open_project", json!({ "path": project }));
    let reopened: Value = serde_json::from_str(&again.ok("describe_project", json!({}))).unwrap();
    assert_eq!(
        reopened["picture_lanes"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reopened["title_lanes"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Delete the title, export what is left.
    let title = reopened["title_lanes"][0]["clips"][0]["clip_id"]
        .as_str()
        .unwrap()
        .to_owned();
    again.ok("delete_clip", json!({ "clip_id": title }));
    let output = dir.join("edit.mp4");
    let (text, failed) = again.tool("export", json!({ "path": output.display().to_string() }));
    if failed && (text.contains("adapter") || text.contains("GPU") || text.contains("gpu")) {
        eprintln!("no GPU here; skipping the export: {text}");
    } else {
        assert!(!failed, "export failed: {text}");
        let exported: Value = serde_json::from_str(&text).unwrap();
        assert!(exported["frames"].as_u64().unwrap() >= 55, "{text}");
        assert!(std::fs::metadata(&output).unwrap().len() > 10_000);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The everyday controls: volume, opacity, speed, reverse, fades, a
/// transition, a filter, moving, trimming and captions.
#[test]
fn an_assistant_uses_the_everyday_controls() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("ntsc-2997.mp4")] }),
    ))
    .unwrap();
    let media = imported[0]["media_id"].as_str().unwrap().to_owned();
    for _ in 0..2 {
        client.ok(
            "add_to_timeline",
            json!({ "media_id": media, "from": 0.0, "to": 2.0 }),
        );
    }
    let state = |client: &mut Client| -> Value {
        serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap()
    };
    let before = state(&mut client);
    let first = before["picture_lanes"][0]["clips"][0]["clip_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = before["picture_lanes"][0]["clips"][1]["clip_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sound = before["sound_lanes"][0]["clips"][0]["clip_id"]
        .as_str()
        .unwrap()
        .to_owned();

    client.ok("set_volume", json!({ "clip_id": sound, "volume": 0.5 }));
    let (_, wrong_lane) = client.tool("set_volume", json!({ "clip_id": first, "volume": 0.5 }));
    assert!(wrong_lane, "a picture clip has no volume");
    client.ok("set_opacity", json!({ "clip_id": first, "opacity": 0.25 }));
    client.ok("set_fades", json!({ "clip_id": first, "fade_in": 0.5 }));
    client.ok(
        "add_transition",
        json!({ "clip_id": first, "kind": "fade through black", "duration": 0.5 }),
    );
    let (unknown, bad_kind) = client.tool(
        "add_transition",
        json!({ "clip_id": first, "kind": "teleport" }),
    );
    assert!(bad_kind && unknown.contains("Crossfade"), "{unknown}");
    client.ok(
        "apply_filter",
        json!({ "clip_ids": [second], "filter": "black and white" }),
    );
    client.ok("reverse_clip", json!({ "clip_id": second }));

    let after = state(&mut client);
    let pictures = &after["picture_lanes"][0]["clips"];
    assert_eq!(after["sound_lanes"][0]["clips"][0]["volume"], 0.5);
    assert_eq!(pictures[0]["opacity"], 0.25);
    assert_eq!(pictures[0]["transition"]["kind"], "Fade through black");
    assert!((pictures[0]["transition"]["duration"].as_f64().unwrap() - 0.5).abs() < 0.05);
    assert_eq!(pictures[1]["filter"], "B&W");
    assert_eq!(pictures[1]["reversed"], true);

    // Twice as fast is half as long.
    client.ok("set_speed", json!({ "clip_id": second, "speed": 2.0 }));
    let fast = state(&mut client);
    let clip = &fast["picture_lanes"][0]["clips"][1];
    let length = clip["end"].as_f64().unwrap() - clip["start"].as_f64().unwrap();
    assert!(
        (length - 1.0).abs() < 0.05,
        "a 2 s clip at 2x lasts {length} s"
    );
    assert_eq!(clip["speed"], 2.0);

    // Trim the first clip's end in, then move the second later along.
    client.ok(
        "trim_clip",
        json!({ "clip_id": first, "edge": "end", "to": 1.5 }),
    );
    client.ok("move_clip", json!({ "clip_id": second, "start": 6.0 }));
    let moved = state(&mut client);
    let pictures = &moved["picture_lanes"][0]["clips"];
    assert!((pictures[0]["end"].as_f64().unwrap() - 1.5).abs() < 0.05);
    assert!((pictures[1]["start"].as_f64().unwrap() - 6.0).abs() < 0.05);

    let srt = dir.join("words.srt");
    std::fs::write(
        &srt,
        "1\n00:00:00,000 --> 00:00:01,000\nHello\n\n2\n00:00:01,000 --> 00:00:02,000\nThere\n",
    )
    .unwrap();
    let text = client.ok(
        "import_captions",
        json!({ "path": srt.display().to_string() }),
    );
    assert!(text.starts_with('2'), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
