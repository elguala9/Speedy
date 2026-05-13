use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::process::Command;

const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "speedy-mcp";
const SERVER_VERSION: &str = "0.1.0";

fn main() {
    let stdin = io::stdin();
    let reader = stdin.lock();
    let runner = |args: &[&str]| run_speedy(args);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = process_line(&line, &runner);
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

fn process_line(line: &str, run_cmd: &dyn Fn(&[&str]) -> Result<String, String>) -> Option<String> {
    let request: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            let err = JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
            return Some(serde_json::to_string(&err).expect("serialize"));
        }
    };

    let response = handle_request(&request, run_cmd);
    response.map(|r| serde_json::to_string(&r).expect("serialize"))
}

fn handle_request(req: &JsonRpcRequest, run_cmd: &dyn Fn(&[&str]) -> Result<String, String>) -> Option<JsonRpcResponse> {
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
                    "Semantic search over the codebase using natural language.",
                    serde_json::json!({
                        "query": {"type": "string", "description": "Natural language query"},
                        "top_k": {"type": "number", "description": "Number of results (default: 5)", "default": 5}
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
            ];
            Some(JsonRpcResponse::success(req.id, serde_json::json!({"tools": tools})))
        }

        "tools/call" => {
            let name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = req.params.get("arguments").unwrap_or(&serde_json::Value::Null);
            match name {
                "speedy_query" => {
                    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5);
                    let cmd_args = ["query", query, "-k", &top_k.to_string(), "--json"];
                    match run_cmd(&cmd_args) {
                        Ok(output) => Some(JsonRpcResponse::success(req.id, content_json(&output))),
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

fn run_speedy(args: &[&str]) -> Result<String, String> {
    let bin = std::env::var("SPEEDY_BIN").unwrap_or_else(|_| "speedy".to_string());
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
        let json = process_line(&line.to_string(), &mock_runner("ok"));
        parse_response(&json.unwrap_or_else(|| panic!("no response for {method}")))
    }

    // ── initialize ──────────────────────────────────────

    #[test]
    fn test_initialize() {
        let resp = send("initialize", serde_json::json!({}));
        assert_success(&resp, &serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "speedy-mcp", "version": "0.1.0"}
        }));
    }

    #[test]
    fn test_initialize_includes_id() {
        let line = r#"{"jsonrpc":"2.0","id":42,"method":"initialize","params":{}}"#;
        let json = process_line(line, &mock_runner("")).unwrap();
        let resp: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(resp["id"], 42);
    }

    // ── notifications/initialized ──────────────────────

    #[test]
    fn test_notification_returns_none() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let result = process_line(line, &mock_runner(""));
        assert!(result.is_none(), "notifications should not produce a response");
    }

    // ── tools/list ──────────────────────────────────────

    #[test]
    fn test_tools_list_has_three_tools() {
        let resp = send("tools/list", serde_json::json!({}));
        let tools = &resp["result"]["tools"];
        assert!(tools.is_array());
        assert_eq!(tools.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_tools_list_names() {
        let resp = send("tools/list", serde_json::json!({}));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array().unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["speedy_query", "speedy_index", "speedy_context"]);
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
        let json = process_line(&line.to_string(), &fail_runner);
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

    // ── unknown method ──────────────────────────────────

    #[test]
    fn test_unknown_method() {
        let resp = send("foobar", serde_json::json!({}));
        assert_error(&resp, -32601, "Method not found: foobar");
    }

    // ── malformed JSON ──────────────────────────────────

    #[test]
    fn test_malformed_json() {
        let json = process_line("not json at all", &mock_runner(""));
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
        let json = process_line(line, &mock_runner(""));
        let resp: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
        assert!(resp["id"].is_null());
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
        process_line(&line.to_string(), &runner);

        let args = captured.lock().unwrap();
        assert!(args.contains(&"query".to_string()));
        assert!(args.contains(&"hello world".to_string()));
        assert!(args.contains(&"-k".to_string()));
        assert!(args.contains(&"2".to_string()));
        assert!(args.contains(&"--json".to_string()));
    }
}
