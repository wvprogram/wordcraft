//! An MCP client doing realistic tasks through the protocol only, checking results via MCP.

use serde_json::{Value, json};

use crate::{Headless, Server};

fn call(s: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let r = s.handle_line(&line).expect("reply");
    serde_json::from_str(&r).expect("json")
}

fn tool(s: &mut Server, name: &str, args: Value) -> Value {
    let r = call(s, 9, "tools/call", json!({"name": name, "arguments": args}));
    let res = &r["result"];
    assert_eq!(res["isError"], false, "{name}: {res}");
    let text = res["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
}

#[test]
fn lifecycle_and_tools() {
    let mut s = Server::new(Box::new(Headless::default()));
    let init = call(&mut s, 1, "initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(init["result"]["serverInfo"]["name"], "wordcraft");
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    let tools = call(&mut s, 2, "tools/list", json!({}));
    assert!(tools["result"]["tools"].as_array().unwrap().len() >= 12);
    let bad = call(&mut s, 3, "nope", json!({}));
    assert!(bad["error"].is_object());
    assert!(s.handle_line("{not json").unwrap().contains("parse error"));
}

#[test]
fn agent_writes_a_formatted_document() {
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "type_text", json!({"text": "Quarterly Report"}));
    tool(&mut s, "execute", json!({"command": "para.style", "params": {"style": "Heading 1"}}));
    tool(&mut s, "execute", json!({"command": "text.newParagraph"}));
    tool(&mut s, "type_text", json!({"text": "Sales grew strongly this quarter.\nCosts fell.", "paragraphs": true}));
    tool(&mut s, "select_text", json!({"text": "strongly"}));
    tool(&mut s, "execute", json!({"command": "format.bold"}));
    tool(&mut s, "batch", json!({"commands": [{"command": "caret.docEnd"}, {"command": "insert.table", "params": {"rows": 2, "cols": 2}}]}));
    let doc = tool(&mut s, "inspect_document", json!({}));
    let blocks = doc["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["style"], "Heading1");
    assert_eq!(blocks[0]["text"], "Quarterly Report");
    let runs = blocks[1]["runs"].as_array().unwrap();
    assert!(runs.iter().any(|r| r["props"]["bold"] == true));
    assert!(blocks.iter().any(|b| b["type"] == "table"));
    let text = tool(&mut s, "get_text", json!({}));
    assert!(text["text"].as_str().unwrap().contains("Costs fell."));
    let r = call(&mut s, 10, "tools/call", json!({"name": "render_page", "arguments": {"page": 1, "scale": 0.5}}));
    assert_eq!(r["result"]["content"][0]["type"], "image");
    let r = call(&mut s, 11, "tools/call", json!({"name": "screenshot", "arguments": {}}));
    assert_eq!(r["result"]["isError"], true);
    let p = tool(&mut s, "parity", json!({}));
    assert!(p["percent"].as_f64().unwrap() > 50.0);
}

#[test]
fn resources() {
    let mut s = Server::new(Box::new(Headless::default()));
    let l = call(&mut s, 1, "resources/list", json!({}));
    assert_eq!(l["result"]["resources"].as_array().unwrap().len(), 2);
    let r = call(&mut s, 2, "resources/read", json!({"uri": "wordcraft://document"}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("blocks"));
}

/// An agent steered by a hostile document must not read or overwrite files outside the server's
/// folder: through `open_document`, `save_document`, `execute`, `batch`, or a path nested in a
/// recorded macro. Before `--root`, `file.open` read any text file and `file.exportPng`
/// overwrote any file.
#[test]
fn agents_cannot_reach_files_outside_the_root() {
    let base = std::env::temp_dir().join(format!("wordcraft-mcp-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, private) = (base.join("work"), base.join("private"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&private).unwrap();
    let secret = private.join("passwords.txt");
    std::fs::write(&secret, "bank pin 4321").unwrap();
    let notes = private.join("notes.md");
    std::fs::write(&notes, "my notes").unwrap();
    let mut s = Server::with_paths(Box::new(Headless::default()), crate::PathPolicy::within(&root).unwrap());
    let refused = |s: &mut Server, name: &str, args: Value| {
        let r = call(s, 20, "tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(r["result"]["isError"], true, "{name} {args}: {r}");
        assert!(r["result"]["content"][0]["text"].as_str().unwrap_or("").contains("outside"), "{r}");
    };
    let (secret_s, notes_s) = (secret.to_string_lossy().to_string(), notes.to_string_lossy().to_string());
    refused(&mut s, "open_document", json!({"path": secret_s}));
    refused(&mut s, "open_document", json!({"path": "../private/passwords.txt"}));
    refused(&mut s, "save_document", json!({"path": notes_s}));
    refused(&mut s, "save_document", json!({"path": "../private/out.png"}));
    refused(&mut s, "execute", json!({"command": "file.exportPng", "params": {"path": notes_s}}));
    refused(&mut s, "execute", json!({"command": "insert.textFromFile", "params": {"path": secret_s}}));
    refused(&mut s, "batch", json!({"commands": [{"command": "file.exportPdf", "params": {"path": notes_s}}]}));
    refused(
        &mut s,
        "execute",
        json!({"command": "tools.macros", "params": {"define": "m", "steps": [{"command": "file.open", "params": {"path": secret_s}}]}}),
    );
    let text = tool(&mut s, "get_text", json!({}));
    assert!(!text.to_string().contains("4321"), "the secret was not read");
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "my notes", "the file outside was not overwritten");

    // Inside the folder everything works, with relative paths too.
    tool(&mut s, "type_text", json!({"text": "Inside the root"}));
    tool(&mut s, "save_document", json!({"path": "report.docx"}));
    assert!(root.join("report.docx").exists());
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "open_document", json!({"path": "report.docx"}));
    assert!(tool(&mut s, "get_text", json!({})).to_string().contains("Inside the root"));
    let _ = std::fs::remove_dir_all(&base);
}
