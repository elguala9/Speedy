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
