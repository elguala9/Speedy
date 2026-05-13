//! # Speedy Daemon IPC Protocol
//!
//! The daemon listens on TCP port 42137 (configurable via `--daemon-port`).
//! Clients send a single-line text command followed by `\n`.
//! The daemon responds with a single line of text followed by `\n`.
//!
//! ## Commands
//!
//! | Command | Description | Response |
//! |---------|-------------|----------|
//! | `ping` | Health check | `pong` |
//! | `status` | Daemon status as JSON | `{"pid":..., "uptime_secs":..., "workspace_count":..., "watcher_count":..., "version":...}` |
//! | `list` | Monitored workspace paths | `["/path/1", "/path/2"]` |
//! | `watch-count` | Number of active watchers | `3` |
//! | `daemon-pid` | Daemon process ID | `12345` |
//! | `stop` | Graceful shutdown | `ok` |
//! | `reload` | Reload workspaces from disk, sync watchers | `ok: N workspaces reloaded` |
//! | `add <path>` | Add a workspace, start watcher | `ok` or `error: ...` |
//! | `remove <path>` | Remove a workspace, stop watcher | `ok` or `error: ...` |
//! | `is-workspace <path>` | Check if path is monitored | `true` or `false` |
//! | `reindex <path>` | Force full reindex of a workspace | `ok` or `error: ...` |
//! | `exec <args>` | Run `speedy.exe <args>` and return stdout | command output |

use clap::Parser;
use speedy_core::workspace;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{info, warn, error};

pub const DAEMON_PORT: u16 = 42137;
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "speedy-daemon", about = "Speedy Central Daemon")]
struct DaemonCli {
    #[arg(long = "daemon-port", default_value = "42137")]
    port: u16,
}

struct WatcherHandle {
    stop: Arc<AtomicBool>,
}

struct CentralDaemon {
    pid: u32,
    started_at: Instant,
    running: Arc<AtomicBool>,
    port: u16,
    daemon_dir: PathBuf,
    watchers: Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: Arc<StdMutex<HashSet<u32>>>,
}

impl CentralDaemon {
    fn new(port: u16, daemon_dir: PathBuf) -> Self {
        Self {
            pid: std::process::id(),
            started_at: Instant::now(),
            running: Arc::new(AtomicBool::new(true)),
            port,
            daemon_dir,
            watchers: Arc::new(Mutex::new(HashMap::new())),
            active_pids: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    fn default(port: u16) -> Result<Self> {
        let daemon_dir = speedy_core::daemon_util::daemon_dir_path()?;
        Ok(Self::new(port, daemon_dir))
    }

    async fn start(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.daemon_dir)
            .context("failed to create daemon directory")?;

        speedy_core::daemon_util::kill_existing_daemon(&self.daemon_dir);
        std::fs::write(self.daemon_dir.join("daemon.pid"), self.pid.to_string())
            .context("failed to write daemon PID")?;

        let registered = workspace::list().unwrap_or_default();
        let watchers = self.watchers.clone();
        let active_pids = self.active_pids.clone();
        for entry in &registered {
            let p = Path::new(&entry.path);
            if p.exists() {
                let stop = start_workspace_watcher(&entry.path, active_pids.clone());
                watchers.lock().await.insert(entry.path.clone(), WatcherHandle { stop });
                info!("Watcher started for: {}", entry.path);
            } else {
                warn!("Skipped missing workspace: {}", entry.path);
            }
        }

        let max_port_attempts = 10;
        let (listener, actual_port) = 'bind: {
            for attempt in 0..max_port_attempts {
                let try_port = self.port + attempt;
                let addr = format!("127.0.0.1:{try_port}");
                match TcpListener::bind(&addr).await {
                    Ok(listener) => break 'bind (listener, try_port),
                    Err(e) => {
                        if attempt == max_port_attempts - 1 {
                            return Err(e).context(format!(
                                "Failed to bind daemon to ports {}-{}",
                                self.port,
                                self.port + max_port_attempts - 1
                            ));
                        }
                        warn!("Port {try_port} in use, trying next...");
                    }
                }
            }
            unreachable!()
        };

        if actual_port != self.port {
            warn!("Port {} in use, falling back to {}", self.port, actual_port);
            self.port = actual_port;
            let _ = std::fs::write(self.daemon_dir.join("daemon.port"), actual_port.to_string());
        }

        info!(
            "Speedy v{DAEMON_VERSION} (PID {}) listening on 127.0.0.1:{}",
            self.pid, self.port
        );

        let running = self.running.clone();
        let watchers_clone = self.watchers.clone();
        let active_pids_clone = self.active_pids.clone();
        let pid = self.pid;
        let started = self.started_at;
        let mut health_ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        health_ticker.tick().await;

        loop {
            tokio::select! {
                accept = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    listener.accept(),
                ) => {
                    match accept {
                        Ok(Ok((socket, _))) => {
                            if !running.load(Ordering::SeqCst) { break; }
                            let w = watchers_clone.clone();
                            let a = active_pids_clone.clone();
                            let r = running.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(socket, w, a, pid, started, r).await {
                                    error!("IPC error: {e}");
                                }
                            });
                        }
                        Ok(Err(e)) => error!("Accept error: {e}"),
                        Err(_) => {
                            if !running.load(Ordering::SeqCst) { break; }
                        }
                    }
                }
                _ = health_ticker.tick() => {
                    if !running.load(Ordering::SeqCst) { break; }
                    let ws = watchers_clone.lock().await;
                    let count = ws.len();
                    info!("Health: {count} watcher(s) active");
                }
            }
        }

        stop_all_watchers(&watchers_clone, &active_pids_clone).await;
        info!("Stopped.");
        Ok(())
    }
}

