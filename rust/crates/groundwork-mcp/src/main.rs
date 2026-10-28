//! Groundwork MCP server.
//!
//! Speaks the Model Context Protocol over stdio: newline-delimited JSON-RPC
//! 2.0 on stdin/stdout, logging on stderr. Holds one open world in memory.

mod args;
mod base64;
mod tools;

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "\
Groundwork edits Minecraft maps as a height map plus painted terrain and layers, then exports a playable \
Minecraft Java Edition 1.21 save. Coordinates: x runs east, y runs south (Minecraft's z axis); heights are \
Minecraft y levels. A world is made of 128×128-block tiles; only columns inside tiles can be edited. \
Typical flow: create_world → sculpt (raise_lower, flatten, smooth) → paint_terrain / paint_layer → flood for \
lakes → render_preview to check → save_world → export_minecraft. Every edit can be reverted with undo.";

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut server = tools::Server::default();
    eprintln!("groundwork-mcp {} ready", env!("CARGO_PKG_VERSION"));

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("stdin error: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let responses = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => batch
                .into_iter()
                .filter_map(|m| handle(&mut server, m))
                .collect(),
            Ok(message) => handle(&mut server, message).into_iter().collect(),
            Err(e) => vec![error_response(
                Value::Null,
                -32700,
                &format!("parse error: {e}"),
            )],
        };
        for response in responses {
            // Writing can only fail if the client went away.
            if writeln!(stdout, "{response}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                return;
            }
        }
    }
}

/// Handles one JSON-RPC message; returns a response for requests only.
fn handle(server: &mut tools::Server, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str);
    let Some(method) = method else {
        // A response from the client (we never send requests) or garbage.
        return id.map(|id| error_response(id, -32600, "invalid request"));
    };
    let Some(id) = id else {
        // Notification: notifications/initialized, notifications/cancelled, ...
        return None;
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v == requested)
                .unwrap_or(&SUPPORTED_PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "groundwork", "title": "Groundwork", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools::definitions() }),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_response(
                    id,
                    -32602,
                    "tools/call requires a tool name",
                ));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            server.call(name, &arguments)
        }
        _ => {
            return Some(error_response(
                id,
                -32601,
                &format!("method not found: {method}"),
            ))
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(server: &mut tools::Server, id: i64, method: &str, params: Value) -> Value {
        handle(
            server,
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .unwrap()
    }

    #[test]
    fn handshake_and_listing() {
        let mut server = tools::Server::default();
        let init = request(
            &mut server,
            1,
            "initialize",
            json!({ "protocolVersion": "2025-03-26", "capabilities": {} }),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(
            &mut server,
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
        )
        .is_none());
        let list = request(&mut server, 2, "tools/list", json!({}));
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"create_world") && names.contains(&"export_minecraft"));
        let unknown = request(&mut server, 3, "resources/list", json!({}));
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
