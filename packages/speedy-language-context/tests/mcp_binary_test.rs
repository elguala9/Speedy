use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn quiet_command(exe: &Path) -> Command {
    let cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = cmd;
        c.creation_flags(CREATE_NO_WINDOW);
        return c;
    }
    #[cfg(not(windows))]
    cmd
}

struct McpClient {
    process: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl McpClient {
    fn start(workspace: &Path) -> Self {
        let mut process = quiet_command(mcp_bin())
            .arg("--workspace")
            .arg(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start speedy-language-context-mcp");

        let reader = BufReader::new(process.stdout.take().unwrap());
        Self { process, reader }
    }

    fn start_with_cli(workspace: &Path) -> Self {
        let mut cmd = quiet_command(mcp_bin());
        cmd.arg("--workspace").arg(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cli) = speedy_cli_bin() {
            cmd.env("SPEEDY_BIN", cli);
        }
        let mut process = cmd.spawn().expect("failed to start speedy-language-context-mcp");
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
        if let Some(stdin) = self.process.stdin.as_mut() {
            let _ = writeln!(stdin, r#"{{"jsonrpc":"2.0","id":100,"method":"exit","params":{{}}}}"#);
            let _ = stdin.flush();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match self.process.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let _ = self.process.kill();
                    let _ = self.process.wait();
                    return;
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }
}

fn mcp_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| stage_binary("speedy-language-context", "speedy-language-context-mcp"))
}

fn speedy_cli_bin() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "speedy-cli.exe" } else { "speedy-cli" };
    let p = cargo_target_debug().join(exe);
    if p.exists() { Some(p) } else { None }
}

fn cargo_target_debug() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
}

fn test_stage_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir()
            .join("speedy_lc_mcp_test_bins")
            .join(format!("pid_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    })
}

fn stage_binary(package: &str, bin: &str) -> PathBuf {
    let exe = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    };
    let source = cargo_target_debug().join(exe(bin));
    let status = Command::new("cargo")
        .args(["build", "-p", package, "--bin", bin])
        .status()
        .expect("failed to run cargo build");
    if !status.success() {
        assert!(
            source.exists(),
            "cargo build failed for {package}/{bin} and no pre-built binary at {}",
            source.display()
        );
    }

    let stage_dir = test_stage_dir();
    let dest = stage_dir.join(exe(bin));
    copy_with_retry(&source, &dest);
    dest
}

fn copy_with_retry(source: &PathBuf, dest: &PathBuf) {
    let mut last_err = None;
    for _ in 0..30 {
        match std::fs::copy(source, dest) {
            Ok(_) => return,
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    panic!(
        "failed to stage binary {} → {}: {:?}",
        source.display(),
        dest.display(),
        last_err
    );
}

fn temp_workspace() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("speedy_lc_mcp_ws_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lib.rs"),
        b"pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    dir
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn test_initialize() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(resp["error"].is_null(), "unexpected error: {resp}");
    assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(resp["result"]["serverInfo"]["name"], "speedy-language-context");
    assert!(resp["result"]["capabilities"]["tools"].is_object());

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_tools_list() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_null(), "unexpected error: {resp}");
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"index_status"), "missing index_status: {names:?}");
    assert!(names.contains(&"get_skeleton"), "missing get_skeleton: {names:?}");
    assert!(names.contains(&"run_pipeline"), "missing run_pipeline: {names:?}");
    assert!(names.contains(&"save_observation"), "missing save_observation: {names:?}");
    assert!(names.contains(&"search_observations"), "missing search_observations: {names:?}");
    assert!(names.contains(&"force_reindex"), "missing force_reindex: {names:?}");
    assert!(names.contains(&"workspace_add"), "missing workspace_add: {names:?}");
    assert!(names.contains(&"workspace_remove"), "missing workspace_remove: {names:?}");
    assert!(names.contains(&"workspace_list"), "missing workspace_list: {names:?}");

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_index_status_empty_workspace() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"index_status","arguments":{}}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_null(), "unexpected error: {resp}");
    let content = &resp["result"]["content"];
    assert!(content.is_array(), "expected content array: {resp}");
    let text = content[0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "index_status returned empty text");

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_unknown_method_returns_error() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"unknown/method","params":{}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_object(), "expected error for unknown method: {resp}");
    assert_eq!(resp["error"]["code"], -32601);

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