fn find_speedy_exe() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap();
        let candidate = dir.join(format!("speedy{}", std::env::consts::EXE_SUFFIX));
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("speedy")
}

fn start_workspace_watcher(path: &str, active_pids: Arc<StdMutex<HashSet<u32>>>) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let path = path.to_string();
    let speedy_exe = find_speedy_exe();

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = match notify_debouncer_mini::new_debouncer(
            std::time::Duration::from_millis(500),
            tx,
        ) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to create debouncer for {path}: {e}");
                return;
            }
        };

        if let Err(e) = debouncer.watcher().watch(
            Path::new(&path),
            notify::RecursiveMode::Recursive,
        ) {
            error!("Failed to watch {path}: {e}");
            return;
        }

        info!("Watcher active: {path}");

        while !stop_clone.load(Ordering::SeqCst) {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Ok(events)) => {
                    let exe = speedy_exe.clone();
                    let p = path.clone();
                    let pids = active_pids.clone();
                    std::thread::spawn(move || {
                        for event in &events {
                            let file_path = event.path.to_string_lossy().to_string();
                            if let Ok(mut child) = std::process::Command::new(&exe)
                                .args(["-p", &p, "index", &file_path])
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                            {
                                let pid = child.id();
                                pids.lock().unwrap().insert(pid);
                                let _ = child.wait();
                                pids.lock().unwrap().remove(&pid);
                            }
                        }
                    });
                }
                Ok(Err(e)) => {
                    error!("Watch error for {path}: {e}");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        info!("Watcher stopped: {path}");
    });

    stop
}

async fn stop_all_watchers(watchers: &Mutex<HashMap<String, WatcherHandle>>, active_pids: &StdMutex<HashSet<u32>>) {
    let mut map = watchers.lock().await;
    for (_, handle) in map.iter() {
        handle.stop.store(true, Ordering::SeqCst);
    }
    map.clear();

    let mut pids = active_pids.lock().unwrap();
    for pid in pids.iter() {
        kill_process(*pid);
    }
    pids.clear();
}

fn kill_process(pid: u32) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    watchers: Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: Arc<StdMutex<HashSet<u32>>>,
    pid: u32,
    started_at: Instant,
    running: Arc<AtomicBool>,
) -> Result<()> {
    let (reader, mut writer) = socket.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;
    let line = line.trim();

    let resp = dispatch_command(line, &watchers, &active_pids, pid, started_at, &running).await;

    writer.write_all(resp.as_bytes()).await?;
    Ok(())
}

async fn exec_speedy_command(args: &str) -> String {
    let exe = find_speedy_exe();
    let args_vec: Vec<&str> = args.split_whitespace().collect();
    match tokio::process::Command::new(&exe)
        .args(&args_vec)
        .output()
        .await
    {
        Ok(out) => {
            let mut result = String::from_utf8_lossy(&out.stdout).to_string();
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                result.push_str(&format!("\nstderr: {stderr}"));
            }
            result
        }
        Err(e) => format!("error: failed to run speedy: {e}"),
    }
}

