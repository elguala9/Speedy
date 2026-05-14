use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Spawn a child binary without popping a console window on Windows. We
/// always capture stdio, so the window adds no value and pollutes the screen
/// when tests run.
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
    fn start(workdir: &PathBuf) -> Self {
        let mut process = quiet_command(mcp_bin())
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
    BIN.get_or_init(|| stage_binary("speedy-mcp", "speedy-mcp"))
}

fn speedy_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| stage_binary("speedy", "speedy"))
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
            .join("speedy_mcp_test_bins")
            .join(format!("pid_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    })
}

/// Build the binary via cargo, then copy it to a per-test-run staging dir to
/// dodge Windows "os error 5" caused by link.exe / AV briefly holding a
/// write-handle on the freshly emitted exe. If `speedy-daemon` happens to be
/// already built, it is staged alongside so daemon discovery still works.
///
/// On Windows it's common for a developer to have `speedy.exe` already
/// running (the daemon or the CLI), which blocks rebuilds. We treat the
/// rebuild as best-effort: if the binary already exists at the expected
/// path, use it instead of failing the whole suite.
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
        eprintln!(
            "warning: cargo build failed for {package}/{bin}; using pre-built binary at {}",
            source.display()
        );
    }

    let stage_dir = test_stage_dir();
    let dest = stage_dir.join(exe(bin));
    copy_with_retry(&source, &dest);

    // Best-effort: stage speedy-daemon next to speedy so find_daemon_exe()
    // resolves it. We don't shell out to `cargo build` for it — a nested
    // cargo invocation on Windows races the parent and trips os error 5 on
    // the daemon's exe link step.
    let daemon_src = cargo_target_debug().join(exe("speedy-daemon"));
    let daemon_dest = stage_dir.join(exe("speedy-daemon"));
    if daemon_src.exists() {
        copy_with_retry(&daemon_src, &daemon_dest);
    }

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

fn temp_project() -> PathBuf {
    // Each test gets its own dir. Parallel integration tests were racing on a
    // single shared path: one test would remove the dir while another was
    // setting current_dir on a child process, producing "not a directory".
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("speedy_mcp_int_{}_{n}", std::process::id()));
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

// ── MCP ↔ daemon integration (real daemon, real speedy, real MCP) ──

/// Returns the daemon binary path directly from target/debug. We don't stage
/// the daemon to a per-process scratch dir like we do for speedy.exe: the
/// daemon is launched as a long-lived child here, and parallel test threads
/// would race the copy step ("file in use by another process") on Windows.
fn daemon_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let exe = if cfg!(windows) { "speedy-daemon.exe" } else { "speedy-daemon" };
        let p = cargo_target_debug().join(exe);
        assert!(
            p.exists(),
            "speedy-daemon binary not found at {}; build it with `cargo build -p speedy-daemon`",
            p.display()
        );
        p
    })
}

/// Spawn a `speedy-daemon` on a unique socket name inside a dedicated
/// daemon_dir so the test never touches the user's running daemon.
struct TestDaemon {
    process: Option<Child>,
    socket: String,
    daemon_dir: PathBuf,
}

impl TestDaemon {
    fn start(label: &str) -> Self {
        // Unique per-process, per-test-instance.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let socket = format!("speedy_mcp_d_{label}_{}_{n}", std::process::id());
        let daemon_dir = std::env::temp_dir().join(format!("speedy_mcp_d_dir_{label}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&daemon_dir);
        std::fs::create_dir_all(&daemon_dir).unwrap();

        let process = quiet_command(daemon_bin())
            .args(["--daemon-socket", &socket])
            .arg("--daemon-dir").arg(&daemon_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start speedy-daemon");

        std::thread::sleep(std::time::Duration::from_secs(1));
        Self { process: Some(process), socket, daemon_dir }
    }

    /// Read the daemon's list of registered workspaces via a raw IPC call.
    fn list_workspaces(&self) -> Vec<String> {
        let socket = self.socket.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
            use speedy_core::local_sock::{Stream, StreamTrait as _, ToNsName, GenericNamespaced};
            let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap();
            let mut stream = Stream::connect(name).await.unwrap();
            stream.write_all(b"list\n").await.unwrap();
            stream.shutdown().await.unwrap();
            let mut reader = tokio::io::BufReader::new(&mut stream);
            let mut resp = String::new();
            reader.read_line(&mut resp).await.unwrap();
            serde_json::from_str(resp.trim()).unwrap_or_default()
        })
    }

    fn watch_count(&self) -> usize {
        let socket = self.socket.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
            use speedy_core::local_sock::{Stream, StreamTrait as _, ToNsName, GenericNamespaced};
            let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap();
            let mut stream = Stream::connect(name).await.unwrap();
            stream.write_all(b"watch-count\n").await.unwrap();
            stream.shutdown().await.unwrap();
            let mut reader = tokio::io::BufReader::new(&mut stream);
            let mut resp = String::new();
            reader.read_line(&mut resp).await.unwrap();
            resp.trim().parse().unwrap_or(0)
        })
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if let Some(mut p) = self.process.take() {
            let _ = p.kill();
            let _ = p.wait();
        }
        let _ = std::fs::remove_dir_all(&self.daemon_dir);
    }
}

fn start_mcp_with_daemon(workdir: &PathBuf, daemon: &TestDaemon) -> McpClient {
    let mut process = quiet_command(mcp_bin())
        .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
        .env("SPEEDY_DEFAULT_SOCKET", &daemon.socket)
        .env("SPEEDY_DAEMON_DIR", &daemon.daemon_dir)
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start speedy-mcp");

    let reader = BufReader::new(process.stdout.take().unwrap());
    McpClient { process, reader }
}

