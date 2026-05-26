use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::process::Command;

const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "speedy-ai-context-mcp";
const SERVER_VERSION: &str = "0.1.0";

fn main() {
    use tracing_subscriber::prelude::*;
    let logs_dir = speedy_core::daemon_util::exe_log_dir();
    let file_appender = tracing_appender::rolling::daily(&logs_dir, "speedy-ai-context-mcp.log");
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(true).with_writer(file_writer))
        .init();

    let stdin = io::stdin();
    let reader = stdin.lock();
    let runner = |args: &[&str]| run_speedy(args);
    let lc_runner = |args: &[&str]| run_lc(args);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = process_line(&line, &runner, &lc_runner);
        if let Some(json) = response {
            let mut stdout = io::stdout().lock();
            let _ = writeln!(stdout, "{json}");
            let _ = stdout.flush();
        }

        if line.contains("\"exit\"") || line.contains("'exit'") {
            break;
        }
    }
}

fn process_line(
    line: &str,
    run_cmd: &dyn Fn(&[&str]) -> Result<String, String>,
    run_lc_cmd: &dyn Fn(&[&str]) -> Result<String, String>,
) -> Option<String> {
    let request: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            let err = JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
            return Some(serde_json::to_string(&err).expect("serialize"));
        }
    };

    let response = handle_request(&request, run_cmd, run_lc_cmd);
    response.map(|r| serde_json::to_string(&r).expect("serialize"))
}

