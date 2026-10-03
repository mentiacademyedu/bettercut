//! The live link: an assistant attaches to an open window and edits the
//! project there. The "window" here is a thread holding an editor and
//! serving the link the way the app does between frames.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_editor_core::Editor;
use bettercut_mcp::Server;
use bettercut_mcp::live::Listener;
use serde_json::{Value, json};

fn call(server: &mut Server, name: &str, arguments: Value) -> (String, bool) {
    let line = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": { "name": name, "arguments": arguments } });
    let reply: Value =
        serde_json::from_str(&server.handle_line(&line.to_string()).unwrap()).unwrap();
    let result = &reply["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        result["isError"].as_bool().unwrap_or(true),
    )
}

fn ok(server: &mut Server, name: &str, arguments: Value) -> String {
    let (text, is_error) = call(server, name, arguments);
    assert!(!is_error, "{name} failed: {text}");
    text
}

#[test]
fn an_assistant_edits_the_open_window() {
    let dir = std::env::temp_dir().join(format!("bettercut-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("live.json");
    let mut server = Server::with_live_file(file.clone());

    // No window yet.
    let (text, is_error) = call(&mut server, "attach_to_app", json!({}));
    assert!(is_error);
    assert!(text.contains("not open"), "{text}");

    // The window: an editor, serving calls between "frames".
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, started) = std::sync::mpsc::channel();
    let window = {
        let stop = stop.clone();
        let file = file.clone();
        std::thread::spawn(move || {
            let (mut editor, _events) = Editor::new_project("In The Window");
            let listener = Listener::start(file, || {}).unwrap();
            ready.send(()).unwrap();
            let mut said = Vec::new();
            // What the person has selected, kept as the app keeps it.
            let mut selected = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let served = listener.serve(&mut editor, &selected);
                said.extend(served.last_change);
                if let Some(clips) = served.select {
                    selected = clips;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            // The window's crash recovery brings back what the assistant did,
            // undo included, exactly.
            let recovered = bettercut_editor_core::recover(editor.recovery_paths().clone())
                .expect("the window's edits are in recovery");
            assert_eq!(recovered.failed, 0);
            assert_eq!(
                serde_json::to_value(&recovered.project).unwrap(),
                serde_json::to_value(editor.project()).unwrap(),
                "recovery differs from the window's project"
            );
            // Unsaved, so it lives in the machine's temp folder: removed, or
            // the next launch of the real app would offer this test's work.
            recovered.discard();
            (editor.project().clone(), said)
        })
    };
    started.recv().unwrap();

    // Someone else on the machine, without the token, is turned away.
    let said: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let mut stranger =
        std::net::TcpStream::connect(("127.0.0.1", said["port"].as_u64().unwrap() as u16)).unwrap();
    writeln!(stranger, r#"{{"token":"guess","tool":"undo","args":{{}}}}"#).unwrap();
    let mut answer = String::new();
    BufReader::new(stranger).read_line(&mut answer).unwrap();
    assert!(answer.contains("wrong token"), "{answer}");

    let attached = ok(&mut server, "attach_to_app", json!({}));
    assert!(attached.contains("In The Window"), "{attached}");

    // Opening another project while attached is refused, not done quietly.
    let (_, is_error) = call(
        &mut server,
        "new_project",
        json!({ "path": dir.join("x.vproj").display().to_string() }),
    );
    assert!(is_error);

    // Edits land in the window's editor, and undo there.
    ok(
        &mut server,
        "add_title",
        json!({ "text": "Hello", "at": 0 }),
    );
    ok(
        &mut server,
        "add_title",
        json!({ "text": "Again", "at": 5 }),
    );
    ok(&mut server, "undo", json!({}));
    let described: Value =
        serde_json::from_str(&ok(&mut server, "describe_project", json!({}))).unwrap();
    assert!(described.to_string().contains("Hello"), "{described}");
    assert!(!described.to_string().contains("Again"), "{described}");

    // Pointing at a clip in the window, and reading back what is selected.
    let hello = described["title_lanes"][0]["clips"][0]["clip_id"].clone();
    let (text, is_error) = call(&mut server, "get_selection", json!({}));
    assert!(!is_error, "{text}");
    let nothing: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(nothing["selected"], json!([]), "{nothing}");
    ok(&mut server, "select_clips", json!({ "clip_ids": [hello] }));
    ok(&mut server, "set_playhead", json!({ "at": 1.25 }));
    let now: Value = serde_json::from_str(&ok(&mut server, "get_selection", json!({}))).unwrap();
    assert_eq!(now["selected"][0]["kind"], "title", "{now}");
    assert_eq!(now["selected"][0]["name"], "Hello", "{now}");
    assert!(
        (now["playhead"].as_f64().unwrap() - 1.25).abs() < 0.05,
        "{now}"
    );

    // Export renders a copy, away from the window's thread.
    let out = dir.join("out.mp4");
    let exported = ok(
        &mut server,
        "export",
        json!({ "path": out.display().to_string(), "width": 320, "height": 180 }),
    );
    assert!(exported.contains("frames"), "{exported}");
    assert!(std::fs::metadata(&out).unwrap().len() > 0);

    // In the background, from a copy the window hands over.
    let later = dir.join("later.mp4").display().to_string();
    ok(
        &mut server,
        "export",
        json!({ "path": later, "width": 320, "height": 180, "wait": false }),
    );
    let mut state = Value::Null;
    for _ in 0..600 {
        let all: Value =
            serde_json::from_str(&ok(&mut server, "export_status", json!({}))).unwrap();
        state = all[0].clone();
        if state["state"] != "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(state["state"], "finished", "{state}");

    ok(&mut server, "detach_from_app", json!({}));
    let (text, is_error) = call(&mut server, "describe_project", json!({}));
    assert!(is_error, "detached, there is no project of its own: {text}");
    let (text, is_error) = call(&mut server, "get_selection", json!({}));
    assert!(is_error && text.contains("attach_to_app"), "{text}");

    stop.store(true, Ordering::Relaxed);
    let (project, said) = window.join().unwrap();
    // The window says what changed, not what was only looked at.
    assert!(said.iter().any(|s| s == "add title"), "{said:?}");
    assert!(!said.iter().any(|s| s.contains("describe")), "{said:?}");
    let seen = serde_json::to_string(&project).unwrap();
    assert!(seen.contains("Hello") && !seen.contains("Again"));
    // The window closing takes its file with it.
    assert!(!file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
