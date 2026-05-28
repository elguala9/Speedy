//! MCP server over stdio. JSON-RPC 2.0, one message per line.
//!
//! Tools exposed:
//!   - `text_status`        — index stats (file/occurrence/symbol counts)
//!   - `text_query`         — find occurrences of a symbol
//!   - `text_force_reindex` — drop the index and re-index the workspace

use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::tokenize::SearchType;
use crate::{config, db, indexer, query};

const SERVER_NAME: &str = "speedy-text-context";
const SERVER_VERSION: &str = "0.1.0";
const PROTOCOL_VERSION: &str = "2025-03-26";

pub async fn run_server(workspace_root: PathBuf) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("malformed JSON-RPC line: {e}");
                continue;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        // Notifications (no `id`) — log and move on.
        if id.is_none() {
            tracing::debug!("received notification: {method}");
            continue;
        }
        let id = id.unwrap();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let response = match method {
            "initialize" => handle_initialize(id.clone()),
            "tools/list" => handle_tools_list(id.clone()),
            "tools/call" => handle_tools_call(id.clone(), params, &workspace_root),
            _ => error_response(id.clone(), -32601, &format!("method not found: {method}")),
        };

        let line = serde_json::to_string(&response)?;
        stdout.write_all(line.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

fn handle_initialize(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
        }
    })
}

fn handle_tools_list(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [
                {
                    "name": "text_status",
                    "description": "Get the text index status: file, occurrence and unique-symbol counts.",
                    "inputSchema": { "type": "object", "properties": {} }
                },
                {
                    "name": "text_query",
                    "description": "Find occurrences of a symbol in indexed text/config files. Returns file, line and column positions.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbol": { "type": "string", "description": "Symbol to search for" },
                            "type": { "type": "string", "enum": ["cased", "isolated_special", "isolated"], "description": "Match strictness (default: cased)" },
                            "ext": { "type": "string", "description": "Restrict to a file extension, e.g. md" },
                            "ignore_case": { "type": "boolean", "description": "Case-insensitive search" }
                        },
                        "required": ["symbol"]
                    }
                },
                {
                    "name": "text_force_reindex",
                    "description": "Drop the text index and re-index the whole workspace, then return updated stats.",
                    "inputSchema": { "type": "object", "properties": {} }
                }
            ]
        }
    })
}

fn handle_tools_call(id: Value, params: Value, workspace_root: &Path) -> Value {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    let text_result = match name {
        "text_status" => match tool_status(workspace_root) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32603, &format!("text_status failed: {e}")),
        },
        "text_query" => match tool_query(workspace_root, args) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("text_query failed: {e}")),
        },
        "text_force_reindex" => match tool_force_reindex(workspace_root) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("text_force_reindex failed: {e}")),
        },
        _ => return error_response(id, -32601, &format!("unknown tool: {name}")),
    };

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text_result }]
        }
    })
}

fn tool_status(root: &Path) -> Result<String> {
    let db_path = config::db_path(root);
    let conn = db::open(&db_path)?;
    db::migrate(&conn)?;
    let status = db::get_status(&conn, &db_path)?;
    Ok(serde_json::to_string_pretty(&status)?)
}

fn tool_query(root: &Path, args: Value) -> Result<String> {
    let symbol = args
        .get("symbol")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("symbol is required"))?;
    let search_type = match args.get("type").and_then(|t| t.as_str()) {
        Some("isolated") => SearchType::Isolated,
        Some("isolated_special") => SearchType::IsolatedSpecial,
        _ => SearchType::Cased,
    };
    let ext = args.get("ext").and_then(|e| e.as_str());
    let ignore_case = args
        .get("ignore_case")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);

    let db_path = config::db_path(root);
    let conn = db::open(&db_path)?;
    db::migrate(&conn)?;
    let result = query::query_json(&conn, symbol, &search_type, ext, ignore_case)?;
    Ok(serde_json::to_string_pretty(&result)?)
}

fn tool_force_reindex(root: &Path) -> Result<String> {
    let db_path = config::db_path(root);
    let mut conn = db::open(&db_path)?;
    db::migrate(&conn)?;
    indexer::index(&mut conn, root)?;
    let status = db::get_status(&conn, &db_path)?;
    Ok(serde_json::to_string_pretty(&status)?)
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp() -> std::path::PathBuf {
        // Unique-enough temp dir without Date/rand (forbidden in some contexts):
        // derive from process id + a counter env knob.
        let base = std::env::temp_dir();
        base.join(format!("speedy-text-mcp-test-{}", std::process::id()))
    }

    #[test]
    fn initialize_returns_server_info() {
        let resp = handle_initialize(json!(1));
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn tools_list_includes_all_tools() {
        let resp = handle_tools_list(json!(1));
        let tools: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(tools.contains(&"text_status"));
        assert!(tools.contains(&"text_query"));
        assert!(tools.contains(&"text_force_reindex"));
    }

    #[test]
    fn status_on_empty_workspace() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let resp = handle_tools_call(
            json!(1),
            json!({"name": "text_status", "arguments": {}}),
            &dir,
        );
        assert_eq!(resp["id"], 1);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let status: Value = serde_json::from_str(text).unwrap();
        assert_eq!(status["files"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_tool_returns_error() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let resp = handle_tools_call(
            json!(1),
            json!({"name": "nope", "arguments": {}}),
            &dir,
        );
        assert!(resp.get("error").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
