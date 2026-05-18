//! MCP server over stdio. JSON-RPC 2.0, one message per line.
//!
//! Tools exposed:
//!   - `index_status`     — counts + last-indexed-at
//!   - `get_skeleton`     — file skeletons at multiple detail levels
//!   - `run_pipeline`     — search + impact for a free-form task
//!   - `save_observation` — write to the FTS-backed memory store

use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::graph::GraphStore;
use crate::impact;
use crate::indexer::Indexer;
use crate::memory::Memory;
use crate::search;
use crate::skeleton::{self, DetailLevel};

const SERVER_NAME: &str = "speedy-language-context";
const SERVER_VERSION: &str = "0.1.0";
const PROTOCOL_VERSION: &str = "2025-03-26";

pub async fn run_server(workspace_root: PathBuf) -> Result<()> {
    let store = Arc::new(GraphStore::open(&workspace_root)?);
    let memory = Arc::new(Memory::open(&workspace_root)?);
    let indexer = Arc::new(Indexer::new(&workspace_root)?);

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
            "tools/call" => {
                handle_tools_call(id.clone(), params, &store, &memory, &indexer, &workspace_root)
                    .await
            }
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
                    "name": "index_status",
                    "description": "Get the current indexing status",
                    "inputSchema": { "type": "object", "properties": {} }
                },
                {
                    "name": "get_skeleton",
                    "description": "Get file skeletons at configurable detail levels",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "files": { "type": "array", "items": { "type": "string" } },
                            "detail": { "type": "string", "enum": ["minimal", "standard", "detailed"] }
                        },
                        "required": ["files"]
                    }
                },
                {
                    "name": "run_pipeline",
                    "description": "Run a full analysis pipeline for a given task",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "task": { "type": "string" },
                            "preset": { "type": "string", "enum": ["auto", "explore", "modify", "debug", "refactor"] },
                            "top_k": { "type": "number" }
                        },
                        "required": ["task"]
                    }
                },
                {
                    "name": "save_observation",
                    "description": "Save an observation about the codebase",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "text": { "type": "string" } },
                        "required": ["text"]
                    }
                },
                {
                    "name": "search_observations",
                    "description": "Full-text search over saved observations",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "limit": { "type": "number" }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "force_reindex",
                    "description": "Force a full reindex of the workspace and return updated stats",
                    "inputSchema": { "type": "object", "properties": {} }
                },
                {
                    "name": "workspace_add",
                    "description": "Add a directory to the speedy workspace registry",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }
                },
                {
                    "name": "workspace_remove",
                    "description": "Remove a directory from the speedy workspace registry",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }
                },
                {
                    "name": "workspace_list",
                    "description": "List all registered speedy workspaces",
                    "inputSchema": { "type": "object", "properties": {} }
                }
            ]
        }
    })
}

async fn handle_tools_call(
    id: Value,
    params: Value,
    store: &Arc<GraphStore>,
    memory: &Arc<Memory>,
    indexer: &Arc<Indexer>,
    workspace_root: &std::path::Path,
) -> Value {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    let text_result = match name {
        "index_status" => match tool_index_status(store, indexer, workspace_root).await {
            Ok(t) => t,
            Err(e) => return error_response(id, -32603, &format!("index_status failed: {e}")),
        },
        "get_skeleton" => match tool_get_skeleton(store, args, workspace_root) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("get_skeleton failed: {e}")),
        },
        "run_pipeline" => match tool_run_pipeline(store, args) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("run_pipeline failed: {e}")),
        },
        "save_observation" => match tool_save_observation(memory, args) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("save_observation failed: {e}")),
        },
        "search_observations" => match tool_search_observations(memory, args) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("search_observations failed: {e}")),
        },
        "force_reindex" => match tool_force_reindex(store, indexer).await {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("force_reindex failed: {e}")),
        },
        "workspace_add" => match tool_run_cli(&["workspace", "add", args.get("path").and_then(|v| v.as_str()).unwrap_or(""), "--json"]) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("workspace_add failed: {e}")),
        },
        "workspace_remove" => match tool_run_cli(&["workspace", "remove", args.get("path").and_then(|v| v.as_str()).unwrap_or(""), "--json"]) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("workspace_remove failed: {e}")),
        },
        "workspace_list" => match tool_run_cli(&["workspace", "list", "--json"]) {
            Ok(t) => t,
            Err(e) => return error_response(id, -32602, &format!("workspace_list failed: {e}")),
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

async fn tool_index_status(
    store: &Arc<GraphStore>,
    indexer: &Arc<Indexer>,
    _workspace_root: &std::path::Path,
) -> Result<String> {
    let status = add_indexer_status_method(store, indexer)?;
    Ok(serde_json::to_string_pretty(&status)?)
}

fn tool_get_skeleton(
    store: &Arc<GraphStore>,
    args: Value,
    workspace_root: &std::path::Path,
) -> Result<String> {
    let files: Vec<String> = args
        .get("files")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        return Err(anyhow::anyhow!("files[] is required"));
    }
    let detail_str = args
        .get("detail")
        .and_then(|d| d.as_str())
        .unwrap_or("standard");
    let detail: DetailLevel = detail_str.parse()?;
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    skeleton::get_skeleton(store, workspace_root, &refs, detail)
}