// ── Tool implementation tests ──────────────────────────────────────────────

#[test]
fn test_save_observation_and_search_returns_result() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    // Save an observation
    let save_resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"save_observation","arguments":{"text":"the add function handles integer addition"}}}"#,
    ))
    .unwrap();
    assert!(
        save_resp["error"].is_null(),
        "save_observation should not error: {save_resp}"
    );

    // Search for it
    let search_resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_observations","arguments":{"query":"integer addition"}}}"#,
    ))
    .unwrap();
    assert!(
        search_resp["error"].is_null(),
        "search_observations should not error: {search_resp}"
    );
    let content = &search_resp["result"]["content"];
    assert!(content.is_array(), "expected content array: {search_resp}");
    let text = content[0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "search_observations returned empty content");

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_search_observations_on_empty_store_returns_ok() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_observations","arguments":{"query":"anything"}}}"#,
    ))
    .unwrap();
    assert!(
        resp["error"].is_null(),
        "search_observations on empty store should not error: {resp}"
    );
    let content = &resp["result"]["content"];
    assert!(content.is_array(), "expected content array: {resp}");

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_get_skeleton_unindexed_file_returns_placeholder() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_skeleton","arguments":{"files":["nonexistent.rs"],"detail":"standard"}}}"#,
    ))
    .unwrap();
    assert!(
        resp["error"].is_null(),
        "get_skeleton should not error for unindexed file: {resp}"
    );
    let content = &resp["result"]["content"];
    assert!(content.is_array(), "expected content array: {resp}");
    let text = content[0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "get_skeleton returned empty text for unindexed file");

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_save_multiple_observations_and_search() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    // Save two observations
    client.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"save_observation","arguments":{"text":"function foo handles error cases"}}}"#);
    client.send(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"save_observation","arguments":{"text":"struct Bar is the main data container"}}}"#);

    // Search for first
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_observations","arguments":{"query":"error handling"}}}"#,
    ))
    .unwrap();
    assert!(resp["error"].is_null(), "search should succeed: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "should find relevant observations");

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_force_reindex_indexes_workspace() {
    let ws = temp_workspace();
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"force_reindex","arguments":{}}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_null(), "force_reindex should not error: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "force_reindex returned empty text");

    let status: serde_json::Value = serde_json::from_str(text).expect("force_reindex should return JSON");
    assert_ne!(status["last_indexed"], "never", "workspace should be indexed after force_reindex");
    assert!(
        status["symbols"].as_u64().unwrap_or(0) > 0,
        "should have found symbols in lib.rs: {status}"
    );

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_workspace_tools_return_valid_response() {
    // Tests that workspace_add, workspace_remove, workspace_list each produce
    // a valid JSON-RPC response (either content or a structured error), never
    // a hang or panic. speedy-cli may or may not be available in the test env.
    let ws = temp_workspace();
    let mut client = McpClient::start_with_cli(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    for (id, tool, args) in [
        (2u64, "workspace_add",    r#"{"path": "/tmp/test-ws-add"}"#),
        (3u64, "workspace_remove", r#"{"path": "/tmp/test-ws-add"}"#),
        (4u64, "workspace_list",   r#"{}"#),
    ] {
        let call = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
        );
        let resp: serde_json::Value = serde_json::from_str(&client.send(&call))
            .unwrap_or_else(|e| panic!("{tool}: invalid JSON in response: {e}"));

        assert_eq!(resp["jsonrpc"], "2.0", "{tool}: bad jsonrpc field");
        assert_eq!(resp["id"], id, "{tool}: id mismatch");
        // Either a result with content array, or a structured error.
        let has_result = resp["result"]["content"].is_array();
        let has_error = resp["error"].is_object();
        assert!(
            has_result || has_error,
            "{tool}: response must have result.content or error, got: {resp}"
        );
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

// ── Tool implementation tests — Priorità 4 ────────────────────────────────

#[test]
fn test_index_status_shows_counts_after_reindex() {
    let ws = temp_workspace(); // lib.rs with `pub fn add`
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    client.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"force_reindex","arguments":{}}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"index_status","arguments":{}}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_null(), "index_status should not error: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    let status: serde_json::Value =
        serde_json::from_str(text).expect("index_status should return JSON");
    assert_ne!(status["last_indexed"], "never", "workspace should be indexed: {status}");
    assert!(
        status["symbols"].as_u64().unwrap_or(0) > 0,
        "should have at least one symbol: {status}"
    );
    assert!(
        status["files"].as_u64().unwrap_or(0) > 0,
        "should have at least one file: {status}"
    );

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_get_skeleton_after_indexing_returns_symbols() {
    let ws = temp_workspace(); // lib.rs with `pub fn add`
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    client.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"force_reindex","arguments":{}}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_skeleton","arguments":{"files":["lib.rs"],"detail":"standard"}}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_null(), "get_skeleton should not error: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "get_skeleton returned empty text after indexing");
    assert!(
        text.contains("add"),
        "skeleton should contain the 'add' function from lib.rs: {text}"
    );

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_run_pipeline_on_indexed_workspace() {
    let ws = temp_workspace(); // lib.rs with `pub fn add`
    let mut client = McpClient::start(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    client.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"force_reindex","arguments":{}}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_pipeline","arguments":{"task":"add function","top_k":5}}}"#,
    ))
    .unwrap();

    assert!(resp["error"].is_null(), "run_pipeline should not error: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "run_pipeline returned empty text");
    let result: serde_json::Value =
        serde_json::from_str(text).expect("run_pipeline should return JSON");
    assert_eq!(result["task"], "add function", "task field mismatch: {result}");
    assert!(result["matches"].is_array(), "should have matches array: {result}");
    assert!(result["impact"].is_array(), "should have impact array: {result}");
    let matches = result["matches"].as_array().unwrap();
    assert!(
        !matches.is_empty(),
        "should find at least one match for 'add function' in lib.rs: {result}"
    );

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_workspace_add_and_list_roundtrip() {
    // If speedy-cli is available, adding a workspace should make it appear in list.
    let ws = temp_workspace();
    let extra = temp_workspace();
    let mut client = McpClient::start_with_cli(&ws);

    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let add_call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "workspace_add", "arguments": {"path": extra.to_str().unwrap()}}
    });
    let add_resp: serde_json::Value = serde_json::from_str(&client.send(&add_call.to_string())).unwrap();

    // If add failed (no speedy-cli), skip the round-trip check.
    if add_resp["error"].is_object() {
        client.stop();
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&extra);
        return;
    }

    let list_resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"workspace_list","arguments":{}}}"#,
    ))
    .unwrap();
    assert!(list_resp["error"].is_null(), "workspace_list should succeed: {list_resp}");
    let list_text = list_resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    let extra_canonical = extra.canonicalize().unwrap();
    let list_json: serde_json::Value = serde_json::from_str(list_text).unwrap_or(serde_json::Value::Null);
    let found = list_json.as_array().map_or(false, |paths| {
        paths.iter().any(|p| {
            p.as_str().and_then(|s| std::path::Path::new(s).canonicalize().ok())
                .as_deref() == Some(extra_canonical.as_path())
        })
    });
    assert!(found, "workspace_list should contain the added path; list={list_text}");

    // Clean up: remove the added workspace.
    let remove_call = serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "workspace_remove", "arguments": {"path": extra.to_str().unwrap()}}
    });
    client.send(&remove_call.to_string());

    client.stop();
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&extra);
}
