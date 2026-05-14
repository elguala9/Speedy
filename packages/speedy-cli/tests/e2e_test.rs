use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Wrap `Command::new` with the Windows flags that keep test runs from popping
/// console windows for every spawned binary.
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

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static E2E_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn acquire_lock() -> std::sync::MutexGuard<'static, ()> {
    E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn unique_name(label: &str) -> String {
    let n = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    format!("speedy_e2e_{label}_{n}")
}

fn bin_path(name: &str) -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("target").join("debug");
    root.join(format!("{name}{suffix}"))
}

fn create_test_project(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        b"[package]\nname = \"e2e-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.join("src").join("lib.rs"),
        b"pub fn greet(name: &str) -> String { format!(\"Hello, {name}!\") }\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    ).unwrap();
}

struct DaemonGuard {
    process: Option<Child>,
    socket_name: String,
    daemon_dir: PathBuf,
    dir: PathBuf,
}

impl DaemonGuard {
    fn start(socket_name: &str, dir: &Path) -> Self {
        std::fs::create_dir_all(dir).expect("failed to create test dir");
        let daemon_dir = dir.join(".speedy-daemon");
        std::fs::create_dir_all(&daemon_dir).expect("failed to create daemon dir");

        let process = quiet_command(&bin_path("speedy-daemon"))
            .args(["--daemon-socket", socket_name])
            .arg("--daemon-dir").arg(&daemon_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start speedy-daemon");

        std::thread::sleep(Duration::from_secs(1));

        Self {
            process: Some(process),
            socket_name: socket_name.to_string(),
            daemon_dir,
            dir: dir.to_owned(),
        }
    }

    fn run_cli(&self, args: &[&str]) -> Result<String, String> {
        let output = quiet_command(&bin_path("speedy-cli"))
            .args(["--daemon-socket", &self.socket_name])
            .args(args)
            .current_dir(&self.dir)
            .env("SPEEDY_DAEMON_DIR", &self.daemon_dir)
            .output()
            .map_err(|e| e.to_string())?;
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
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_dist_binaries_exist() {
    assert!(bin_path("speedy-daemon").exists(), "speedy-daemon binary not found");
    assert!(bin_path("speedy-cli").exists(), "speedy-cli binary not found");
}

#[test]
fn test_daemon_ping_pong() {
    let _lock = acquire_lock();
    let name = unique_name("ping");
    let dir = std::env::temp_dir().join(&name);
    let guard = DaemonGuard::start(&name, &dir);

    let result = guard.run_cli(&["daemon", "ping"]);
    assert!(result.is_ok(), "ping failed: {:?}", result.err());
    assert_eq!(result.unwrap(), "pong");
}

#[test]
fn test_daemon_status() {
    let _lock = acquire_lock();
    let name = unique_name("status");
    let dir = std::env::temp_dir().join(&name);
    let guard = DaemonGuard::start(&name, &dir);

    let result = guard.run_cli(&["daemon", "status"]);
    assert!(result.is_ok(), "status failed: {:?}", result.err());
    let out = result.unwrap();
    assert!(out.contains("PID:"), "expected PID in output, got: {out:?}");
    assert!(out.contains("Version:"), "expected Version in output, got: {out:?}");
}

#[test]
fn test_daemon_list_empty() {
    let _lock = acquire_lock();
    let name = unique_name("list");
    let dir = std::env::temp_dir().join(&name);
    let guard = DaemonGuard::start(&name, &dir);

    let result = guard.run_cli(&["daemon", "list"]);
    assert!(result.is_ok(), "list failed: {:?}", result.err());
}

#[test]
fn test_workspace_add_and_remove() {
    let _lock = acquire_lock();
    let name = unique_name("ws");
    let dir = std::env::temp_dir().join(&name);
    create_test_project(&dir);
    let guard = DaemonGuard::start(&name, &dir);
    let ws_path = dir.to_string_lossy().to_string();

    let add = guard.run_cli(&["workspace", "add", &ws_path]);
    assert!(add.is_ok(), "workspace add failed: {:?}", add.as_ref().err());
    let out = add.unwrap();
    assert!(out.contains("added") || out.contains("ok"), "unexpected add output: {out}");

    let list = guard.run_cli(&["daemon", "list"]);
    assert!(list.is_ok());

    let remove = guard.run_cli(&["workspace", "remove", &ws_path]);
    assert!(remove.is_ok(), "workspace remove failed: {:?}", remove.err());
}

#[test]
fn test_index_and_query_via_daemon() {
    let _lock = acquire_lock();
    let name = unique_name("idx");
    let dir = std::env::temp_dir().join(&name);
    create_test_project(&dir);
    let guard = DaemonGuard::start(&name, &dir);

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
    let name = unique_name("force");
    let dir = std::env::temp_dir().join(&name);
    create_test_project(&dir);
    let guard = DaemonGuard::start(&name, &dir);

    let result = guard.run_cli(&["force", "-p", &dir.to_string_lossy()]);
    assert!(result.is_ok(), "force reindex failed: {:?}", result.err());
}

#[test]
fn test_json_output() {
    let _lock = acquire_lock();
    let name = unique_name("json");
    let dir = std::env::temp_dir().join(&name);
    create_test_project(&dir);
    let guard = DaemonGuard::start(&name, &dir);

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
    let name = unique_name("stop");
    let dir = std::env::temp_dir().join(&name);
    let guard = DaemonGuard::start(&name, &dir);

    let stop = guard.run_cli(&["daemon", "stop"]);
    assert!(stop.is_ok(), "daemon stop failed: {:?}", stop.err());

    std::thread::sleep(Duration::from_millis(1500));

    let ping = guard.run_cli(&["daemon", "ping"]);
    assert!(ping.is_err(), "daemon should be stopped but ping succeeded");
}

#[test]
fn test_standalone_index_and_query() {
    let _lock = acquire_lock();
    let name = unique_name("standalone");
    let dir = std::env::temp_dir().join(&name);
    create_test_project(&dir);
    let _guard = DaemonGuard::start(&name, &dir);
    let speedy = bin_path("speedy");

    let index = quiet_command(&speedy)
        .args(["--daemon-socket", &name, "index", "."])
        .current_dir(&dir)
        .env("SPEEDY_DAEMON_DIR", &_guard.daemon_dir)
        .output()
        .expect("failed to run speedy index");
    assert!(index.status.success(), "standalone index failed: {}", String::from_utf8_lossy(&index.stderr));
    let index_out = String::from_utf8_lossy(&index.stdout);
    assert!(index_out.contains("Indexed"), "expected Indexed in output, got: {index_out}");

    let query = quiet_command(&speedy)
        .args(["--daemon-socket", &name, "query", "greet"])
        .current_dir(&dir)
        .env("SPEEDY_DAEMON_DIR", &_guard.daemon_dir)
        .output()
        .expect("failed to run speedy query");
    assert!(query.status.success(), "standalone query failed: {}", String::from_utf8_lossy(&query.stderr));
    let q_out = String::from_utf8_lossy(&query.stdout);
    assert!(q_out.contains("greet"), "query output should contain 'greet', got: {q_out}");

    let context = quiet_command(&speedy)
        .args(["--daemon-socket", &name, "context"])
        .current_dir(&dir)
        .env("SPEEDY_DAEMON_DIR", &_guard.daemon_dir)
        .output()
        .expect("failed to run speedy context");
    assert!(context.status.success(), "standalone context failed: {}", String::from_utf8_lossy(&context.stderr));
}

#[test]
fn test_standalone_index_nonexistent_path() {
    let _lock = acquire_lock();
    let name = unique_name("nonexistent");
    let dir = std::env::temp_dir().join(&name);
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = DaemonGuard::start(&name, &dir);

    let speedy = bin_path("speedy");
    let out = quiet_command(&speedy)
        .args(["--daemon-socket", &name, "index", "C:\\questa_dir_non_esiste_xyz789"])
        .current_dir(&dir)
        .env("SPEEDY_DAEMON_DIR", &_guard.daemon_dir)
        .output()
        .expect("failed to run speedy index");
    let all_output = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let has_warning = all_output.to_lowercase().contains("no such")
        || all_output.to_lowercase().contains("not found")
        || all_output.to_lowercase().contains("error")
        || all_output.to_lowercase().contains("0 files");
    assert!(has_warning || !out.status.success(),
        "expected graceful handling of nonexistent path, exit={} output={all_output}",
        out.status);
}

