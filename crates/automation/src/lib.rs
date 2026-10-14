//! A minimal MCP server (JSON-RPC 2.0, newline-delimited, over stdio). Every engine command is a
//! tool: the tool name is the command id with `.` replaced by `_`, the input is the command's
//! params object. Agents (Claude Code, etc.) drive DbCraft headlessly through this.

use std::io::{BufRead, Write};

use dbcraft_engine::{Session, commands};
use serde_json::{Value as Json, json};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn tool_name(id: &str) -> String {
    id.replace('.', "_")
}

/// Handle one JSON-RPC message. Returns `None` for notifications.
pub fn handle(session: &mut Session, msg: &Json) -> Option<Json> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(Json::as_str).unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "dbcraft", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": commands().iter().map(|c| json!({
                "name": tool_name(c.id),
                "description": format!("{} ({}). Params: {}", c.label, c.id, c.params_doc),
                "inputSchema": { "type": "object", "additionalProperties": true },
            })).collect::<Vec<_>>()
        })),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(Json::Null);
            let name = params.get("name").and_then(Json::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match commands().iter().find(|c| tool_name(c.id) == name) {
                Some(c) => Ok(match session.dispatch(c.id, &args) {
                    Ok(v) => json!({ "content": [{ "type": "text", "text": v.to_string() }] }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true }),
                }),
                None => Err((-32602, format!("unknown tool `{name}`"))),
            }
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

/// Serve until stdin closes.
pub fn serve_stdio(session: &mut Session) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Json>(&line) {
            Ok(msg) => handle(session, &msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_calls_tools() {
        let mut s = Session::new();
        let list = handle(&mut s, &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
        assert!(list["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "query_run"));

        let r = handle(&mut s, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"connection_list","arguments":{}}})).unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "[]");

        let bad = handle(&mut s, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"connection_open","arguments":{}}})).unwrap();
        assert_eq!(bad["result"]["isError"], true);
        assert!(handle(&mut s, &json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
    }
}