fn handle_request(
    req: &JsonRpcRequest,
    run_cmd: &dyn Fn(&[&str]) -> Result<String, String>,
    run_lc_cmd: &dyn Fn(&[&str]) -> Result<String, String>,
) -> Option<JsonRpcResponse> {
    match req.method.as_str() {
        "initialize" => {
            let capabilities = serde_json::json!({"tools": {}});
            let result = serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": capabilities,
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION}
            });
            Some(JsonRpcResponse::success(req.id, result))
        }

        "notifications/initialized" => None,

        "tools/list" => {
            let tools = vec![
                tool_json("speedy_query",
                    "Semantic/conceptual search over the codebase. Requires an embedding model \
                     (Ollama or configured provider). Use for 'where is X logic handled?' questions \
                     when you don't know the exact symbol name. \
                     For exact keyword/symbol searches, prefer speedy_grep (no embedding required).",
                    serde_json::json!({
                        "query": {"type": "string", "description": "Natural language query"},
                        "top_k": {"type": "number", "description": "Number of results (default: 5)", "default": 5},
                        "mode": {
                            "type": "string",
                            "enum": ["full", "files"],
                            "description": "Output mode: 'full' returns chunks with text (default), 'files' returns only unique file paths (token-efficient for location queries)",
                            "default": "full"
                        }
                    }),
                    &["query"]),
                tool_json("speedy_index",
                    "Index a directory into the vector database for semantic search.",
                    serde_json::json!({
                        "path": {"type": "string", "description": "Directory to index (default: .)", "default": "."}
                    }),
                    &[]),
                tool_json("speedy_context",
                    "Show project context summary: files and chunks indexed.",
                    serde_json::json!({}),
                    &[]),
                tool_json("speedy_workspace_add",
                    "Add a directory to the speedy workspace registry.",
                    serde_json::json!({
                        "path": {"type": "string", "description": "Workspace path to add"}
                    }),
                    &["path"]),
                tool_json("speedy_workspace_remove",
                    "Remove a directory from the speedy workspace registry.",
                    serde_json::json!({
                        "path": {"type": "string", "description": "Workspace path to remove"}
                    }),
                    &["path"]),
                tool_json("speedy_workspace_list",
                    "List all registered speedy workspaces.",
                    serde_json::json!({}),
                    &[]),
                tool_json("speedy_force_reindex",
                    "Force a full reindex of a workspace.",
                    serde_json::json!({
                        "path": {"type": "string", "description": "Workspace path to reindex (default: .)", "default": "."}
                    }),
                    &[]),
                tool_json("speedy_lc_status",
                    "Show language-context index status: file/symbol/edge counts and last-indexed timestamp.",
                    serde_json::json!({
                        "path": {"type": "string", "description": "Workspace path (default: .)", "default": "."}
                    }),
                    &[]),
                tool_json("speedy_lc_skeleton",
                    "Return file skeletons from the language-context graph at a configurable detail level.",
                    serde_json::json!({
                        "files": {"type": "array", "items": {"type": "string"}, "description": "File paths relative to workspace root"},
                        "detail": {"type": "string", "enum": ["minimal", "standard", "detailed"], "description": "Detail level (default: standard)", "default": "standard"},
                        "path": {"type": "string", "description": "Workspace path (default: .)", "default": "."}
                    }),
                    &["files"]),
                tool_json("speedy_grep",
                    "Keyword/phrase search over indexed files using SQLite FTS5. \
                     Does NOT require an embedding model — works even without Ollama. \
                     Use for exact symbol names, string literals, function signatures, or \
                     structural queries. Supports FTS5 syntax: 'fn authenticate', \
                     '\"error handling\"' (phrase), 'fn*' (prefix), 'fn OR struct'.",
                    serde_json::json!({
                        "pattern": {"type": "string", "description": "FTS5 search pattern"},
                        "top_k": {"type": "number", "description": "Max results (default: 20)", "default": 20}
                    }),
                    &["pattern"]),
            ];
            Some(JsonRpcResponse::success(req.id, serde_json::json!({"tools": tools})))
        }

        "tools/call" => {
            let name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = req.params.get("arguments").unwrap_or(&serde_json::Value::Null);
            match name {
                "speedy_query" => {
                    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("full");
                    let default_top_k = std::env::var("SPEEDY_MCP_TOP_K")
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(5);
                    let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(default_top_k);
                    let cmd_args = ["query", query, "-k", &top_k.to_string(), "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => {
                            let result_text = if mode == "files" {
                                compact_to_files(&output)
                            } else {
                                output
                            };
                            Some(JsonRpcResponse::success(req.id, content_json(&result_text)))
                        }
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy query failed: {e}"))),
                    }
                }
                "speedy_index" => {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    let cmd_args = ["index", path, "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy index failed: {e}"))),
                    }
                }
                "speedy_context" => {
                    let cmd_args = ["context", "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy context failed: {e}"))),
                    }
                }
                "speedy_workspace_add" => {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    let cmd_args = ["workspace", "add", path, "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy workspace add failed: {e}"))),
                    }
                }
                "speedy_workspace_remove" => {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    let cmd_args = ["workspace", "remove", path, "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy workspace remove failed: {e}"))),
                    }
                }
                "speedy_workspace_list" => {
                    let cmd_args = ["workspace", "list", "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy workspace list failed: {e}"))),
                    }
                }
                "speedy_force_reindex" => {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    let cmd_args = ["force", "-p", path];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy force reindex failed: {e}"))),
                    }
                }
                "speedy_lc_status" => {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    let cmd_args = ["-p", path, "status", "--json"];
                    match run_lc_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy lc status failed: {e}"))),
                    }
                }
                "speedy_lc_skeleton" => {
                    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    let detail = args.get("detail").and_then(|v| v.as_str()).unwrap_or("standard");
                    let files: Vec<&str> = args
                        .get("files")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();
                    if files.is_empty() {
                        return Some(JsonRpcResponse::error(req.id, -32602, "speedy_lc_skeleton: files[] is required"));
                    }
                    let mut cmd_args = vec!["-p", path, "skeleton", "--detail", detail];
                    cmd_args.extend(files.iter().copied());
                    match run_lc_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy lc skeleton failed: {e}"))),
                    }
                }
                "speedy_grep" => {
                    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                    let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(20);
                    let cmd_args = ["grep", pattern, "-k", &top_k.to_string(), "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
                        Err(e) => Some(JsonRpcResponse::error(req.id, -32000, format!("speedy grep failed: {e}"))),
                    }
                }
                _ => Some(JsonRpcResponse::error(req.id, -32601, format!("Unknown tool: {name}"))),
            }
        }

        "shutdown" | "exit" => {
            Some(JsonRpcResponse::success(req.id, serde_json::Value::Null))
        }

        _ => Some(JsonRpcResponse::error(req.id, -32601, format!("Method not found: {}", req.method))),
    }
}

fn tool_json(name: &str, description: &str, properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required
        }
    })
}

fn content_json(text: &str) -> serde_json::Value {
    serde_json::json!({"content": [{"type": "text", "text": text}]})
}