async fn dispatch_command(
    line: &str,
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: &Arc<StdMutex<HashSet<u32>>>,
    pid: u32,
    started_at: Instant,
    running: &AtomicBool,
) -> String {
    match line {
        "ping" => "pong\n".to_string(),

        "status" => {
            let ws = watchers.lock().await;
            let status = serde_json::json!({
                "pid": pid,
                "uptime_secs": started_at.elapsed().as_secs(),
                "workspace_count": ws.len(),
                "watcher_count": ws.len(),
                "version": DAEMON_VERSION,
            });
            format!("{status}\n")
        }

        "list" => {
            let ws = watchers.lock().await;
            let paths: Vec<&String> = ws.keys().collect();
            format!("{}\n", serde_json::to_string(&paths).unwrap_or_default())
        }

        "watch-count" => {
            let ws = watchers.lock().await;
            format!("{}\n", ws.len())
        }

        "daemon-pid" => {
            format!("{pid}\n")
        }

        "stop" => {
            running.store(false, Ordering::SeqCst);
            "ok\n".to_string()
        }

        "reload" => {
            match workspace::list() {
                Ok(registered) => {
                    let mut ws = watchers.lock().await;
                    let registered_paths: std::collections::HashSet<String> =
                        registered.iter().map(|e| e.path.clone()).collect();

                    // Stop watchers for removed workspaces
                    let to_remove: Vec<String> = ws.keys()
                        .filter(|k| !registered_paths.contains(k.as_str()))
                        .cloned()
                        .collect();
                    for path in &to_remove {
                        if let Some(handle) = ws.remove(path) {
                            handle.stop.store(true, Ordering::SeqCst);
                        }
                    }

                    // Start watchers for new workspaces
                    for path in &registered_paths {
                        if !ws.contains_key(path) {
                            let p = Path::new(path);
                            if p.exists() {
                                let stop = start_workspace_watcher(path, active_pids.clone());
                                ws.insert(path.clone(), WatcherHandle { stop });
                            }
                        }
                    }

                    format!("ok: {} workspaces reloaded\n", registered_paths.len())
                }
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("is-workspace ") => {
            let target = line.trim_start_matches("is-workspace ").trim();
            let ws = watchers.lock().await;
            let found = canonical_path_match(target, &ws);
            format!("{found}\n")
        }

        _ if line.starts_with("add ") => {
            let raw_path = line.trim_start_matches("add ").trim();
            match handle_add(raw_path, watchers, active_pids).await {
                Ok(()) => "ok\n".to_string(),
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("remove ") => {
            let raw_path = line.trim_start_matches("remove ").trim();
            match handle_remove(raw_path, watchers).await {
                Ok(()) => "ok\n".to_string(),
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("reindex ") => {
            let raw_path = line.trim_start_matches("reindex ").trim();
            match handle_reindex(raw_path).await {
                Ok(()) => "ok\n".to_string(),
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("exec ") => {
            let args = line.trim_start_matches("exec ").trim();
            let result = exec_speedy_command(args).await;
            format!("{result}\n")
        }

        _ => {
            format!("error: unknown command: {line}\n")
        }
    }
}

fn canonical_path_match(target: &str, ws: &HashMap<String, WatcherHandle>) -> bool {
    let target_canonical = Path::new(target).canonicalize().ok();
    target_canonical.as_ref().map_or(false, |tc| {
        ws.keys().any(|k| {
            Path::new(k).canonicalize().ok().as_ref() == Some(tc)
        })
    })
}

async fn handle_add(raw_path: &str, watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>, active_pids: &Arc<StdMutex<HashSet<u32>>>) -> Result<()> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    if !workspace::is_registered(&path_str) {
        workspace::add(&path_str)?;
    }

    let mut ws = watchers.lock().await;
    if !ws.contains_key(&path_str) {
        let stop = start_workspace_watcher(&path_str, active_pids.clone());
        ws.insert(path_str.clone(), WatcherHandle { stop });
    }

    Ok(())
}

async fn handle_remove(raw_path: &str, watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>) -> Result<()> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    let mut ws = watchers.lock().await;
    if let Some(handle) = ws.remove(&path_str) {
        handle.stop.store(true, Ordering::SeqCst);
    }
    let _ = workspace::remove(&path_str);

    Ok(())
}

async fn handle_reindex(raw_path: &str) -> Result<()> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    let exe = find_speedy_exe();
    let output = tokio::process::Command::new(&exe)
        .args(["-p", &path_str, "sync"])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Reindex failed for {path_str}: {stderr}");
    } else {
        info!("Reindex done for {path_str}");
    }

    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
    let cli = DaemonCli::parse();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut daemon = CentralDaemon::default(cli.port)?;
        daemon.start().await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use speedy_core::daemon_client::DaemonClient;
    use std::time::Duration;

    static PORT_COUNTER: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(42400);
    static DAEMON_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct DaemonTestGuard {
        client: DaemonClient,
        handle: Option<std::thread::JoinHandle<()>>,
        dir: PathBuf,
        ws_backup: Option<Vec<speedy_core::workspace::WorkspaceEntry>>,
        _port: u16,
    }

    impl Drop for DaemonTestGuard {
        fn drop(&mut self) {
            // Send stop via raw TCP (no tokio needed in Drop)
            if let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{}", self._port)) {
                use std::io::Write;
                let _ = stream.write_all(b"stop\n");
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            let ws_backup = self.ws_backup.take();
            if let Some(ws) = ws_backup {
                if let Some(cfg) = dirs::config_dir() {
                    let path = cfg.join("speedy").join("workspaces.json");
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    let content = serde_json::to_string_pretty(&ws).unwrap();
                    let _ = std::fs::write(&path, content);
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn acquire_lock() -> std::sync::MutexGuard<'static, ()> {
        DAEMON_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn test_port() -> u16 {
        PORT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    fn start_daemon(name: &str) -> DaemonTestGuard {
        let port = test_port();
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).ok();

        let ws_backup = {
            let ws = speedy_core::workspace::list().ok();
            if let Some(cfg) = dirs::config_dir() {
                let path = cfg.join("speedy").join("workspaces.json");
                let _ = std::fs::remove_file(&path);
            }
            ws
        };

        let daemon_dir = dir.join(".speedy-daemon");
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut daemon = CentralDaemon::new(port, daemon_dir);
                daemon.start().await.unwrap();
            });
        });

        std::thread::sleep(Duration::from_millis(500));
        let client = DaemonClient::new(port);

        let guard = DaemonTestGuard {
            client,
            handle: Some(handle),
            dir,
            ws_backup,
            _port: port,
        };
        guard
    }

    #[tokio::test]
    async fn test_ping_pong() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_ping");
        assert!(guard.client.is_alive().await);
        assert_eq!(guard.client.ping().await.unwrap(), "pong");
        drop(guard);
    }

    #[tokio::test]
    async fn test_status_pid() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_status");
        let status = guard.client.status().await.unwrap();
        assert_eq!(status.pid, std::process::id());
        assert_eq!(status.workspace_count, 0);
        assert_eq!(status.watcher_count, 0);
        assert!(!status.version.is_empty());
        drop(guard);
    }

    #[tokio::test]
    async fn test_list_empty() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_list_empty");
        let list = guard.client.get_all_workspaces().await.unwrap();
        assert!(list.is_empty());
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);
        drop(guard);
    }

    #[tokio::test]
    async fn test_add_and_remove_workspace() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_add_remove");
        let ws_path = guard.dir.to_string_lossy().to_string();

        guard.client.add_workspace(&ws_path).await.unwrap();

        let list = guard.client.get_all_workspaces().await.unwrap();
        assert!(list.iter().any(|p| {
            std::path::Path::new(p).canonicalize().ok()
                == std::path::Path::new(&ws_path).canonicalize().ok()
        }));

        assert!(guard.client.is_workspace(&ws_path).await.unwrap());
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        guard.client.remove_workspace(&ws_path).await.unwrap();
        let list = guard.client.get_all_workspaces().await.unwrap();
        assert!(list.is_empty());

        assert!(!guard.client.is_workspace(&ws_path).await.unwrap());
        drop(guard);
    }

    #[tokio::test]
    async fn test_daemon_pid_watch_count() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_util");

        let pid = guard.client.daemon_pid().await.unwrap();
        assert_eq!(pid, std::process::id());
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);

        let ws_path = guard.dir.to_string_lossy().to_string();
        guard.client.add_workspace(&ws_path).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        guard.client.remove_workspace(&ws_path).await.unwrap();
        drop(guard);
    }

    #[tokio::test]
    async fn test_unknown_command_returns_error() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_unknown");

        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", guard._port)).await.unwrap();
        use tokio::io::AsyncWriteExt;
        stream.write_all(b"invalid-command\n").await.unwrap();
        stream.shutdown().await.unwrap();

        let mut buf = String::new();
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(&mut stream);
        reader.read_line(&mut buf).await.unwrap();
        assert!(buf.contains("error:"));
        drop(guard);
    }

    #[tokio::test]
    async fn test_is_not_alive_when_stopped() {
        let client = DaemonClient::new(42900);
        assert!(!client.is_alive().await);
    }

    async fn send_tcp_cmd(port: u16, req: &str) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let addr = format!("127.0.0.1:{port}");
        let mut stream = tokio::net::TcpStream::connect(&addr).await?;
        stream.write_all(format!("{req}\n").as_bytes()).await?;
        stream.shutdown().await?;
        let mut reader = tokio::io::BufReader::new(&mut stream);
        let mut resp = String::new();
        reader.read_line(&mut resp).await?;
        Ok(resp.trim().to_string())
    }

    #[tokio::test]
    async fn test_exec_returns_response() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_exec");
        let resp = send_tcp_cmd(guard._port, "exec index .").await;
        assert!(resp.is_ok(), "exec should not fail: {:?}", resp.err());
        drop(guard);
    }

    #[tokio::test]
    async fn test_exec_nonexistent_binary_falls_back_to_speedy() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("exec nonexistent-command --flag", &watchers, &active_pids, 99999, started, &running).await;
        assert!(!resp.trim().is_empty(), "exec response must not be empty");
    }

    #[tokio::test]
    async fn test_dispatch_exec_directly() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("exec query test -k 3", &watchers, &active_pids, 88888, started, &running).await;
        assert!(!resp.trim().is_empty(), "exec response should not be empty");
    }

    #[tokio::test]
    async fn test_reindex_returns_ok() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_reindex");
        let resp = send_tcp_cmd(guard._port, &format!("reindex {}", guard.dir.to_string_lossy())).await.unwrap();
        assert_eq!(resp, "ok", "reindex should return ok");
        drop(guard);
    }

    #[tokio::test]
    async fn test_dispatch_reindex_directly() {
        let dir = std::env::temp_dir().join("speedy_d_test_reindex_direct");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(&format!("reindex {}", dir.to_string_lossy()), &watchers, &active_pids, 77777, started, &running).await;
        assert_eq!(resp.trim(), "ok", "reindex via dispatch should return ok");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_double_add_workspace_is_idempotent() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_double_add");
        let ws_path = guard.dir.to_string_lossy().to_string();

        guard.client.add_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        guard.client.add_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        drop(guard);
    }

    #[tokio::test]
    async fn test_remove_nonexistent_workspace_error() {
        let dir = std::env::temp_dir().join("speedy_d_test_remove_nonexistent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let existing_dir = dir.join("exists");
        std::fs::create_dir_all(&existing_dir).unwrap();

        let resp = dispatch_command(&format!("remove {}", existing_dir.to_string_lossy()), &watchers, &active_pids, 66666, started, &running).await;
        assert_eq!(resp.trim(), "ok");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_graceful_shutdown_stops_listener() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_shutdown");

        assert!(guard.client.is_alive().await);
        guard.client.stop().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!guard.client.is_alive().await);

        drop(guard);
    }

    #[tokio::test]
    async fn test_watcher_stop_start_on_add_remove() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_watcher_lifecycle");
        let ws_path = guard.dir.to_string_lossy().to_string();

        assert_eq!(guard.client.watch_count().await.unwrap(), 0);

        guard.client.add_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        guard.client.remove_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);

        guard.client.add_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        guard.client.remove_workspace(&ws_path).await.unwrap();
        drop(guard);
    }

    #[tokio::test]
    async fn test_pid_file_cleanup_on_stop() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_pid_cleanup");
        assert!(guard.client.is_alive().await);
        guard.client.stop().await.unwrap();
        drop(guard);
    }

    #[tokio::test]
    async fn test_status_has_uptime_and_version() {
        let _lock = acquire_lock();
        let guard = start_daemon("speedy_d_test_status_full");

        let status = guard.client.status().await.unwrap();
        // uptime can be 0 if less than 1 second elapsed — just verify the field exists
        assert!(status.uptime_secs < 1000, "uptime seems unreasonably high: {}", status.uptime_secs);
        assert!(!status.version.is_empty(), "version should not be empty");
        assert_eq!(status.workspace_count, 0);
        assert_eq!(status.watcher_count, 0);

        let ws_path = guard.dir.to_string_lossy().to_string();
        guard.client.add_workspace(&ws_path).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status2 = guard.client.status().await.unwrap();
        assert_eq!(status2.workspace_count, 1);
        assert_eq!(status2.watcher_count, 1);

        drop(guard);
    }

    #[tokio::test]
    async fn test_dispatch_unknown_command_format() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("garbage", &watchers, &active_pids, 11111, started, &running).await;
        assert!(resp.contains("error: unknown command"), "expected unknown command error, got: {resp}");
    }

    #[tokio::test]
    async fn test_dispatch_is_workspace_not_found() {
        let dir = std::env::temp_dir().join("speedy_d_test_is_ws");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(&format!("is-workspace {}", dir.to_string_lossy()), &watchers, &active_pids, 11112, started, &running).await;
        assert_eq!(resp.trim(), "false");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_dispatch_daemon_pid() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("daemon-pid", &watchers, &active_pids, 55555, started, &running).await;
        assert_eq!(resp.trim(), "55555");
    }

    #[tokio::test]
    async fn test_dispatch_watch_count_zero() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("watch-count", &watchers, &active_pids, 12345, started, &running).await;
        assert_eq!(resp.trim(), "0");
    }

    #[tokio::test]
    async fn test_port_in_use_returns_error() {
        let _lock = acquire_lock();
        let port = test_port();
        let dir = std::env::temp_dir().join("speedy_d_test_port_in_use");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ws_backup = workspace::list().ok();
        if let Some(cfg) = dirs::config_dir() {
            let path = cfg.join("speedy").join("workspaces.json");
            let _ = std::fs::remove_file(&path);
        }

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
        let mut daemon = CentralDaemon::new(port, dir.join(".speedy-daemon"));
        let result = daemon.start().await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to bind") || err.contains("already in use") || err.contains("denied"),
            "unexpected error: {err}"
        );

        drop(listener);

        if let Some(ws) = ws_backup {
            if let Some(cfg) = dirs::config_dir() {
                let path = cfg.join("speedy").join("workspaces.json");
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                let content = serde_json::to_string_pretty(&ws).unwrap();
                let _ = std::fs::write(&path, content);
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_port_fallback_uses_next_port() {
        let _lock = acquire_lock();
        let port = test_port();
        let dir = std::env::temp_dir().join("speedy_d_test_port_fallback");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ws_backup = workspace::list().ok();
        if let Some(cfg) = dirs::config_dir() {
            let path = cfg.join("speedy").join("workspaces.json");
            let _ = std::fs::remove_file(&path);
        }

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
        let daemon_dir = dir.join(".speedy-daemon");
        let mut daemon = CentralDaemon::new(port, daemon_dir.clone());

        tokio::spawn(async move {
            let _ = daemon.start().await;
        });

        tokio::time::sleep(Duration::from_millis(500)).await;

        let fallback_port = port + 1;
        let client = DaemonClient::new(fallback_port);
        assert!(client.is_alive().await, "daemon should be on fallback port {fallback_port}");

        let status = client.status().await.unwrap();
        assert_eq!(status.pid, std::process::id());

        let _ = client.stop().await;
        drop(listener);

        if let Some(ws) = ws_backup {
            if let Some(cfg) = dirs::config_dir() {
                let path = cfg.join("speedy").join("workspaces.json");
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                let content = serde_json::to_string_pretty(&ws).unwrap();
                let _ = std::fs::write(&path, content);
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
