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

/// Whatever a test did, every edit reached the autosave journal: an edit that
/// cannot be written there is lost in a crash, and says only "autosave failed
/// once". Checked as each test's session ends — and with it, every tool.
impl Drop for Client {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(
                self.server.autosave_failures(),
                0,
                "an edit could not be written to the autosave journal"
            );
            // And what was written brings the project back as it is.
            if let Some(difference) = self.server.recovery_differs() {
                panic!("crash recovery would not bring the edit back: {difference}");
            }
            // And the project file holds all of it.
            if let Some(difference) = self.server.file_round_trip_differs() {
                panic!("saving and opening the project changes it: {difference}");
            }
            // Last, since it rewrites history: undo it all, redo it all.
            if let Some(difference) = self.server.undo_redo_all_differs() {
                panic!("undo and redo do not come back to the same project: {difference}");
            }
        }
    }
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

    // Ready-made requests, offered as prompts.
    assert!(init["result"]["capabilities"]["prompts"].is_object());
    let prompts = client.request("prompts/list", json!({}));
    let names = prompts["result"]["prompts"].to_string();
    assert!(
        names.contains("tighten_interview") && names.contains("make_short"),
        "{names}"
    );
    let got = client.request(
        "prompts/get",
        json!({ "name": "title_card", "arguments": { "title": "Summer" } }),
    );
    let text = got["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("\"Summer\""), "{text}");
    let missing = client.request("prompts/get", json!({ "name": "title_card" }));
    assert_eq!(missing["error"]["code"], -32602);
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
    // Checked while the recovery data is still on disk.
    drop(again);
    drop(client);
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
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An assistant can look at its edit: the frame comes back as a PNG image,
/// at most as wide as asked.
#[test]
fn an_assistant_sees_a_frame() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp3-{}", std::process::id()));
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
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    );

    let reply = client.request(
        "tools/call",
        json!({ "name": "preview_frame", "arguments": { "at": 0.5, "max_width": 320 } }),
    );
    let result = &reply["result"];
    if result["isError"] == true {
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("adapter") || text.to_lowercase().contains("gpu"),
            "{text}"
        );
        eprintln!("no GPU here; skipping: {text}");
        return;
    }
    assert_eq!(result["content"][0]["type"], "image");
    assert_eq!(result["content"][0]["mimeType"], "image/png");
    let data = result["content"][0]["data"].as_str().unwrap();
    // A PNG's signature, base64-encoded, begins "iVBORw0KGgo".
    assert!(
        data.starts_with("iVBORw0KGgo"),
        "{}",
        &data[..16.min(data.len())]
    );
    let caption = result["content"][1]["text"].as_str().unwrap();
    assert!(caption.contains("320x180"), "{caption}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_places_styles_and_marks() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp4-{}", std::process::id()));
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
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let picture = placed["clip_ids"][0].clone();
    let title: Value =
        serde_json::from_str(&client.ok("add_title", json!({ "text": "Hi", "at": 0 }))).unwrap();
    let title = title["clip_id"].clone();

    client.ok(
        "set_transform",
        json!({ "clip_id": picture, "x": 0.25, "scale": 0.5, "rotation": 10 }),
    );
    client.ok("set_transform", json!({ "clip_id": title, "y": 0.3 }));
    client.ok(
        "style_title",
        json!({ "clip_id": title, "text": "Hello", "size": 120, "color": "#ff8800",
                "bold": true, "outline": "#000000", "background": "#00000080" }),
    );
    client.ok(
        "adjust_colour",
        json!({ "clip_id": picture, "brightness": 1.2, "saturation": 0 }),
    );
    // Between two frames: it lands on one, named.
    client.ok("add_marker", json!({ "at": 1.51, "label": "drop" }));

    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    let shot = &described["picture_lanes"][0]["clips"][0];
    assert_eq!(shot["place"]["x"], 0.25, "{shot}");
    assert_eq!(shot["place"]["y"], 0.0, "y was left as it was: {shot}");
    assert_eq!(shot["place"]["scale"], 0.5);
    assert_eq!(shot["place"]["rotation"], 10.0);
    let words = &described["title_lanes"][0]["clips"][0];
    assert_eq!(words["text"], "Hello", "{words}");
    assert_eq!(words["size"], 120.0);
    assert_eq!(words["color"], "#ff8800");
    assert!((words["place"]["y"].as_f64().unwrap() - 0.3).abs() < 1e-6);
    assert_eq!(described["markers"][0]["label"], "drop");
    assert!((described["markers"][0]["at"].as_f64().unwrap() - 1.5).abs() < 1e-3);

    // The steps are listed, newest first, under the editor's own names.
    let steps: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(steps["undo"][0], "Change markers", "{steps}");
    assert_eq!(steps["undo"][1], "Adjust Colour", "{steps}");
    assert!(steps["redo"].as_array().unwrap().is_empty());

    // Each tool is one undo step: the named marker, then the colour as a
    // whole, then the title's words and look together.
    client.ok("undo", json!({}));
    client.ok("undo", json!({}));
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert!(described["markers"].as_array().unwrap().is_empty());
    let words = &described["title_lanes"][0]["clips"][0];
    assert_eq!(words["text"], "Hello", "the style is still there: {words}");
    client.ok("undo", json!({}));
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    let words = &described["title_lanes"][0]["clips"][0];
    assert_eq!(
        words["text"], "Hi",
        "words and style were one step: {words}"
    );

    // Mistakes are explained.
    let (text, is_error) = client.tool("style_title", json!({ "clip_id": picture }));
    assert!(is_error && text.contains("not a title"), "{text}");
    let (text, is_error) = client.tool(
        "style_title",
        json!({ "clip_id": title, "color": "orange" }),
    );
    assert!(is_error && text.contains("#rrggbb"), "{text}");
    let (text, is_error) = client.tool("set_transform", json!({ "clip_id": picture }));
    assert!(is_error && text.contains("nothing"), "{text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_moves_photos_and_animates_titles() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp5-{}", std::process::id()));
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
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let picture = placed["clip_ids"][0].clone();
    let title: Value =
        serde_json::from_str(&client.ok("add_title", json!({ "text": "Hi", "at": 0 }))).unwrap();
    let title = title["clip_id"].clone();

    client.ok(
        "set_movement",
        json!({ "clip_id": picture, "movement": "Zoom in", "strength": "gentle" }),
    );
    client.ok(
        "animate_title",
        json!({ "clip_id": title, "intro": "slide-up", "outro": "fade", "looping": "pulse" }),
    );
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    let shot = &described["picture_lanes"][0]["clips"][0];
    assert_eq!(shot["movement"], "Zoom in", "{shot}");
    let words = &described["title_lanes"][0]["clips"][0];
    assert_eq!(words["intro"], "Slide up", "{words}");
    assert_eq!(words["outro"], "Fade");
    assert_eq!(words["looping"], "Pulse");

    // "none" takes them off again; the rest stays.
    client.ok(
        "animate_title",
        json!({ "clip_id": title, "outro": "none" }),
    );
    client.ok(
        "set_movement",
        json!({ "clip_id": picture, "movement": "none" }),
    );
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert!(described["picture_lanes"][0]["clips"][0]["movement"].is_null());
    let words = &described["title_lanes"][0]["clips"][0];
    assert!(words["outro"].is_null(), "{words}");
    assert_eq!(words["intro"], "Slide up");

    let (text, is_error) = client.tool(
        "set_movement",
        json!({ "clip_id": picture, "movement": "barrel roll" }),
    );
    assert!(
        is_error && text.contains("Pan left"),
        "the choices are listed: {text}"
    );
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_writes_captions() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp6-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let said = client.ok(
        "add_captions",
        json!({ "lines": [
            { "start": 2, "end": 4, "text": "Second" },
            { "start": 0, "end": 2.5, "text": "First" },
            { "start": 5, "end": 4.5, "text": "Backwards" }
        ] }),
    );
    assert!(
        said.contains("2 captions added") && said.contains("1 could not"),
        "{said}"
    );
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    let all = described["title_lanes"].to_string();
    let first = all.find("First").expect("first is there");
    let second = all.find("Second").expect("second is there");
    assert!(first < second, "sorted by time: {all}");
    assert!(!all.contains("Backwards"));

    // One undo takes the lot.
    client.ok("undo", json!({}));
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert!(!described["title_lanes"].to_string().contains("First"));

    let (text, is_error) = client.tool(
        "add_captions",
        json!({ "lines": [{ "start": 1, "end": 2 }] }),
    );
    assert!(is_error && text.contains("no text"), "{text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_animates_with_keyframes() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp7-{}", std::process::id()));
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
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let picture = placed["clip_ids"][0].clone();
    let animated = |client: &mut Client| -> Value {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["picture_lanes"][0]["clips"][0]["animated"].clone()
    };

    let said = client.ok(
        "animate",
        json!({ "clip_id": picture, "property": "opacity",
                "keys": [{ "at": 0, "value": 0 }, { "at": 0.5, "value": 1, "easing": "ease out" }] }),
    );
    assert!(said.contains("2 keys"), "{said}");
    client.ok(
        "animate",
        json!({ "clip_id": picture, "property": "Scale",
                "keys": [{ "at": 0, "value": 1 }, { "at": 0.9, "value": 1.3, "easing": "ease in-out" }] }),
    );
    assert_eq!(animated(&mut client), json!(["opacity", "scale"]));

    // Replacing: three keys now, not five.
    client.ok(
        "animate",
        json!({ "clip_id": picture, "property": "opacity",
                "keys": [{ "at": 0, "value": 1 }, { "at": 0.3, "value": 0.2 }, { "at": 0.6, "value": 1 }] }),
    );
    // One undo per call: back to the two-key fade, still animated.
    client.ok("undo", json!({}));
    assert_eq!(animated(&mut client), json!(["opacity", "scale"]));
    // An empty list takes it off.
    client.ok(
        "animate",
        json!({ "clip_id": picture, "property": "opacity", "keys": [] }),
    );
    assert_eq!(animated(&mut client), json!(["scale"]));

    let (text, is_error) = client.tool(
        "animate",
        json!({ "clip_id": picture, "property": "opacity", "keys": [{ "at": 60, "value": 1 }] }),
    );
    assert!(is_error && text.contains("outside the clip"), "{text}");
    let (text, is_error) = client.tool(
        "animate",
        json!({ "clip_id": picture, "property": "volume", "keys": [] }),
    );
    assert!(is_error && text.contains("sound_lanes"), "{text}");
    let (text, is_error) = client.tool(
        "animate",
        json!({ "clip_id": picture, "property": "wobble", "keys": [] }),
    );
    assert!(
        is_error && text.contains("rotation"),
        "the choices are listed: {text}"
    );
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_builds_from_a_template() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let listed: Value = serde_json::from_str(&client.ok("list_templates", json!({}))).unwrap();
    let intro = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["template_id"] == "quick-intro")
        .expect("the starters are listed");
    let slots = intro["slots"].to_string();
    assert!(
        slots.contains("\"main\"") && slots.contains("\"text\""),
        "{slots}"
    );

    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("ntsc-2997.mp4")] }),
    ))
    .unwrap();
    let applied: Value = serde_json::from_str(&client.ok(
        "apply_template",
        json!({ "template_id": "Quick Intro",
                "fills": { "main": imported[0]["media_id"], "title": "Hello there" } }),
    ))
    .unwrap();
    assert!(
        !applied["clip_ids"].as_array().unwrap().is_empty(),
        "{applied}"
    );
    assert_eq!(applied["unfilled_slots"], json!(["music"]), "{applied}");
    let described = client.ok("describe_project", json!({}));
    assert!(described.contains("Hello there"), "{described}");

    // One undo takes the whole template away.
    client.ok("undo", json!({}));
    let described = client.ok("describe_project", json!({}));
    assert!(!described.contains("Hello there"), "{described}");

    let (text, is_error) = client.tool(
        "apply_template",
        json!({ "template_id": "quick-intro", "fills": { "nope": "x" } }),
    );
    assert!(
        is_error && text.contains("main"),
        "the slots are named: {text}"
    );
    let (text, is_error) = client.tool("apply_template", json!({ "template_id": "no-such" }));
    assert!(is_error && text.contains("list_templates"), "{text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_fixes_the_sound() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp9-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("tone-48k.wav"), fixture("still.png")] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let tone = placed["clip_ids"][0].clone();
    let volume = |client: &mut Client| -> f64 {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["sound_lanes"][0]["clips"][0]["volume"]
            .as_f64()
            .unwrap()
    };

    let said = client.ok("normalise_volume", json!({ "clip_id": tone }));
    assert!(said.contains("dB"), "{said}");
    assert!(
        (volume(&mut client) - 1.0).abs() > 1e-3,
        "the level changed: {said}"
    );

    client.ok("enhance_voice", json!({ "clip_id": tone }));
    client.ok("mute_clip", json!({ "clip_id": tone }));
    let (text, is_error) = client.tool("mute_clip", json!({ "clip_id": tone }));
    assert!(is_error && text.contains("already"), "{text}");
    client.ok("mute_clip", json!({ "clip_id": tone, "muted": false }));

    // Nothing else plays over it: nothing to duck under, and it says so.
    let (text, is_error) = client.tool("duck_under_voice", json!({ "clip_id": tone }));
    assert!(is_error && text.contains("nothing"), "{text}");

    // A photo has no sound.
    let photo: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[1]["media_id"] }),
    ))
    .unwrap();
    let (text, is_error) = client.tool(
        "normalise_volume",
        json!({ "clip_id": photo["clip_ids"][0] }),
    );
    assert!(is_error && text.contains("no sound"), "{text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A mono 16-bit WAV: a tone for each `true` second, silence for each `false`.
fn talk_and_pauses(path: &std::path::Path, pattern: &[(f64, bool)]) {
    let rate = 48_000_u32;
    let mut samples: Vec<i16> = Vec::new();
    for &(seconds, loud) in pattern {
        for i in 0..(seconds * f64::from(rate)) as usize {
            let t = i as f64 / f64::from(rate);
            let v = if loud {
                (t * 440.0 * std::f64::consts::TAU).sin() * 0.5
            } else {
                0.0
            };
            samples.push((v * f64::from(i16::MAX)) as i16);
        }
    }
    let data = samples.len() as u32 * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn an_assistant_cuts_the_pauses() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp10-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("talk.wav");
    talk_and_pauses(&wav, &[(1.0, true), (1.5, false), (1.0, true)]);
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [wav.display().to_string()] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let talk = placed["clip_ids"][0].clone();
    let length = |client: &mut Client| -> f64 {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["sequence"]["duration"].as_f64().unwrap()
    };
    let before = length(&mut client);

    let found: Value = serde_json::from_str(&client.ok(
        "remove_silences",
        json!({ "clip_id": talk, "preview": true }),
    ))
    .unwrap();
    let pauses = found["pauses"].as_array().unwrap();
    assert_eq!(pauses.len(), 1, "{found}");
    let start = pauses[0]["start"].as_f64().unwrap();
    let end = pauses[0]["end"].as_f64().unwrap();
    // The pause is 1.0–2.5 s; the padding keeps a little either side.
    assert!(
        start > 1.0 && start < 1.3 && end > 2.2 && end < 2.5,
        "{found}"
    );
    assert!(
        (length(&mut client) - before).abs() < 1e-6,
        "a preview cuts nothing"
    );

    let said = client.ok("remove_silences", json!({ "clip_id": talk }));
    assert!(said.contains("1 pauses cut"), "{said}");
    let after = length(&mut client);
    assert!(
        (before - after - (end - start)).abs() < 0.05,
        "{before} -> {after}"
    );
    client.ok("undo", json!({}));
    assert!((length(&mut client) - before).abs() < 1e-6, "one undo step");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_splits_scenes_and_marks_beats() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp11-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();

    // Two shots, made into one file by an export: a red photo, then colour
    // bars.
    client.ok(
        "new_project",
        json!({ "path": dir.join("make.vproj").display().to_string(), "width": 320, "height": 180 }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("exif-6.jpg"), fixture("ntsc-2997.mp4")] }),
    ))
    .unwrap();
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    );
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[1]["media_id"] }),
    );
    let made: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    let photo_ends = made["picture_lanes"][0]["clips"][0]["end"]
        .as_f64()
        .unwrap();
    let two_shots = dir.join("two-shots.mp4");
    client.ok("export", json!({ "path": two_shots.display().to_string() }));

    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [two_shots.display().to_string()] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let clip = placed["clip_ids"][0].clone();
    let found: Value = serde_json::from_str(&client.ok(
        "split_at_scenes",
        json!({ "clip_id": clip, "preview": true }),
    ))
    .unwrap();
    let changes = found["scene_changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{found}");
    assert!(
        (changes[0].as_f64().unwrap() - photo_ends).abs() < 0.1,
        "{found}, the photo ends at {photo_ends}"
    );
    client.ok("split_at_scenes", json!({ "clip_id": clip }));
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert_eq!(
        described["picture_lanes"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "{described}"
    );

    // A click track at 120 beats a minute.
    let wav = dir.join("clicks.wav");
    let mut pattern = Vec::new();
    for _ in 0..24 {
        pattern.push((0.04, true));
        pattern.push((0.46, false));
    }
    talk_and_pauses(&wav, &pattern);
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [wav.display().to_string()] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let said = client.ok("mark_beats", json!({ "clip_id": placed["clip_ids"][0] }));
    assert!(said.contains("120 BPM"), "{said}");
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert!(
        described["markers"].as_array().unwrap().len() >= 10,
        "{said}"
    );
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_lays_out_and_reshapes() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp12-{}", std::process::id()));
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
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let shot = placed["clip_ids"][0].clone();
    let describe = |client: &mut Client| -> Value {
        serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap()
    };
    let colour: Value = serde_json::from_str(&client.ok(
        "add_colour",
        json!({ "color": "#102030", "to": "#000000", "at": 0 }),
    ))
    .unwrap();
    assert!(colour["clip_id"].is_string(), "{colour}");
    let before = describe(&mut client)["sequence"]["duration"]
        .as_f64()
        .unwrap();

    client.ok(
        "picture_in_picture",
        json!({ "clip_id": shot, "corner": "top left", "size": "small" }),
    );
    let lanes = describe(&mut client)["picture_lanes"].to_string();
    assert!(lanes.contains("\"x\":-"), "moved left: {lanes}");
    client.ok(
        "picture_in_picture",
        json!({ "clip_id": shot, "size": "full" }),
    );

    // Last: the freeze opens a gap by cutting the shot, so its id is spent.
    client.ok(
        "freeze_frame",
        json!({ "clip_id": shot, "at": 0.5, "duration": 1.5 }),
    );
    let after = describe(&mut client)["sequence"]["duration"]
        .as_f64()
        .unwrap();
    assert!((after - before - 1.5).abs() < 0.05, "{before} -> {after}");

    let copied: Value =
        serde_json::from_str(&client.ok("copy_as_shape", json!({ "shape": "9:16" }))).unwrap();
    let now = describe(&mut client);
    let (w, h) = (
        now["sequence"]["width"].as_u64().unwrap(),
        now["sequence"]["height"].as_u64().unwrap(),
    );
    assert!(h > w, "vertical: {w}x{h}");
    let sequences = now["sequences"].as_array().unwrap();
    assert_eq!(sequences.len(), 2, "{now}");
    let first = sequences
        .iter()
        .find(|s| s["sequence_id"] != copied["sequence_id"])
        .unwrap()["sequence_id"]
        .clone();
    client.ok("switch_sequence", json!({ "sequence_id": first }));
    let back = describe(&mut client);
    assert!(
        back["sequence"]["width"].as_u64().unwrap() > back["sequence"]["height"].as_u64().unwrap()
    );

    let (text, is_error) = client.tool("copy_as_shape", json!({ "shape": "triangle" }));
    assert!(is_error && text.contains("9:16"), "{text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_adds_graphics() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp13-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let third: Value = serde_json::from_str(&client.ok(
        "add_lower_third",
        json!({ "name": "Ada Lovelace", "role": "Analyst", "at": 1, "accent": "#3366ff" }),
    ))
    .unwrap();
    assert_eq!(third["clip_ids"].as_array().unwrap().len(), 3, "{third}");
    let shape: Value = serde_json::from_str(&client.ok(
        "add_shape",
        json!({ "shape": "rounded rectangle", "at": 2 }),
    ))
    .unwrap();
    client.ok(
        "style_title",
        json!({ "clip_id": shape["clip_id"], "color": "#ff0000" }),
    );
    client.ok("add_sticker", json!({ "sticker": "Music note", "at": 0 }));
    client.ok("add_sticker", json!({ "sticker": "\u{2605}", "at": 3 }));
    client.ok("add_timer", json!({ "at": 0, "direction": "up" }));

    let described = client.ok("describe_project", json!({}));
    assert!(
        described.contains("Ada Lovelace") && described.contains("Analyst"),
        "{described}"
    );
    assert!(described.contains("#ff0000"), "{described}");

    let (text, is_error) = client.tool("add_sticker", json!({ "sticker": "unicorn", "at": 0 }));
    assert!(is_error && text.contains("Football"), "{text}");
    let (text, is_error) = client.tool("add_lower_third", json!({ "name": " ", "at": 0 }));
    assert!(is_error, "a lower third needs a name: {text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_sets_effects() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp14-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("p.vproj");
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": project.display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("ntsc-2997.mp4")] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let picture = placed["clip_ids"][0].clone();
    let sound = placed["clip_ids"][1].clone();

    client.ok(
        "set_effect",
        json!({ "clip_id": picture, "effect": "Glow", "amount": 40 }),
    );
    client.ok(
        "set_effect",
        json!({ "clip_id": picture, "effect": "old-film", "amount": 70 }),
    );
    client.ok(
        "set_effect",
        json!({ "clip_id": picture, "effect": "vignette", "amount": 50 }),
    );
    // The assistant can read back what is on.
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    let effects = &described["picture_lanes"][0]["clips"][0]["effects"];
    assert_eq!(effects["glow"], 40.0, "{effects}");
    assert_eq!(effects["old film"], 70.0, "{effects}");
    assert_eq!(effects["vignette"], 50.0, "{effects}");
    assert!(effects.get("blur").is_none(), "only what is on: {effects}");
    client.ok("save_project", json!({}));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    let text = saved.to_string();
    assert!(text.contains("\"glow\":40"), "glow is saved");
    assert!(text.contains("\"old_film\":70"), "old film is saved");
    assert!(
        text.contains("\"vignette\":0.5"),
        "vignette in its own 0-1 range"
    );

    let (text, is_error) = client.tool(
        "set_effect",
        json!({ "clip_id": picture, "effect": "sparkles", "amount": 10 }),
    );
    assert!(is_error && text.contains("pixelate"), "{text}");
    let (text, is_error) = client.tool(
        "set_effect",
        json!({ "clip_id": sound, "effect": "glow", "amount": 10 }),
    );
    assert!(is_error && text.contains("picture"), "{text}");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Ask export_status until `path` is no longer running; its last word.
fn finished_export(client: &mut Client, path: &str) -> Value {
    for _ in 0..600 {
        let all: Value = serde_json::from_str(&client.ok("export_status", json!({}))).unwrap();
        if let Some(one) = all.as_array().unwrap().iter().find(|e| e["path"] == path)
            && one["state"] != "running"
        {
            return one.clone();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("the export of {path} never finished");
}

#[test]
fn an_assistant_exports_in_the_background() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp15-{}", std::process::id()));
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
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    );
    let out = dir.join("later.mp4").display().to_string();
    let said = client.ok(
        "export",
        json!({ "path": out, "width": 320, "height": 180, "wait": false }),
    );
    assert!(said.contains("background"), "{said}");
    // The session goes on being usable while it renders.
    client.ok("add_title", json!({ "text": "Meanwhile", "at": 0 }));
    let done = finished_export(&mut client, &out);
    assert_eq!(done["state"], "finished", "{done}");
    assert!(done["summary"]["frames"].as_u64().unwrap() > 0, "{done}");
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_keys_a_green_screen() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp16-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("p.vproj");
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": project.display().to_string() }),
    );
    let screen: Value =
        serde_json::from_str(&client.ok("add_colour", json!({ "color": "#00ff00", "at": 0 })))
            .unwrap();
    let screen = screen["clip_id"].clone();
    let keyed = |client: &mut Client| -> Value {
        client.ok("save_project", json!({}));
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
        // The one picture clip's key, wherever the file keeps it.
        fn find(v: &Value) -> Option<Value> {
            match v {
                Value::Object(map) => map
                    .get("chroma_key")
                    .cloned()
                    .or_else(|| map.values().find_map(find)),
                Value::Array(items) => items.iter().find_map(find),
                _ => None,
            }
        }
        find(&saved).expect("a clip with a chroma_key field")
    };

    client.ok(
        "green_screen",
        json!({ "clip_id": screen, "tolerance": 0.2 }),
    );
    let key = keyed(&mut client);
    assert!(
        (key["tolerance"].as_f64().unwrap() - 0.2).abs() < 1e-6,
        "{key}"
    );
    assert_eq!(key["color"], json!([0.0, 1.0, 0.0]), "{key}");

    client.ok(
        "green_screen",
        json!({ "clip_id": screen, "color": "#0000ff" }),
    );
    assert_eq!(keyed(&mut client)["color"], json!([0.0, 0.0, 1.0]));

    client.ok("green_screen", json!({ "clip_id": screen, "off": true }));

    // Cropping the same clip: only the edges given change, and it is held
    // to leave something.
    let said = client.ok(
        "crop",
        json!({ "clip_id": screen, "left": 0.1, "right": 0.2 }),
    );
    assert!(
        said.contains("left 0.10") && said.contains("right 0.20"),
        "{said}"
    );
    let said = client.ok(
        "crop",
        json!({ "clip_id": screen, "top": 0.9, "bottom": 0.9 }),
    );
    assert!(said.contains("left 0.10"), "kept: {said}");
    assert!(
        !said.contains("top 0.90, right 0.20, bottom 0.90"),
        "held apart: {said}"
    );
    let said = client.ok(
        "crop",
        json!({ "clip_id": screen, "left": 0, "top": 0, "right": 0, "bottom": 0 }),
    );
    assert_eq!(said, "Crop off");
    assert!(keyed(&mut client).is_null(), "the key is off");
    // Checked while the recovery data is still on disk.
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_file_ending_picks_what_is_exported() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp17-{}", std::process::id()));
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
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"], "to": 0.5 }),
    );

    let gif = dir.join("loop.gif");
    let (text, failed) = client.tool("export", json!({ "path": gif.display().to_string() }));
    if failed && (text.contains("adapter") || text.to_lowercase().contains("gpu")) {
        eprintln!("no GPU here; skipping: {text}");
    } else {
        assert!(!failed, "{text}");
        let bytes = std::fs::read(&gif).unwrap();
        assert!(bytes.starts_with(b"GIF89a"), "a GIF");
        let wide = u16::from_le_bytes([bytes[6], bytes[7]]);
        assert_eq!(wide, 480, "small by default");
    }

    let wav = dir.join("sound.wav");
    let text = client.ok("export", json!({ "path": wav.display().to_string() }));
    let bytes = std::fs::read(&wav).unwrap();
    assert!(
        bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE",
        "{text}"
    );
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_sees_the_whole_edit_at_a_glance() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp18-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string(), "width": 640, "height": 360 }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("ntsc-2997.mp4")] }),
    ))
    .unwrap();
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    );
    let reply = client.request(
        "tools/call",
        json!({ "name": "contact_sheet", "arguments": { "count": 6, "tile_width": 160 } }),
    );
    let result = &reply["result"];
    if result["isError"] == true {
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("adapter") || text.to_lowercase().contains("gpu"),
            "{text}"
        );
        eprintln!("no GPU here; skipping: {text}");
    } else {
        assert_eq!(result["content"][0]["type"], "image");
        let caption = result["content"][1]["text"].as_str().unwrap();
        assert!(caption.starts_with("6 frames"), "{caption}");
        let times = caption.rsplit(" at ").next().unwrap_or_default();
        assert_eq!(times.split(", ").count(), 6, "six times: {caption}");
    }
    let (text, is_error) = {
        let mut empty = Client::new();
        empty.ok(
            "new_project",
            json!({ "path": dir.join("e.vproj").display().to_string() }),
        );
        let out = empty.tool("contact_sheet", json!({}));
        drop(empty);
        out
    };
    assert!(is_error && text.contains("empty"), "{text}");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_batch_is_one_undo_step() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp19-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let before: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    let steps_before = before["undo"].as_array().unwrap().len();

    let done: Value = serde_json::from_str(&client.ok(
        "batch",
        json!({ "label": "Opening titles", "calls": [
            { "tool": "add_title", "args": { "text": "One", "at": 0 } },
            { "tool": "add_title", "args": { "text": "Two", "at": 4 } },
            { "tool": "add_marker", "args": { "at": 2, "label": "beat" } }
        ] }),
    ))
    .unwrap();
    assert_eq!(done["undo_step"], "Opening titles", "{done}");
    assert_eq!(done["results"].as_array().unwrap().len(), 3);
    let after: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(
        after["undo"].as_array().unwrap().len(),
        steps_before + 1,
        "{after}"
    );
    assert_eq!(after["undo"][0], "Opening titles");

    // One undo takes all three back.
    client.ok("undo", json!({}));
    let described = client.ok("describe_project", json!({}));
    assert!(
        !described.contains("\"One\"") && !described.contains("beat"),
        "{described}"
    );
    client.ok("redo", json!({}));
    let described = client.ok("describe_project", json!({}));
    assert!(
        described.contains("\"Two\"") && described.contains("beat"),
        "{described}"
    );

    // A failure stops it, and what went before is kept, as one step.
    let (text, is_error) = client.tool(
        "batch",
        json!({ "calls": [
            { "tool": "add_title", "args": { "text": "Kept", "at": 8 } },
            { "tool": "split_clip", "args": { "clip_id": "not-an-id", "at": 1 } },
            { "tool": "add_title", "args": { "text": "Never", "at": 9 } }
        ] }),
    );
    assert!(is_error && text.contains("call 2 of 3"), "{text}");
    let described = client.ok("describe_project", json!({}));
    assert!(
        described.contains("Kept") && !described.contains("Never"),
        "{described}"
    );

    let (text, is_error) = client.tool(
        "batch",
        json!({ "calls": [{ "tool": "export", "args": { "path": "x.mp4" } }] }),
    );
    assert!(
        is_error && text.contains("cannot be part of a batch"),
        "{text}"
    );
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_writes_chapters_and_subtitles() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp20-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    // Thirty seconds of edit: a colour clip held long enough.
    let colour: Value =
        serde_json::from_str(&client.ok("add_colour", json!({ "color": "#203040", "at": 0 })))
            .unwrap();
    client.ok(
        "trim_clip",
        json!({ "clip_id": colour["clip_id"], "edge": "end", "to": 40 }),
    );
    client.ok("add_marker", json!({ "at": 12, "label": "The middle" }));
    client.ok("add_marker", json!({ "at": 25, "label": "The end" }));
    let chapters: Value = serde_json::from_str(&client.ok("chapters", json!({}))).unwrap();
    let text = chapters["text"].as_str().unwrap();
    assert!(text.starts_with("0:00"), "{text}");
    assert!(
        text.contains("0:12 The middle") && text.contains("0:25 The end"),
        "{text}"
    );

    client.ok(
        "add_captions",
        json!({ "lines": [
            { "start": 0, "end": 2, "text": "Hello" },
            { "start": 2, "end": 4, "text": "And goodbye" }
        ] }),
    );
    let srt = dir.join("subs.srt");
    let said = client.ok(
        "export_captions",
        json!({ "path": srt.display().to_string() }),
    );
    assert!(said.starts_with("2 captions"), "{said}");
    let written = std::fs::read_to_string(&srt).unwrap();
    assert!(
        written.contains("00:00:02,000 --> 00:00:04,000"),
        "{written}"
    );
    assert!(written.contains("And goodbye"), "{written}");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_rearranges_the_edit() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp21-{}", std::process::id()));
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
    let mut pictures = Vec::new();
    for (from, to) in [(0.0, 0.6), (0.6, 1.2), (1.2, 1.8)] {
        let placed: Value = serde_json::from_str(&client.ok(
            "add_to_timeline",
            json!({ "media_id": imported[0]["media_id"], "from": from, "to": to }),
        ))
        .unwrap();
        pictures.push(placed["clip_ids"][0].clone());
    }
    let (a, b, c) = (&pictures[0], &pictures[1], &pictures[2]);
    let lane = |client: &mut Client| -> Vec<Value> {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["picture_lanes"][0]["clips"]
            .as_array()
            .unwrap()
            .clone()
    };

    let said = client.ok(
        "transition_every_cut",
        json!({ "clip_id": a, "kind": "wipe" }),
    );
    assert_eq!(said, "Wipe on 2 cuts; 0 skipped for want of footage");
    let said = client.ok(
        "transition_every_cut",
        json!({ "clip_id": a, "kind": "none" }),
    );
    assert!(said.starts_with("2 transitions"), "{said}");

    // A hole in the middle, then closed.
    client.ok("delete_clip", json!({ "clip_id": b }));
    let described: Value = serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
    assert_eq!(
        described["sound_lanes"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "the shot's sound went with it: {described}"
    );
    let said = client.ok("close_gaps", json!({ "clip_id": c }));
    assert_eq!(said, "1 gaps closed");
    let clips = lane(&mut client);
    assert_eq!(clips[1]["clip_id"], *c);
    assert!(
        (clips[1]["start"].as_f64().unwrap() - 0.6).abs() < 0.04,
        "{clips:?}"
    );
    let (text, is_error) = client.tool("close_gaps", json!({ "clip_id": c }));
    assert!(is_error, "no gap left: {text}");

    let looped: Value =
        serde_json::from_str(&client.ok("loop_clip", json!({ "clip_id": a, "times": 3 }))).unwrap();
    assert_eq!(looped["clip_ids"].as_array().unwrap().len(), 2, "{looped}");
    let (_, is_error) = client.tool("loop_clip", json!({ "clip_id": a, "times": 50 }));
    assert!(is_error, "more than 20 loops is refused");

    let back: Value =
        serde_json::from_str(&client.ok("boomerang", json!({ "clip_id": c }))).unwrap();
    let clips = lane(&mut client);
    let reversed = clips
        .iter()
        .find(|clip| clip["clip_id"] == back["reversed_clip_id"])
        .expect("the reversed copy is on the lane");
    assert_eq!(reversed["reversed"], true);

    let said = client.ok("group_clips", json!({ "clip_ids": [a, c] }));
    assert!(said.contains("one group"), "{said}");
    let said = client.ok("group_clips", json!({ "clip_ids": [a], "ungroup": true }));
    assert_eq!(said, "1 groups taken apart");
    let (text, is_error) = client.tool("group_clips", json!({ "clip_ids": [a], "ungroup": true }));
    assert!(is_error && text.contains("no"), "{text}");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_takes_out_a_stretch_and_holds_the_end() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp23-{}", std::process::id()));
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
    let mut pictures = Vec::new();
    for (from, to) in [(0.0, 0.6), (0.6, 1.2), (1.2, 1.8)] {
        let placed: Value = serde_json::from_str(&client.ok(
            "add_to_timeline",
            json!({ "media_id": imported[0]["media_id"], "from": from, "to": to }),
        ))
        .unwrap();
        pictures.push(placed["clip_ids"][0].clone());
    }
    let described = |client: &mut Client| -> Value {
        serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap()
    };
    let length = |client: &mut Client| -> f64 {
        described(client)["sequence"]["duration"].as_f64().unwrap()
    };
    let full = length(&mut client);

    let said = client.ok("remove_range", json!({ "from": 0.3, "to": 0.9 }));
    assert!(said.contains("gap closed"), "{said}");
    assert!((length(&mut client) - (full - 0.6)).abs() < 0.04);
    let history: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(history["undo"][0], "Remove Range", "{history}");
    client.ok("undo", json!({}));
    assert!(
        (length(&mut client) - full).abs() < 0.04,
        "one undo puts it back"
    );

    // Lifted: a hole, the same length.
    client.ok(
        "remove_range",
        json!({ "from": 0.3, "to": 0.9, "close_gap": false }),
    );
    assert!((length(&mut client) - full).abs() < 0.04);
    let lane = described(&mut client)["picture_lanes"][0]["clips"].clone();
    assert!(
        (lane[1]["start"].as_f64().unwrap() - 0.9).abs() < 0.04,
        "{lane}"
    );
    let (text, is_error) = client.tool("remove_range", json!({ "from": 2, "to": 1 }));
    assert!(is_error && text.contains("after"), "{text}");

    let said = client.ok("split_into", json!({ "clip_id": pictures[2], "parts": 4 }));
    assert_eq!(said, "3 cuts made");
    let (text, is_error) = client.tool("split_into", json!({ "clip_id": pictures[2] }));
    assert!(is_error && text.contains("either"), "{text}");

    let before = length(&mut client);
    let lane = described(&mut client)["picture_lanes"][0]["clips"].clone();
    let last = lane.as_array().unwrap().last().unwrap()["clip_id"].clone();
    client.ok(
        "hold_last_frame",
        json!({ "clip_id": last, "duration": 1.5 }),
    );
    assert!((length(&mut client) - (before + 1.5)).abs() < 0.04);
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_sets_the_whole_video_look() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp24-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let look = |client: &mut Client| -> Value {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["sequence"]["whole_video"].clone()
    };
    assert!(look(&mut client)["bars"].is_null());

    let said = client.ok(
        "whole_video_look",
        json!({ "bars": "2.39:1", "progress_bar": "thick", "progress_color": "#00ff00",
                "progress_top": true, "vignette": 40, "grain": 10, "background": "#102030" }),
    );
    assert!(said.contains("bars at 2.39:1"), "{said}");
    let now = look(&mut client);
    assert!((now["bars"].as_f64().unwrap() - 2.39).abs() < 1e-4, "{now}");
    assert_eq!(now["progress_bar"]["color"], "#00ff00", "{now}");
    assert_eq!(now["progress_bar"]["top"], true, "{now}");
    assert_eq!(now["vignette"], 40.0, "{now}");
    assert_eq!(now["grain"], 10.0, "{now}");

    let history: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(history["undo"][0], "Whole Video Look", "{history}");
    client.ok("undo", json!({}));
    let now = look(&mut client);
    assert!(
        now["bars"].is_null() && now["progress_bar"].is_null() && now["vignette"] == 0.0,
        "one undo takes it all back: {now}"
    );
    client.ok("redo", json!({}));
    client.ok(
        "whole_video_look",
        json!({ "bars": "none", "progress_bar": "none" }),
    );
    let now = look(&mut client);
    assert!(
        now["bars"].is_null() && now["progress_bar"].is_null(),
        "{now}"
    );

    let (text, is_error) = client.tool("whole_video_look", json!({ "bars": "square" }));
    assert!(is_error && text.contains("2.39"), "{text}");
    let (text, is_error) = client.tool("whole_video_look", json!({}));
    assert!(is_error && text.contains("nothing"), "{text}");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_sets_up_the_lanes() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp25-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("tone-48k.wav")] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let described = |client: &mut Client| -> Value {
        serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap()
    };
    let sound_lane = described(&mut client)["sound_lanes"][0]["lane"]
        .as_str()
        .unwrap()
        .to_owned();

    let said = client.ok(
        "set_lane",
        json!({ "clip_id": placed["clip_ids"][0], "rename": "Music",
                "volume": 0.5, "locked": true }),
    );
    assert!(
        said.contains("renamed") && said.contains("locked"),
        "{said}"
    );
    let lane = described(&mut client)["sound_lanes"][0].clone();
    assert_eq!(lane["lane"], "Music", "{lane}");
    assert_eq!(lane["locked"], true, "{lane}");
    assert_eq!(lane["volume"], 0.5, "{lane}");
    let history: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(history["undo"][0], "Change Lane", "{history}");
    client.ok("undo", json!({}));
    let lane = described(&mut client)["sound_lanes"][0].clone();
    assert_eq!(lane["lane"], sound_lane.as_str(), "one undo: {lane}");
    assert_eq!(lane["locked"], false, "{lane}");
    client.ok("redo", json!({}));

    // A locked lane can still be renamed, and unlocked by name.
    client.ok(
        "set_lane",
        json!({ "lane": "music", "rename": "Song", "locked": false, "on": false }),
    );
    let lane = described(&mut client)["sound_lanes"][0].clone();
    assert!(
        lane["lane"] == "Song" && lane["locked"] == false && lane["on"] == false,
        "{lane}"
    );

    let title_lane = described(&mut client)["title_lanes"][0]["lane"].clone();
    let (text, is_error) = client.tool("set_lane", json!({ "lane": title_lane, "volume": 1 }));
    assert!(is_error && text.contains("not a sound lane"), "{text}");
    let (text, is_error) = client.tool("set_lane", json!({ "lane": "nowhere", "solo": true }));
    assert!(is_error && text.contains("no lane"), "{text}");
    let (text, is_error) = client.tool("set_lane", json!({ "lane": "Song" }));
    assert!(is_error && text.contains("nothing"), "{text}");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_tidies_the_media() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp26-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("tone-48k.wav"), fixture("still.png")] }),
    ))
    .unwrap();
    client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    );
    let media = |client: &mut Client| -> Vec<Value> {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["media"].as_array().unwrap().clone()
    };

    let said = client.ok(
        "organise_media",
        json!({ "media_id": imported[0]["media_id"], "rename": "Theme tune", "bin": "Music" }),
    );
    assert_eq!(said, "Now called \"Theme tune\", in the \"Music\" bin");
    let tune = media(&mut client)[0].clone();
    assert!(
        tune["name"] == "Theme tune" && tune["bin"] == "Music",
        "{tune}"
    );
    let history: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(history["undo"][0], "Organise Media", "{history}");

    let (text, is_error) = client.tool(
        "organise_media",
        json!({ "media_id": imported[0]["media_id"] }),
    );
    assert!(is_error && text.contains("nothing"), "{text}");

    // The photo was never used.
    let said = client.ok("remove_unused_media", json!({}));
    assert!(said.starts_with("1 unused"), "{said}");
    assert_eq!(media(&mut client).len(), 1);
    let said = client.ok("remove_unused_media", json!({}));
    assert!(said.contains("nothing removed"), "{said}");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assistant_shapes_the_sound() {
    let dir = std::env::temp_dir().join(format!("bettercut-mcp22-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = Client::new();
    client.ok(
        "new_project",
        json!({ "path": dir.join("p.vproj").display().to_string() }),
    );
    let imported: Value = serde_json::from_str(&client.ok(
        "import_media",
        json!({ "paths": [fixture("tone-48k.wav")] }),
    ))
    .unwrap();
    let placed: Value = serde_json::from_str(&client.ok(
        "add_to_timeline",
        json!({ "media_id": imported[0]["media_id"] }),
    ))
    .unwrap();
    let tone = placed["clip_ids"][0].clone();
    let sound = |client: &mut Client| -> Value {
        let described: Value =
            serde_json::from_str(&client.ok("describe_project", json!({}))).unwrap();
        described["sound_lanes"][0]["clips"][0].clone()
    };

    let said = client.ok(
        "shape_sound",
        json!({ "clip_id": tone, "eq": "phone", "voice": "deep",
                "space": "hall", "space_amount": 50, "robot": 20 }),
    );
    assert!(said.contains("hall at 50"), "{said}");
    let shaped = sound(&mut client);
    assert_eq!(shaped["eq"], "Phone", "{shaped}");
    assert_eq!(shaped["pitch"], -5.0, "{shaped}");
    assert_eq!(shaped["space"]["kind"], "Hall", "{shaped}");
    assert_eq!(shaped["space"]["amount"], 50.0, "{shaped}");
    assert_eq!(shaped["robot"], 20.0, "{shaped}");

    // All of it was one step.
    let history: Value = serde_json::from_str(&client.ok("history", json!({}))).unwrap();
    assert_eq!(history["undo"][0], "Shape Sound", "{history}");
    client.ok("undo", json!({}));
    let plain = sound(&mut client);
    assert!(plain["eq"].is_null() && plain["space"].is_null(), "{plain}");
    assert_eq!(plain["pitch"], 0.0);
    client.ok("redo", json!({}));

    let (text, is_error) = client.tool("shape_sound", json!({ "clip_id": tone }));
    assert!(is_error && text.contains("nothing"), "{text}");
    let (text, is_error) = client.tool("shape_sound", json!({ "clip_id": tone, "eq": "tin can" }));
    assert!(is_error && text.contains("Megaphone"), "{text}");
    client.ok(
        "shape_sound",
        json!({ "clip_id": tone, "eq": "flat", "pitch": 3.04 }),
    );
    let shaped = sound(&mut client);
    assert!(shaped["eq"].is_null(), "{shaped}");
    assert!(
        (shaped["pitch"].as_f64().unwrap() - 3.0).abs() < 1e-4,
        "{shaped}"
    );

    // Music held to where the pictures end, with a fade.
    let colour: Value =
        serde_json::from_str(&client.ok("add_colour", json!({ "color": "#000000", "at": 0 })))
            .unwrap();
    client.ok(
        "trim_clip",
        json!({ "clip_id": colour["clip_id"], "edge": "end", "to": 0.5 }),
    );
    let said = client.ok("fit_music", json!({ "clip_id": tone }));
    assert!(said.contains("0.500"), "{said}");
    let (_, is_error) = client.tool("fit_music", json!({ "clip_id": colour["clip_id"] }));
    assert!(is_error, "a picture is not music");
    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}
