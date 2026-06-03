use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::OnceLock;
use std::time::Duration;

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
    rx: Receiver<String>,
}

impl McpClient {
    /// Upper bound for a single response line. Generous for protocol calls,
    /// but short enough that a wedged server fails the offending test fast
    /// instead of hanging the whole test binary forever (a hung read here
    /// also holds the cargo build lock and blocks every other run).
    const READ_TIMEOUT: Duration = Duration::from_secs(30);

    fn start(workdir: &PathBuf) -> Self {
        let mut process = quiet_command(mcp_bin())
            .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start speedy-ai-context-mcp");

        Self::attach(process)
    }

    /// Wrap an already-spawned MCP process, draining its stdout on a dedicated
    /// thread so `send` can wait on a channel with a timeout — a plain blocking
    /// `read_line` has no escape hatch.
    fn attach(mut process: Child) -> Self {
        let stdout = process.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF: child closed stdout
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break; // McpClient dropped
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self { process, rx }
    }

    fn send(&mut self, json: &str) -> String {
        let stdin = self.process.stdin.as_mut().unwrap();
        writeln!(stdin, "{json}").expect("failed to write to stdin");
        stdin.flush().ok();

        match self.rx.recv_timeout(Self::READ_TIMEOUT) {
            Ok(line) => line.trim().to_string(),
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.process.kill();
                panic!("MCP server did not respond within {:?} to: {json}", Self::READ_TIMEOUT);
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("MCP server closed stdout without responding to: {json}");
            }
        }
    }

    fn stop(&mut self) {
        self.send(r#"{"jsonrpc":"2.0","id":99,"method":"shutdown","params":{}}"#);
        // "exit" closes the server without a response — write-only, no read.
        if let Some(stdin) = self.process.stdin.as_mut() {
            let _ = writeln!(stdin, r#"{{"jsonrpc":"2.0","id":100,"method":"exit","params":{{}}}}"#);
            let _ = stdin.flush();
        }
        // Wait up to 5 s, then kill to avoid CI timeout.
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
    BIN.get_or_init(|| stage_binary("speedy-ai-context-mcp", "speedy-ai-context-mcp"))
}

fn speedy_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| stage_binary("speedy-cli", "speedy-cli"))
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

/// Owns a temp project dir for one test. On drop it BOTH wipes the directory
/// AND de-registers the workspace from the daemon's `workspaces.json`. The
/// second step matters: invoking `speedy_query` against this dir triggers
/// `ensure_daemon` in the worker, which adds the cwd to the registry. Without
/// the de-register on drop, the registry accumulates ghost test entries that
/// later show up in the GUI's Workspaces tab.
pub struct TempProject {
    dir: PathBuf,
}

impl TempProject {
    fn new() -> Self {
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

        Self { dir }
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        // The worker canonicalizes cwd before adding it to workspaces.json, so
        // the stored form is usually the `\\?\C:\...` extended-length path on
        // Windows. Try the canonical form first, then the literal as a fallback.
        let canonical = self.dir.canonicalize().ok();
        if let Some(c) = &canonical {
            let _ = speedy_core::workspace::remove(&c.to_string_lossy());
        }
        let _ = speedy_core::workspace::remove(&self.dir.to_string_lossy());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl std::ops::Deref for TempProject {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.dir
    }
}

impl AsRef<Path> for TempProject {
    fn as_ref(&self) -> &Path {
        &self.dir
    }
}

fn temp_project() -> TempProject {
    TempProject::new()
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
    assert_eq!(resp["result"]["serverInfo"]["name"], "speedy-ai-context-mcp");
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
    assert_eq!(tools.len(), 10);

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
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
    assert!(list["result"]["tools"].as_array().unwrap().len() == 10);

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
                text.contains("files") || text.contains("chunks") || text.contains("Indexed")
                    || text.starts_with("error:"),
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
    let process = quiet_command(mcp_bin())
        .env("SPEEDY_BIN", speedy_bin().to_str().unwrap())
        .env("SPEEDY_DEFAULT_SOCKET", &daemon.socket)
        .env("SPEEDY_DAEMON_DIR", &daemon.daemon_dir)
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start speedy-ai-context-mcp");

    McpClient::attach(process)
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
        .expect("failed to start speedy-ai-context-mcp");

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
        .expect("failed to start speedy-ai-context-mcp");

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
        .expect("failed to start speedy-ai-context-mcp");

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
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 10);

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

// ── Workspace & reindex tool tests (real binary) ───────────────────────────

#[test]
fn test_workspace_list_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_workspace_list","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    if resp["error"].is_null() {
        assert!(resp["result"]["content"].is_array());
    } else {
        assert_eq!(resp["error"]["code"], -32000);
        assert!(resp["error"]["message"].as_str().unwrap_or("").contains("speedy workspace list failed"));
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_workspace_add_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "speedy_workspace_add", "arguments": {"path": workdir.to_str().unwrap()}}
    });
    let resp: serde_json::Value = serde_json::from_str(&client.send(&call.to_string())).unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    if resp["error"].is_null() {
        assert!(resp["result"]["content"].is_array());
    } else {
        assert_eq!(resp["error"]["code"], -32000);
        assert!(resp["error"]["message"].as_str().unwrap_or("").contains("speedy workspace add failed"));
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_workspace_remove_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "speedy_workspace_remove", "arguments": {"path": workdir.to_str().unwrap()}}
    });
    let resp: serde_json::Value = serde_json::from_str(&client.send(&call.to_string())).unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    if resp["error"].is_null() {
        assert!(resp["result"]["content"].is_array());
    } else {
        assert_eq!(resp["error"]["code"], -32000);
        assert!(resp["error"]["message"].as_str().unwrap_or("").contains("speedy workspace remove failed"));
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

// Ignored by default: `speedy force` triggers a full synchronous reindex
// through the daemon (`sync <path>`), which only returns once the embedding
// pipeline finishes — it requires a running daemon plus a reachable embedding
// backend. Without them the daemon never closes the connection and the call
// blocks. Run explicitly with `cargo test -- --ignored` in an environment that
// has both. See `test_mcp_force_reindex_with_daemon` for the daemon-backed path.
#[test]
#[ignore = "requires a running daemon + embedding backend; force reindex blocks until full sync completes"]
fn test_force_reindex_via_real_binary() {
    let workdir = temp_project();
    let mut client = McpClient::start(&workdir);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_force_reindex","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    if resp["error"].is_null() {
        assert!(resp["result"]["content"].is_array());
    } else {
        assert_eq!(resp["error"]["code"], -32000);
        assert!(resp["error"]["message"].as_str().unwrap_or("").contains("speedy force reindex failed"));
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

// ── Workspace & reindex tool tests (daemon integration) ────────────────────

#[test]
fn test_mcp_workspace_add_registers_with_daemon() {
    let workdir = temp_project();
    let extra_ws = temp_project();
    let daemon = TestDaemon::start("ws_add");

    assert_eq!(daemon.list_workspaces().len(), 0);

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "speedy_workspace_add", "arguments": {"path": extra_ws.to_str().unwrap()}}
    });
    let resp: serde_json::Value = serde_json::from_str(&client.send(&call.to_string())).unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(resp["error"].is_null(), "workspace_add should succeed: {resp}");

    let registered = daemon.list_workspaces();
    let extra_ws_canonical = extra_ws.canonicalize().unwrap();
    let found = registered.iter().any(|p| {
        std::path::Path::new(p).canonicalize().ok().as_ref() == Some(&extra_ws_canonical)
    });
    assert!(
        found,
        "daemon should have the workspace after speedy_workspace_add, list={registered:?} expected={}",
        extra_ws_canonical.display()
    );

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_workspace_list_with_daemon() {
    let workdir = temp_project();
    let daemon = TestDaemon::start("ws_list");

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    // Register a workspace so the list is non-trivial.
    let add_call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "speedy_workspace_add", "arguments": {"path": workdir.to_str().unwrap()}}
    });
    client.send(&add_call.to_string());

    let list_resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"speedy_workspace_list","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(list_resp["jsonrpc"], "2.0");
    assert!(list_resp["error"].is_null(), "workspace_list should succeed: {list_resp}");
    let text = list_resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "workspace_list returned empty text");

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_workspace_remove_unregisters_from_daemon() {
    let workdir = temp_project();
    let extra_ws = temp_project();
    let daemon = TestDaemon::start("ws_rem");

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    // Add first.
    let add_call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "speedy_workspace_add", "arguments": {"path": extra_ws.to_str().unwrap()}}
    });
    client.send(&add_call.to_string());
    assert!(!daemon.list_workspaces().is_empty(), "workspace should be registered after add");

    // Now remove it.
    let remove_call = serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "speedy_workspace_remove", "arguments": {"path": extra_ws.to_str().unwrap()}}
    });
    let resp: serde_json::Value = serde_json::from_str(&client.send(&remove_call.to_string())).unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(resp["error"].is_null(), "workspace_remove should succeed: {resp}");

    let registered = daemon.list_workspaces();
    let extra_canonical = extra_ws.canonicalize().unwrap();
    let still_present = registered.iter().any(|p| {
        std::path::Path::new(p).canonicalize().ok().as_ref() == Some(&extra_canonical)
    });
    assert!(
        !still_present,
        "workspace should be gone after speedy_workspace_remove, list={registered:?}"
    );

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn test_mcp_force_reindex_with_daemon() {
    let workdir = temp_project();
    let daemon = TestDaemon::start("force_ri");

    let mut client = start_mcp_with_daemon(&workdir, &daemon);
    client.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    let resp: serde_json::Value = serde_json::from_str(&client.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"speedy_force_reindex","arguments":{}}}"#,
    ))
    .unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    if resp["error"].is_null() {
        assert!(resp["result"]["content"].is_array());
    } else {
        assert_eq!(resp["error"]["code"], -32000);
        assert!(resp["error"]["message"].as_str().unwrap_or("").contains("speedy force reindex failed"));
    }

    client.stop();
    let _ = std::fs::remove_dir_all(&workdir);
}