fn tool_run_pipeline(store: &Arc<GraphStore>, args: Value) -> Result<String> {
    let task = args
        .get("task")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("task is required"))?;
    let top_k = args
        .get("top_k")
        .and_then(|n| n.as_u64())
        .unwrap_or(10) as usize;
    let preset = args
        .get("preset")
        .and_then(|p| p.as_str())
        .unwrap_or("auto");

    let results = search::search(store, task, top_k)?;
    // SearchResult.id already holds the symbol ID — no extra full-table scan needed.
    let symbol_ids: Vec<i64> = results.iter().map(|r| r.id).take(5).collect();

    let depth = match preset {
        "refactor" | "modify" => 3,
        "debug" => 2,
        _ => 1,
    };
    let impact = impact::find_impact(store, &symbol_ids, depth).unwrap_or_default();

    let payload = json!({
        "preset": preset,
        "task": task,
        "matches": results,
        "impact": impact,
    });
    Ok(serde_json::to_string_pretty(&payload)?)
}

fn tool_search_observations(memory: &Arc<Memory>, args: Value) -> Result<String> {
    let query = args
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| anyhow::anyhow!("query is required"))?;
    let limit = args
        .get("limit")
        .and_then(|n| n.as_u64())
        .unwrap_or(10) as usize;
    let entries = memory.search(query, limit)?;
    Ok(serde_json::to_string_pretty(&entries)?)
}

fn tool_save_observation(memory: &Arc<Memory>, args: Value) -> Result<String> {
    let text = args
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("text is required"))?;
    let id = memory.save(text)?;
    Ok(format!("saved observation #{id}"))
}

async fn tool_force_reindex(store: &Arc<GraphStore>, indexer: &Arc<Indexer>) -> Result<String> {
    indexer.full_index().await?;
    let status = add_indexer_status_method(store, indexer)?;
    Ok(serde_json::to_string_pretty(&status)?)
}

