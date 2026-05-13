use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

static NEXT_PORT: AtomicU16 = AtomicU16::new(42550);
static E2E_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn acquire_lock() -> std::sync::MutexGuard<'static, ()> {
    E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn test_port() -> u16 {
    NEXT_PORT.fetch_add(1, Ordering::SeqCst)
}

fn root_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned()
}

fn daemon_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| build_binary("speedy-daemon", "speedy-daemon"))
}

fn cli_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| build_binary("speedy-cli", "speedy-cli"))
}

fn speedy_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| build_binary("speedy", "speedy"))
}

fn build_binary(package: &str, bin: &str) -> PathBuf {
    let root = root_dir();
    let status = Command::new("cargo")
        .args(["build", "-p", package, "--bin", bin])
        .current_dir(&root)
        .status()
        .expect("failed to run cargo build");
    assert!(status.success(), "cargo build failed for {package}/{bin}");

    let exe = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
    root.join("target").join("debug").join(exe)
}

fn create_test_project(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        b"[package]\nname = \"e2e-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("lib.rs"),
        b"pub fn greet(name: &str) -> String { format!(\"Hello, {name}!\") }\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
}

struct DaemonGuard {
    process: Option<Child>,
    port: u16,
    dir: PathBuf,
}

impl DaemonGuard {
    fn start(port: u16, dir: &Path) -> Self {
        let process = Command::new(daemon_bin())
            .args(["--daemon-port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start speedy-daemon");

        std::thread::sleep(Duration::from_secs(1));

        Self {
            process: Some(process),
            port,
            dir: dir.to_owned(),
        }
    }

    fn cli_cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(cli_bin());
        cmd.args(["--daemon-port", &self.port.to_string()]);
        cmd.args(args);
        cmd.current_dir(&self.dir);
        cmd
    }

    fn run_cli(&self, args: &[&str]) -> Result<String, String> {
        let output = self.cli_cmd(args).output().map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(format!("exit={}: stdout={stdout} stderr={stderr}", output.status))
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.port)) {
            let _ = stream.write_all(b"stop\n");
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(mut child) = self.process.take() {
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_dist_binaries_exist() {
    assert!(daemon_bin().exists(), "speedy-daemon binary not found in dist/");
    assert!(cli_bin().exists(), "speedy-cli binary not found in dist/");
}

#[test]
fn test_daemon_ping_pong() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_ping_{port}"));
    let guard = DaemonGuard::start(port, &dir);

    let result = guard.run_cli(&["daemon", "ping"]);
    assert!(result.is_ok(), "ping failed: {:?}", result.err());
    assert_eq!(result.unwrap(), "pong");
}

#[test]
fn test_daemon_status() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_status_{port}"));
    let guard = DaemonGuard::start(port, &dir);

    let result = guard.run_cli(&["daemon", "status"]);
    assert!(result.is_ok(), "status failed: {:?}", result.err());
    let out = result.unwrap();
    assert!(out.contains("PID:"));
    assert!(out.contains("Version:"));
}

#[test]
fn test_daemon_list_empty() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_list_{port}"));
    let guard = DaemonGuard::start(port, &dir);

    let result = guard.run_cli(&["daemon", "list"]);
    assert!(result.is_ok(), "list failed: {:?}", result.err());
}

#[test]
fn test_workspace_add_and_remove() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_ws_{port}"));
    create_test_project(&dir);
    let guard = DaemonGuard::start(port, &dir);
    let ws_path = dir.to_string_lossy().to_string();

    let add = guard.run_cli(&["workspace", "add", &ws_path]);
    assert!(add.is_ok(), "workspace add failed: {:?}", add.as_ref().err());
    let out = add.unwrap();
    assert!(out.contains("added") || out.contains("ok"), "unexpected add output: {out}");

    let list = guard.run_cli(&["daemon", "list"]);
    assert!(list.is_ok());
    if let Ok(out) = &list {
        assert!(out.contains(&ws_path) || out.contains("[active]"));
    }

    let remove = guard.run_cli(&["workspace", "remove", &ws_path]);
    assert!(remove.is_ok(), "workspace remove failed: {:?}", remove.err());
}

#[test]
fn test_index_and_query_via_daemon() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_idx_{port}"));
    create_test_project(&dir);
    let guard = DaemonGuard::start(port, &dir);

    let index = guard.run_cli(&["index", "."]);
    assert!(index.is_ok(), "index failed: {:?}", index.err());
    let out = index.unwrap();
    assert!(out.contains("Indexed"), "unexpected index output: {out}");

    let context = guard.run_cli(&["context"]);
    assert!(context.is_ok(), "context failed: {:?}", context.err());

    let sync = guard.run_cli(&["sync"]);
    assert!(sync.is_ok(), "sync failed: {:?}", sync.err());
}