fn compact_to_files(json_output: &str) -> String {
    let Ok(results) = serde_json::from_str::<Vec<serde_json::Value>>(json_output) else {
        return json_output.to_string();
    };
    let mut by_file: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for r in &results {
        let path = r.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let score = r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let entry = by_file.entry(path).or_insert(0.0);
        if score > *entry { *entry = score; }
    }
    let mut files: Vec<_> = by_file.into_iter().collect();
    files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let compact: Vec<serde_json::Value> = files.into_iter()
        .map(|(path, score)| serde_json::json!({"path": path, "score": score}))
        .collect();
    serde_json::to_string(&compact).unwrap_or_default()
}

fn run_speedy(args: &[&str]) -> Result<String, String> {
    // Default to the thin client so a clean `cargo install speedy-mcp` resolves
    // via PATH to the daemon-backed flow, matching the README's documented
    // contract. Tests and power users can override with SPEEDY_BIN.
    let bin = std::env::var("SPEEDY_BIN").unwrap_or_else(|_| "speedy-cli".to_string());
    let output = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute {bin}: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
        Ok(stdout.trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("{bin} exited with {}: {stdout}{stderr}", output.status))
    }
}

fn run_lc(args: &[&str]) -> Result<String, String> {
    // Invoke the speedy-language-context binary. Override with SPEEDY_LC_BIN.
    let bin = std::env::var("SPEEDY_LC_BIN")
        .unwrap_or_else(|_| "speedy-language-context".to_string());
    let output = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute {bin}: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
        Ok(stdout.trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("{bin} exited with {}: {stdout}{stderr}", output.status))
    }
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<u64>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorObj>,
}

impl JsonRpcResponse {
    fn success(id: Option<u64>, result: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".to_string(), id, result: Some(result), error: None }
    }

    fn error(id: Option<u64>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(ErrorObj { code, message: message.into(), data: None }),
        }
    }
}

#[derive(Serialize)]
struct ErrorObj {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn mock_runner(output: &'static str) -> impl Fn(&[&str]) -> Result<String, String> {
        move |args: &[&str]| {
            if args.is_empty() {
                return Err("no args".to_string());
            }
            match args[0] {
                "fail" => Err("mock error".to_string()),
                _ => Ok(output.to_string()),
            }
        }
    }

    fn parse_response(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("valid JSON response")
    }

    fn assert_success(response: &serde_json::Value, expected_result: &serde_json::Value) {
        assert_eq!(response["jsonrpc"], "2.0", "bad jsonrpc: {response}");
        assert!(response["id"].is_number(), "missing id: {response}");
        assert!(response["result"].is_object(), "missing result: {response}");
        assert!(response["error"].is_null(), "unexpected error: {response}");
        assert_eq!(&response["result"], expected_result, "result mismatch");
    }