#[test]
fn test_mcp_index_registers_workspace_with_daemon() {
    let workdir = temp_project();
    let daemon = TestDaemon::start("idx_ws");

    // Confirm we start with zero registered workspaces in this daemon.
    assert_eq!(daemon.list_workspaces().len(), 0);
    assert_eq!(daemon.watch_count(), 0);

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_index","arguments":{"path":"."}}}"#,
    ))
    .unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);

    // Even if speedy index failed to produce a useful output (it might in a
    // skeletal project), the side-effect of contacting the daemon to register
    // the workspace must have happened.
    let registered = daemon.list_workspaces();
    let workdir_canonical = workdir.canonicalize().unwrap();
    let found = registered.iter().any(|p| {
        std::path::Path::new(p).canonicalize().ok().as_ref() == Some(&workdir_canonical)
    });
    assert!(
        found,
        "daemon should have registered the workspace via MCP→speedy→daemon, list={registered:?} expected={}",
        workdir_canonical.display()
    );
    assert!(daemon.watch_count() >= 1, "watcher should be active after index");

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_query_uses_same_daemon() {
    let workdir = temp_project();
    let daemon = TestDaemon::start("qry_same");

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    // First an index to register the workspace.
    client.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_index","arguments":{"path":"."}}}"#);
    let count_after_index = daemon.list_workspaces().len();
    assert!(count_after_index >= 1, "expected at least one workspace after index, got {count_after_index}");

    // Now a query — should hit the same daemon, not spawn a fresh one.
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"speedy_query","arguments":{"query":"greet"}}}"#,
    ))
    .unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");

    // Count is unchanged: a second tool call must not register the workspace twice.
    let count_after_query = daemon.list_workspaces().len();
    assert_eq!(count_after_query, count_after_index, "workspace count should not change on subsequent queries");

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_context_uses_daemon() {
    let workdir = temp_project();
    let daemon = TestDaemon::start("ctx_dae");

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_context","arguments":{}}}"#,
    ))
    .unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");

    // context registers the workspace too (it goes through ensure_daemon)
    let registered = daemon.list_workspaces();
    let workdir_canonical = workdir.canonicalize().unwrap();
    let found = registered.iter().any(|p| {
        std::path::Path::new(p).canonicalize().ok().as_ref() == Some(&workdir_canonical)
    });
    assert!(found, "context tool call should register workspace, registered={registered:?}");

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

// ── MCP loop robustness: stdin handling ─────────────────

#[test]
fn test_mcp_exits_cleanly_on_stdin_eof() {
    // Close stdin without ever sending a message. The for-line loop ends
    // naturally and the process must exit with status 0.
    let workdir = temp_project();
    let mut process = quiet_command(mcp_bin())
        .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
        .current_dir(&workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start speedy-mcp");

    // Drop stdin to signal EOF.
    drop(process.stdin.take());

    // Wait briefly and verify the process exits.
    let status = process.wait().expect("process should terminate");
    assert!(status.success(), "MCP should exit cleanly on stdin EOF, got: {status}");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_skips_blank_lines() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    // Send empty lines mixed with real requests.
    {
        let stdin = client.process.stdin.as_mut().unwrap();
        writeln!(stdin, "").unwrap();
        writeln!(stdin, "   ").unwrap();
        writeln!(stdin).unwrap();
        stdin.flush().ok();
    }

    // A real request after the blanks must still work.
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&resp);
    assert_eq!(resp["id"], 1);

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_handles_pipelined_requests() {
    let workdir = temp_project();
    let mut process = quiet_command(mcp_bin())
        .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
        .current_dir(&workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start speedy-mcp");

    // Write three requests in rapid succession before reading any response.
    {
        let stdin = process.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{{}}}}"#).unwrap();
        stdin.flush().ok();
    }

    // Each request must produce exactly one response line, in order.
    let mut reader = BufReader::new(process.stdout.take().unwrap());
    for expected_id in [1u64, 2, 3] {
        let mut line = String::new();
        reader.read_line(&mut line).expect("response line");
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], expected_id, "responses out of order");
    }

    // Exit cleanly.
    {
        let stdin = process.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":4,"method":"exit","params":{{}}}}"#).unwrap();
        stdin.flush().ok();
    }
    let _ = process.wait();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_id_preserved_across_calls() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    // Use a non-sequential id to verify the server echoes whatever we send.
    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":9001,"method":"initialize","params":{}}"#,
    ))
    .unwrap();
    assert_eq!(resp["id"], 9001);

    let resp2: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":42,"method":"tools/list","params":{}}"#,
    ))
    .unwrap();
    assert_eq!(resp2["id"], 42);

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_exit_request_terminates_process() {
    let workdir = temp_project();
    let mut process = quiet_command(mcp_bin())
        .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
        .current_dir(&workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start speedy-mcp");

    {
        let stdin = process.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"exit","params":{{}}}}"#).unwrap();
        stdin.flush().ok();
    }

    // The MCP loop watches for "exit" in the line and breaks out. Even if we
    // never close stdin, the process should terminate.
    let status = process.wait().expect("process should terminate");
    assert!(status.success(), "exit request should terminate MCP, status={status}");
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_invalid_json_response_per_line() {
    // Garbage on one line must produce one parse-error response on the next
    // line, and a subsequent valid request must still work.
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);

    let resp1: serde_json::Value = serde_json::from_str(&client.send("not valid json")).unwrap();
    assert_eq!(resp1["error"]["code"], -32700);

    let resp2: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":99,"method":"initialize","params":{}}"#,
    ))
    .unwrap();
    assert_rpc_success(&resp2);
    assert_eq!(resp2["id"], 99);

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