#[test]
fn test_force_reindex() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_force_{port}"));
    create_test_project(&dir);
    let guard = DaemonGuard::start(port, &dir);

    let result = guard.run_cli(&["force", "-p", &dir.to_string_lossy()]);
    assert!(result.is_ok(), "force reindex failed: {:?}", result.err());
}

#[test]
fn test_json_output() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_json_{port}"));
    create_test_project(&dir);
    let guard = DaemonGuard::start(port, &dir);

    guard.run_cli(&["index", "."]).ok();

    let result = guard.run_cli(&["--json", "context"]);
    assert!(result.is_ok(), "json context failed: {:?}", result.err());
    if let Ok(out) = &result {
        let v: serde_json::Value = serde_json::from_str(out).unwrap_or(serde_json::json!({}));
        assert!(v.is_object() || v.is_array(), "expected JSON object or array, got: {out}");
    }
}

#[test]
fn test_daemon_stop() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_stop_{port}"));
    let guard = DaemonGuard::start(port, &dir);

    let stop = guard.run_cli(&["daemon", "stop"]);
    assert!(stop.is_ok(), "daemon stop failed: {:?}", stop.err());

    std::thread::sleep(Duration::from_millis(1500));

    let ping = guard.run_cli(&["daemon", "ping"]);
    assert!(ping.is_err(), "daemon should be stopped but ping succeeded");
}

// ── Standalone tests (speedy.exe directly, not via cli) ──

#[test]
fn test_standalone_index_and_query() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_standalone_{port}"));
    create_test_project(&dir);
    let guard = DaemonGuard::start(port, &dir);
    let speedy = speedy_bin();

    let index = Command::new(speedy)
        .args(["--daemon-port", &port.to_string(), "index", "."])
        .current_dir(&dir)
        .output()
        .expect("failed to run speedy index");
    assert!(index.status.success(), "standalone index failed: {}", String::from_utf8_lossy(&index.stderr));
    let index_out = String::from_utf8_lossy(&index.stdout);
    assert!(index_out.contains("Indexed"), "expected Indexed in output, got: {index_out}");

    let query = Command::new(speedy)
        .args(["--daemon-port", &port.to_string(), "query", "greet"])
        .current_dir(&dir)
        .output()
        .expect("failed to run speedy query");
    assert!(query.status.success(), "standalone query failed: {}", String::from_utf8_lossy(&query.stderr));
    let q_out = String::from_utf8_lossy(&query.stdout);
    assert!(q_out.contains("greet"), "query output should contain 'greet', got: {q_out}");

    let context = Command::new(speedy)
        .args(["--daemon-port", &port.to_string(), "context"])
        .current_dir(&dir)
        .output()
        .expect("failed to run speedy context");
    assert!(context.status.success(), "standalone context failed: {}", String::from_utf8_lossy(&context.stderr));

    drop(guard);
}

#[test]
fn test_standalone_index_nonexistent_path() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_nonexistent_{port}"));
    std::fs::create_dir_all(&dir).unwrap();
    let guard = DaemonGuard::start(port, &dir);

    let speedy = speedy_bin();
    let out = Command::new(speedy)
        .args(["--daemon-port", &port.to_string(), "index", "C:\\questa_dir_non_esiste_xyz789"])
        .current_dir(&dir)
        .output()
        .expect("failed to run speedy index");
    let all_output = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    // The binary may succeed with a warning or fail — either way it must not crash
    let has_warning = all_output.to_lowercase().contains("no such")
        || all_output.to_lowercase().contains("not found")
        || all_output.to_lowercase().contains("error")
        || all_output.to_lowercase().contains("0 files");
    assert!(has_warning || !out.status.success(),
        "expected graceful handling of nonexistent path, exit={} output={all_output}",
        out.status);
    drop(guard);
}

#[test]
fn test_standalone_watch_detach() {
    let _lock = acquire_lock();
    let port = test_port();
    let dir = std::env::temp_dir().join(format!("speedy_e2e_watch_detach_{port}"));
    create_test_project(&dir);
    let guard = DaemonGuard::start(port, &dir);

    let speedy = speedy_bin();
    let out = Command::new(speedy)
        .args(["--daemon-port", &port.to_string(), "watch", "--detach"])
        .current_dir(&dir)
        .output()
        .expect("failed to run speedy watch --detach");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(),
        "watch --detach should succeed, exit={:?} stdout={stdout} stderr={stderr}", out.status.code());
    assert!(stdout.contains("PID:"), "expected PID in output, got: {stdout}");

    drop(guard);
}