    fn assert_error(response: &serde_json::Value, code: i64, msg_contains: &str) {
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response["id"].is_number());
        assert!(response["result"].is_null(), "unexpected result: {response}");
        assert_eq!(response["error"]["code"], code, "wrong error code");
        let msg = response["error"]["message"].as_str().unwrap_or("");
        assert!(msg.contains(msg_contains), "error message '{msg}' does not contain '{msg_contains}'");
    }

    fn send(method: &str, params: serde_json::Value) -> serde_json::Value {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });
        let json = process_line(&line.to_string(), &mock_runner("ok"), &mock_runner("ok"));
        parse_response(&json.unwrap_or_else(|| panic!("no response for {method}")))
    }

    // ── initialize ──────────────────────────────────────

    #[test]
    fn test_initialize() {
        let resp = send("initialize", serde_json::json!({}));
        assert_success(&resp, &serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "speedy-ai-context-mcp", "version": "0.1.0"}
        }));
    }

    #[test]
    fn test_initialize_includes_id() {
        let line = r#"{"jsonrpc":"2.0","id":42,"method":"initialize","params":{}}"#;
        let json = process_line(line, &mock_runner(""), &mock_runner("")).unwrap();
        let resp: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(resp["id"], 42);
    }

    // ── notifications/initialized ──────────────────────

    #[test]
    fn test_notification_returns_none() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let result = process_line(line, &mock_runner(""), &mock_runner(""));
        assert!(result.is_none(), "notifications should not produce a response");
    }

    // ── tools/list ──────────────────────────────────────

    #[test]
    fn test_tools_list_has_ten_tools() {
        let resp = send("tools/list", serde_json::json!({}));
        let tools = &resp["result"]["tools"];
        assert!(tools.is_array());
        assert_eq!(tools.as_array().unwrap().len(), 10);
    }

    #[test]
    fn test_tools_list_names() {
        let resp = send("tools/list", serde_json::json!({}));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array().unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec![
            "speedy_query",
            "speedy_index",
            "speedy_context",
            "speedy_workspace_add",
            "speedy_workspace_remove",
            "speedy_workspace_list",
            "speedy_force_reindex",
            "speedy_lc_status",
            "speedy_lc_skeleton",
            "speedy_grep",
        ]);
    }

    #[test]
    fn test_tools_list_query_schema() {
        let resp = send("tools/list", serde_json::json!({}));
        let q = &resp["result"]["tools"][0];
        assert_eq!(q["name"], "speedy_query");
        assert!(q["description"].as_str().unwrap().len() > 10);
        let schema = &q["inputSchema"];
        assert!(schema["properties"]["query"]["type"].as_str() == Some("string"));
    }

    #[test]
    fn test_tools_list_context_schema() {
        let resp = send("tools/list", serde_json::json!({}));
        let ctx = &resp["result"]["tools"][2];
        assert_eq!(ctx["name"], "speedy_context");
        let props = &ctx["inputSchema"]["properties"];
        assert!(props.as_object().map_or(true, |m| m.is_empty()));
    }

    // ── tools/call: success ──────────────────────────────

    #[test]
    fn test_call_query_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_query",
            "arguments": {"query": "find auth", "top_k": 3}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_query_default_top_k() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_query",
            "arguments": {"query": "test"}
        }));
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_index_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_index",
            "arguments": {"path": "/tmp"}
        }));
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_index_default_path() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_index",
            "arguments": {}
        }));
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_context_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_context",
            "arguments": {}
        }));
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_workspace_add_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_workspace_add",
            "arguments": {"path": "/tmp/myproject"}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_workspace_remove_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_workspace_remove",
            "arguments": {"path": "/tmp/myproject"}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_workspace_list_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_workspace_list",
            "arguments": {}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_force_reindex_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_force_reindex",
            "arguments": {"path": "/tmp/myproject"}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_force_reindex_default_path() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_force_reindex",
            "arguments": {}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn test_call_workspace_add_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_workspace_add", "arguments": {"path": "/tmp/x"}}
        });
        let json = process_line(&line.to_string(), &fail_runner, &mock_runner("ok"));
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy workspace add failed");
    }

    #[test]
    fn test_call_workspace_remove_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_workspace_remove", "arguments": {"path": "/tmp/x"}}
        });
        let json = process_line(&line.to_string(), &fail_runner, &mock_runner("ok"));
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy workspace remove failed");
    }

    #[test]
    fn test_call_workspace_list_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_workspace_list", "arguments": {}}
        });
        let json = process_line(&line.to_string(), &fail_runner, &mock_runner("ok"));
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy workspace list failed");
    }

    #[test]
    fn test_call_force_reindex_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_force_reindex", "arguments": {"path": "/tmp/x"}}
        });
        let json = process_line(&line.to_string(), &fail_runner, &mock_runner("ok"));
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy force reindex failed");
    }

    // ── tools/call: speedy_lc_status ─────────────────────

    #[test]
    fn test_call_lc_status_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_lc_status",
            "arguments": {}
        }));
        // mock_runner returns "ok" for any non-"fail" first arg;
        // the real run_lc call is bypassed by the mock in send().
        assert_eq!(resp["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn test_call_lc_status_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_lc_status", "arguments": {"path": "/tmp/x"}}
        });
        let json = process_line(&line.to_string(), &mock_runner("ok"), &fail_runner);
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy lc status failed");
    }

    // ── tools/call: speedy_lc_skeleton ───────────────────

    #[test]
    fn test_call_lc_skeleton_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_lc_skeleton",
            "arguments": {"files": ["src/main.rs"], "detail": "standard"}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn test_call_lc_skeleton_empty_files_returns_error() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_lc_skeleton",
            "arguments": {"files": []}
        }));
        assert_error(&resp, -32602, "files[] is required");
    }

    #[test]
    fn test_call_lc_skeleton_missing_files_returns_error() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_lc_skeleton",
            "arguments": {}
        }));
        assert_error(&resp, -32602, "files[] is required");
    }

    #[test]
    fn test_call_lc_skeleton_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_lc_skeleton", "arguments": {"files": ["src/lib.rs"]}}
        });
        let json = process_line(&line.to_string(), &mock_runner("ok"), &fail_runner);
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy lc skeleton failed");
    }

    // ── tools/call: errors ───────────────────────────────

    #[test]
    fn test_call_unknown_tool() {
        let resp = send("tools/call", serde_json::json!({
            "name": "nonexistent",
            "arguments": {}
        }));
        assert_error(&resp, -32601, "Unknown tool: nonexistent");
    }

    #[test]
    fn test_call_speedy_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_query", "arguments": {"query": "x"}}
        });
        let json = process_line(&line.to_string(), &fail_runner, &mock_runner("ok"));
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy query failed");
    }

    #[test]
    fn test_call_query_empty_query() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_query",
            "arguments": {"query": ""}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
    }

    // ── shutdown / exit ─────────────────────────────────

    #[test]
    fn test_shutdown() {
        let resp = send("shutdown", serde_json::json!({}));
        assert!(resp["result"].is_null(), "expected null result, got: {}", resp["result"]);
        assert!(resp["error"].is_null(), "expected no error");
    }

    #[test]
    fn test_exit() {
        let resp = send("exit", serde_json::json!({}));
        assert!(resp["result"].is_null(), "expected null result, got: {}", resp["result"]);
    }

    // ── tools/call: speedy_grep ─────────────────────────────

    #[test]
    fn test_call_grep_success() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_grep",
            "arguments": {"pattern": "fn main"}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_call_grep_with_top_k() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_grep",
            "arguments": {"pattern": "authenticate", "top_k": 5}
        }));
        assert_eq!(resp["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn test_call_grep_binary_failure() {
        let fail_runner = |_: &[&str]| Err("binary not found".to_string());
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_grep", "arguments": {"pattern": "fn main"}}
        });
        let json = process_line(&line.to_string(), &fail_runner, &mock_runner("ok"));
        let resp = parse_response(&json.unwrap());
        assert_error(&resp, -32000, "speedy grep failed");
    }

    #[test]
    fn test_tools_list_grep_schema() {
        let resp = send("tools/list", serde_json::json!({}));
        let grep = resp["result"]["tools"].as_array().unwrap()
            .iter().find(|t| t["name"] == "speedy_grep").unwrap().clone();
        assert!(grep["description"].as_str().unwrap().contains("FTS5"));
        let schema = &grep["inputSchema"];
        assert_eq!(schema["properties"]["pattern"]["type"], "string");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "pattern"));
    }

    // ── unknown method ──────────────────────────────────

    #[test]
    fn test_unknown_method() {
        let resp = send("foobar", serde_json::json!({}));
        assert_error(&resp, -32601, "Method not found: foobar");
    }

    // ── malformed JSON ──────────────────────────────────

    #[test]
    fn test_malformed_json() {
        let json = process_line("not json at all", &mock_runner(""), &mock_runner(""));
        let resp = parse_response(&json.unwrap());
        assert_eq!(resp["jsonrpc"], "2.0");
        assert!(resp["result"].is_null(), "unexpected result: {resp}");
        assert_eq!(resp["error"]["code"], -32700);
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(msg.contains("Parse error"), "error message: {msg}");
    }

    #[test]
    fn test_missing_id() {
        let line = r#"{"jsonrpc":"2.0","method":"shutdown","params":{}}"#;
        let json = process_line(line, &mock_runner(""), &mock_runner(""));
        let resp: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
        assert!(resp["id"].is_null());
    }

    // ── speedy_query mode parameter ─────────────────────

    #[test]
    fn test_tools_list_query_schema_has_mode() {
        let resp = send("tools/list", serde_json::json!({}));
        let q = &resp["result"]["tools"][0];
        assert_eq!(q["name"], "speedy_query");
        let schema = &q["inputSchema"];
        assert!(schema["properties"]["mode"]["type"].as_str() == Some("string"));
        let modes = schema["properties"]["mode"]["enum"].as_array().unwrap();
        let mode_strs: Vec<&str> = modes.iter().filter_map(|v| v.as_str()).collect();
        assert!(mode_strs.contains(&"full"));
        assert!(mode_strs.contains(&"files"));
    }

    #[test]
    fn test_call_query_mode_files_passes_through() {
        let resp = send("tools/call", serde_json::json!({
            "name": "speedy_query",
            "arguments": {"query": "auth logic", "mode": "files"}
        }));
        // mock_runner returns "ok" which is not valid JSON, so compact_to_files falls back
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_compact_to_files_deduplicates() {
        let input = serde_json::json!([
            {"path": "src/auth.rs", "line": 10, "text": "fn login", "score": 0.9},
            {"path": "src/auth.rs", "line": 50, "text": "fn logout", "score": 0.7},
            {"path": "src/main.rs", "line": 1, "text": "fn main", "score": 0.5}
        ]).to_string();
        let output = compact_to_files(&input);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.len(), 2, "should deduplicate to 2 unique files");
        assert_eq!(parsed[0]["path"], "src/auth.rs");
        assert_eq!(parsed[0]["score"], 0.9, "should keep max score");
        assert_eq!(parsed[1]["path"], "src/main.rs");
        assert!(!parsed[0].as_object().unwrap().contains_key("text"), "should not include text");
        assert!(!parsed[0].as_object().unwrap().contains_key("line"), "should not include line");
    }

    #[test]
    fn test_compact_to_files_invalid_json_fallback() {
        let input = "not valid json";
        let output = compact_to_files(input);
        assert_eq!(output, input, "should return raw input on parse error");
    }

    #[test]
    fn test_compact_to_files_empty_array() {
        let input = "[]";
        let output = compact_to_files(input);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap();
        assert!(parsed.is_empty());
    }

    // ── run_speedy function ──────────────────────────

    #[test]
    fn test_run_speedy_uses_speedy_bin_env() {
        // Temporarily set SPEEDY_BIN to a known non-existent path
        let _env_lock = ENV_LOCK.lock().unwrap();
        let original = std::env::var("SPEEDY_BIN").ok();
        std::env::set_var("SPEEDY_BIN", "nonexistent-binary-12345");

        let result = run_speedy(&["--version"]);
        assert!(result.is_err(), "should error when binary not found");
        let err = result.unwrap_err();
        assert!(err.contains("nonexistent-binary-12345"), "error should mention the binary name: {err}");

        if let Some(val) = original {
            std::env::set_var("SPEEDY_BIN", val);
        } else {
            std::env::remove_var("SPEEDY_BIN");
        }
    }

    #[test]
    fn test_run_speedy_defaults_to_speedy_cli() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let original = std::env::var("SPEEDY_BIN").ok();
        std::env::remove_var("SPEEDY_BIN");

        let result = run_speedy(&["--version"]);
        // The default is the thin client. In test envs it usually IS on PATH
        // (the workspace target dir is added by cargo), but we don't depend on
        // that — we only assert the name flows through to the error message.
        if let Err(e) = &result {
            assert!(
                e.contains("speedy-cli"),
                "default binary name should be 'speedy-cli', got: {e}"
            );
        }

        if let Some(val) = original {
            std::env::set_var("SPEEDY_BIN", val);
        } else {
            std::env::remove_var("SPEEDY_BIN");
        }
    }

    /// Clean-env scenario: with no SPEEDY_BIN set, the default name
    /// `speedy-cli` must be the one we look up via PATH. We force a PATH that
    /// can't contain the binary and assert the failure message names it.
    #[test]
    fn test_run_speedy_default_uses_path_for_speedy_cli() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let prev_bin = std::env::var("SPEEDY_BIN").ok();
        let prev_path = std::env::var("PATH").ok();
        std::env::remove_var("SPEEDY_BIN");
        // Empty PATH guarantees the lookup will fail with the binary name in
        // the OS error message — no risk of accidentally finding a real one.
        std::env::set_var("PATH", "");

        let result = run_speedy(&["--version"]);
        assert!(result.is_err(), "with empty PATH the lookup must fail");
        let err = result.unwrap_err();
        assert!(
            err.contains("speedy-cli"),
            "error must name the default binary 'speedy-cli', got: {err}"
        );

        match prev_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        if let Some(v) = prev_bin {
            std::env::set_var("SPEEDY_BIN", v);
        } else {
            std::env::remove_var("SPEEDY_BIN");
        }
    }

    // ── runner function ─────────────────────────────────

    #[test]
    fn test_runner_passes_correct_args() {
        let captured = std::sync::Mutex::new(Vec::new());
        let runner = |args: &[&str]| -> Result<String, String> {
            captured.lock().unwrap().extend(args.iter().map(|s| s.to_string()));
            Ok("output".to_string())
        };

        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "speedy_query", "arguments": {"query": "hello world", "top_k": 2}}
        });
        process_line(&line.to_string(), &runner, &mock_runner("ok"));

        let args = captured.lock().unwrap();
        assert!(args.contains(&"query".to_string()));
        assert!(args.contains(&"hello world".to_string()));
        assert!(args.contains(&"-k".to_string()));
        assert!(args.contains(&"2".to_string()));
        assert!(args.contains(&"--json".to_string()));
    }
}
