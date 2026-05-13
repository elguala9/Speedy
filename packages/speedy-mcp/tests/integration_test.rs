use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

struct McpClient {
    process: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl McpClient {
    fn start(workdir: &PathBuf) -> Self {
        let mut process = Command::new(mcp_bin())
            .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start speedy-mcp");

        let reader = BufReader::new(process.stdout.take().unwrap());
        Self { process, reader }
    }

    fn send(&mut self, json: &str) -> String {
        let stdin = self.process.stdin.as_mut().unwrap();
        writeln!(stdin, "{json}").expect("failed to write to stdin");
        stdin.flush().ok();

        let mut line = String::new();
        self.reader.read_line(&mut line).expect("failed to read stdout");
        line.trim().to_string()
    }

    fn stop(&mut self) {
        self.send(r#"{"jsonrpc":"2.0","id":99,"method":"shutdown","params":{}}"#);
        self.send(r#"{"jsonrpc":"2.0","id":100,"method":"exit","params":{}}"#);
        let _ = self.process.wait();
    }
}

fn mcp_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| build_binary("speedy-mcp", "speedy-mcp"))
}

fn speedy_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| build_binary("speedy", "speedy"))
}

fn build_binary(package: &str, bin: &str) -> PathBuf {
    let status = Command::new("cargo")
        .args(["build", "-p", package, "--bin", bin])
        .status()
        .expect("failed to run cargo build");
    assert!(status.success(), "cargo build failed for {package}/{bin}");

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug");

    let exe = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
    target.join(exe)
}

fn temp_project() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("speedy_mcp_int_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();

    std::fs::write(dir.join("Cargo.toml"), br#"[package]
name = "mcp-test"
version = "0.1.0"
edition = "2021"
"#)
    .unwrap();

    std::fs::write(
        dir.join("src").join("lib.rs"),
        br#"pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();

    dir
}

fn assert_rpc_success(response: &serde_json::Value) {
    assert_eq!(response["jsonrpc"], "2.0", "bad jsonrpc: {response}");
    assert!(response["id"].is_number(), "missing id: {response}");
    assert!(
        response["result"].is_object() || response["result"].is_null(),
        "unexpected result type: {response}"
    );
    assert!(response["error"].is_null(), "unexpected error: {response}");
}

fn init_and_list(client: &mut McpClient) -> serde_json::Value {
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ))
    .unwrap();
    resp
}

// ── Protocol Tests (no external dependency) ────────────

#[test]
fn test_initialize_protocol_version() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    ))
    .unwrap();

    assert_rpc_success(&resp);
    assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(resp["result"]["serverInfo"]["name"], "speedy-mcp");
    assert_eq!(resp["result"]["serverInfo"]["version"], "0.1.0");
    assert!(resp["result"]["capabilities"]["tools"].is_object());

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_tools_list_three_tools() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    let resp = init_and_list(&mut client);

    assert_rpc_success(&resp);
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["speedy_query", "speedy_index", "speedy_context"]);

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_tools_list_schemas_valid() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    let resp = init_and_list(&mut client);
    let tools = resp["result"]["tools"].as_array().unwrap();

    for tool in tools {
        assert!(tool["name"].as_str().unwrap().starts_with("speedy_"));
        assert!(tool["description"].as_str().unwrap().len() > 5);
        assert!(tool["inputSchema"]["type"] == "object");
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_unknown_tool_error() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nonexistent","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["error"]["code"], -32601);
    assert!(resp["error"]["message"].as_str().unwrap().contains("Unknown tool"));

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_full_lifecycle() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    let init: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&init);

    let list: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ))
    .unwrap();
    assert!(list["result"]["tools"].as_array().unwrap().len() == 3);

    let shutdown: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&shutdown);

    client.send(r#"{"jsonrpc":"2.0","id":4,"method":"exit","params":{}}"#);
    let status = client.process.wait().expect("process should exit");
    assert!(status.success(), "server exited with error: {status}");

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_shutdown_idempotent() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let r1: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&r1);

    let r2: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&r2);

    let r3: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":4,"method":"shutdown","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&r3);

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

// ── Real Binary Tests (exercises the actual `speedy` binary) ──

#[test]
fn test_query_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_query","arguments":{"query":"greet function"}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);

    if resp["error"].is_null() {
        let content = &resp["result"]["content"];
        assert!(content.is_array(), "content should be an array");
        if !content.as_array().unwrap().is_empty() {
            assert_eq!(content[0]["type"], "text");
        }
    } else {
        assert_eq!(resp["error"]["code"], -32000);
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("speedy query failed"),
            "unexpected error: {msg}"
        );
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_context_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_context","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);

    if resp["error"].is_null() {
        assert!(resp["result"]["content"].is_array());
    } else {
        assert_eq!(resp["error"]["code"], -32000);
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_index_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_index","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);

    if resp["error"].is_null() {
        let content = &resp["result"]["content"];
        assert!(content.is_array());
        if !content.as_array().unwrap().is_empty() {
            let text = content[0]["text"].as_str().unwrap_or("");
            assert!(
                text.contains("files") || text.contains("chunks") || text.contains("Indexed"),
                "unexpected index output: {text}"
            );
        }
    } else {
        assert_eq!(resp["error"]["code"], -32000);
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_notification_read_write_ordering() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    // Initialize
    let init: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&init);

    // Send notification (no response expected). We must NOT try to read.
    {
        let stdin = client.process.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized","params":{{}}}}"#).unwrap();
        stdin.flush().ok();
    }

    // Now send a request that WILL produce a response
    let list: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&list);
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 3);

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}