fn tool_run_cli(args: &[&str]) -> Result<String> {
    let bin = std::env::var("SPEEDY_BIN").unwrap_or_else(|_| "speedy-cli".to_string());
    let output = std::process::Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute {bin}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{bin} exited with {}: {stderr}", output.status)
    }
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// Public helper: returns a JSON value describing the current index state.
/// Includes live graph counts from `store` plus persisted stats from the last
/// indexing run (files_indexed, symbols_found, duration_ms stored in metadata).
pub fn add_indexer_status_method(store: &GraphStore, indexer: &Arc<Indexer>) -> Result<Value> {
    let last_indexed = store.get_meta("last_indexed_at")?.unwrap_or_else(|| "never".to_string());
    let last_files_indexed = store.get_meta("last_files_indexed")?
        .and_then(|s| s.parse::<u64>().ok());
    let last_symbols_found = store.get_meta("last_symbols_found")?
        .and_then(|s| s.parse::<u64>().ok());
    let last_index_duration_ms = store.get_meta("last_index_duration_ms")?
        .and_then(|s| s.parse::<u64>().ok());
    let workspace = indexer.root.to_string_lossy().to_string();
    Ok(json!({
        "workspace": workspace,
        "files": store.file_count()?,
        "symbols": store.symbol_count()?,
        "edges": store.edge_count()?,
        "last_indexed": last_indexed,
        "last_run": {
            "files_indexed": last_files_indexed,
            "symbols_found": last_symbols_found,
            "duration_ms": last_index_duration_ms,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn make_store(dir: &std::path::Path) -> Arc<GraphStore> {
        Arc::new(GraphStore::open(dir).unwrap())
    }

    fn make_memory(dir: &std::path::Path) -> Arc<Memory> {
        Arc::new(Memory::open(dir).unwrap())
    }

    fn make_indexer(dir: &std::path::Path) -> Arc<Indexer> {
        Arc::new(Indexer::new(dir).unwrap())
    }

    #[test]
    fn handle_initialize_returns_server_info() {
        let resp = handle_initialize(json!(1));
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn handle_tools_list_includes_all_tools() {
        let resp = handle_tools_list(json!(1));
        let tools: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(tools.contains(&"index_status"), "index_status missing");
        assert!(tools.contains(&"get_skeleton"), "get_skeleton missing");
        assert!(tools.contains(&"run_pipeline"), "run_pipeline missing");
        assert!(tools.contains(&"save_observation"), "save_observation missing");
        assert!(tools.contains(&"search_observations"), "search_observations missing");
        assert!(tools.contains(&"force_reindex"), "force_reindex missing");
        assert!(tools.contains(&"workspace_add"), "workspace_add missing");
        assert!(tools.contains(&"workspace_remove"), "workspace_remove missing");
        assert!(tools.contains(&"workspace_list"), "workspace_list missing");
    }

    #[tokio::test]
    async fn tool_index_status_returns_counts() {
        let dir = tempdir().unwrap();
        let store = make_store(dir.path());
        let memory = make_memory(dir.path());
        let indexer = make_indexer(dir.path());

        let resp = handle_tools_call(
            json!(1),
            json!({"name": "index_status", "arguments": {}}),
            &store,
            &memory,
            &indexer,
            dir.path(),
        )
        .await;

        assert_eq!(resp["id"], 1);
        let text = &resp["result"]["content"][0]["text"];
        let status: serde_json::Value = serde_json::from_str(text.as_str().unwrap()).unwrap();
        assert_eq!(status["files"], 0);
        assert_eq!(status["symbols"], 0);
        assert_eq!(status["edges"], 0);
        assert_eq!(status["last_indexed"], "never");
        assert!(status.get("last_run").is_some(), "last_run key must be present");
        assert!(status.get("workspace").is_some(), "workspace key must be present");
    }

    #[tokio::test]
    async fn tool_index_status_shows_last_run_stats_after_indexing() {
        let dir = tempdir().unwrap();
        // Write a small Rust file so the indexer has something to process.
        std::fs::write(dir.path().join("lib.rs"), b"pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();

        let store = make_store(dir.path());
        let memory = make_memory(dir.path());
        let indexer = make_indexer(dir.path());

        // Trigger a full index.
        indexer.full_index().await.unwrap();

        let resp = handle_tools_call(
            json!(1),
            json!({"name": "index_status", "arguments": {}}),
            &store,
            &memory,
            &indexer,
            dir.path(),
        )
        .await;

        let text = &resp["result"]["content"][0]["text"];
        let status: serde_json::Value = serde_json::from_str(text.as_str().unwrap()).unwrap();
        assert_ne!(status["last_indexed"], "never", "should have an indexed-at timestamp");
        let last_run = &status["last_run"];
        assert!(
            last_run["files_indexed"].as_u64().unwrap_or(0) > 0,
            "at least one file should have been indexed: {last_run}"
        );
        assert!(
            last_run["symbols_found"].as_u64().unwrap_or(0) > 0,
            "at least one symbol should have been found: {last_run}"
        );
        assert!(
            last_run["duration_ms"].as_u64().is_some(),
            "duration_ms must be present: {last_run}"
        );
    }

    #[tokio::test]
    async fn tool_save_and_search_observations() {
        let dir = tempdir().unwrap();
        let store = make_store(dir.path());
        let memory = make_memory(dir.path());
        let indexer = make_indexer(dir.path());

        handle_tools_call(
            json!(1),
            json!({"name": "save_observation", "arguments": {"text": "edge extraction was added"}}),
            &store,
            &memory,
            &indexer,
            dir.path(),
        )
        .await;

        let resp = handle_tools_call(
            json!(2),
            json!({"name": "search_observations", "arguments": {"query": "edge extraction"}}),
            &store,
            &memory,
            &indexer,
            dir.path(),
        )
        .await;

        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("edge extraction was added"), "search did not return the saved observation");
    }

    #[tokio::test]
    async fn tool_force_reindex_runs_indexer() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), b"pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();

        let store = make_store(dir.path());
        let memory = make_memory(dir.path());
        let indexer = make_indexer(dir.path());

        let resp = handle_tools_call(
            json!(1),
            json!({"name": "force_reindex", "arguments": {}}),
            &store,
            &memory,
            &indexer,
            dir.path(),
        )
        .await;

        assert_eq!(resp["id"], 1);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let status: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_ne!(status["last_indexed"], "never", "should have an indexed-at timestamp after reindex");
        assert!(
            status["symbols"].as_u64().unwrap_or(0) > 0,
            "should have found symbols after reindex"
        );
    }

    #[tokio::test]
    async fn workspace_tools_fail_gracefully_without_binary() {
        let dir = tempdir().unwrap();
        let store = make_store(dir.path());
        let memory = make_memory(dir.path());
        let indexer = make_indexer(dir.path());

        // With an invalid binary name, all three workspace tools should return
        // an MCP error response rather than panicking.
        std::env::set_var("SPEEDY_BIN", "nonexistent-speedy-binary-xyz");

        for tool in ["workspace_add", "workspace_remove", "workspace_list"] {
            let args = if tool == "workspace_list" {
                json!({"name": tool, "arguments": {}})
            } else {
                json!({"name": tool, "arguments": {"path": "/tmp/test"}})
            };
            let resp = handle_tools_call(
                json!(1),
                args,
                &store,
                &memory,
                &indexer,
                dir.path(),
            )
            .await;
            assert!(resp.get("error").is_some(), "{tool} should return error when binary missing");
        }

        std::env::remove_var("SPEEDY_BIN");
    }

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let dir = tempdir().unwrap();
        let resp = handle_tools_call(
            json!(1),
            json!({"name": "nonexistent_tool", "arguments": {}}),
            &make_store(dir.path()),
            &make_memory(dir.path()),
            &make_indexer(dir.path()),
            dir.path(),
        )
        .await;
        assert!(resp.get("error").is_some(), "expected error for unknown tool");
    }
}
