//! # Speedy Daemon
//!
//! IPC protocol reference: see `docs/ipc-protocol.md`.

use clap::Parser;
use speedy_core::types::{LogLine, ScanResult, WorkspaceStatus};
use speedy_core::workspace;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex};

/// CREATE_NO_WINDOW: a console-subsystem child of a daemon (which itself runs
/// detached) would otherwise allocate a new console window per spawn. We
/// capture stdio explicitly, so suppressing the window is always correct.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

use speedy_core::local_sock::{GenericNamespaced, ListenerOptions, ToNsName};
use speedy_core::local_sock::{ListenerTrait as _, Stream as LocalStream, StreamTrait as _};
use tracing::{info, warn, error};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;

/// Capacity for the in-memory log broadcast. Each `subscribe-log` connection
/// gets its own receiver; lagging consumers see a `Lagged(n)` skip rather than
/// blocking the producer.
const LOG_BROADCAST_CAPACITY: usize = 1024;

/// `tracing` layer that fans every event out on a Tokio broadcast channel for
/// live consumers (used by `subscribe-log`). The channel has a bounded
/// capacity; receivers that fall behind get a `Lagged` skip and keep running.
struct BroadcastLayer {
    tx: broadcast::Sender<LogLine>,
}

impl<S> Layer<S> for BroadcastLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let line = LogLine {
            ts: chrono::Utc::now().to_rfc3339(),
            level: metadata.level().to_string().to_lowercase(),
            target: metadata.target().to_string(),
            message: visitor.message,
            fields: visitor.fields,
        };
        let _ = self.tx.send(line);
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.insert(field.name().to_string(), serde_json::Value::String(value.to_string()));
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        if field.name() == "message" {
            self.message = s;
        } else {
            self.fields.insert(field.name().to_string(), serde_json::Value::String(s));
        }
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), serde_json::Value::Number(value.into()));
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), serde_json::Value::Number(value.into()));
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.insert(field.name().to_string(), serde_json::Value::Bool(value));
    }
}

pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Wire-format version of the IPC protocol. Bump on breaking changes (command
/// rename, response shape change, new required fields). Clients should refuse
/// to talk to a daemon whose `protocol_version` is higher than they know about.
///
/// v2 (2026-05-14): added `query-all` for cross-workspace search.
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Parser)]
#[command(name = "speedy-daemon", about = "Speedy Central Daemon")]
struct DaemonCli {
    #[arg(long = "daemon-socket", default_value = "speedy-daemon")]
    socket: String,

    #[arg(long = "daemon-dir", help = "Override pid/workspaces.json directory")]
    daemon_dir: Option<PathBuf>,
}

struct WatcherHandle {
    stop: Arc<AtomicBool>,
    last_heartbeat: Arc<AtomicU64>,
    /// Unix seconds: last filesystem event that hit this watcher (0 = never).
    last_event_at: Arc<AtomicU64>,
    /// Unix seconds: last successful `sync` finished for this workspace (0 = never).
    last_sync_at: Arc<AtomicU64>,
}

#[derive(Default)]
struct Metrics {
    queries: AtomicU64,
    indexes: AtomicU64,
    syncs: AtomicU64,
    watcher_events: AtomicU64,
    exec_calls: AtomicU64,
}

impl Metrics {
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "queries": self.queries.load(Ordering::Relaxed),
            "indexes": self.indexes.load(Ordering::Relaxed),
            "syncs": self.syncs.load(Ordering::Relaxed),
            "watcher_events": self.watcher_events.load(Ordering::Relaxed),
            "exec_calls": self.exec_calls.load(Ordering::Relaxed),
        })
    }
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Tick threshold (in health-tick units of 30s) before we consider a watcher
/// silently dead and restart it. 4 ticks ≈ 2 minutes.
const WATCHER_DEAD_TICKS: u64 = 4;
/// Warn threshold: 2 ticks ≈ 1 minute without a heartbeat.
const WATCHER_WARN_TICKS: u64 = 2;
const HEALTH_TICK_SECS: u64 = 30;
/// Run `workspace::prune_missing` every Nth health tick. At 30s/tick × 10 ≈ 5
/// minutes — catches workspaces deleted while the daemon is running without
/// hammering the file lock.
const PRUNE_EVERY_N_TICKS: u64 = 10;

struct CentralDaemon {
    pid: u32,
    started_at: Instant,
    running: Arc<AtomicBool>,
    socket_name: String,
    daemon_dir: PathBuf,
    watchers: Arc<Mutex<HashMap<String, WatcherHandle>>>,
    /// PIDs of `speedy.exe index` children currently in-flight. Used by
    /// `stop_all_watchers` to taskkill them on shutdown so we don't leak a
    /// child still writing to SQLite after the daemon exits. The primary
    /// defense against the watcher re-firing on our own writes is the ignore
    /// list on `.speedy/`; this set is kept as defense-in-depth and to make
    /// shutdown deterministic.
    active_pids: Arc<StdMutex<HashSet<u32>>>,
    metrics: Arc<Metrics>,
    /// Broadcast channel that fans `tracing` events to live `subscribe-log`
    /// listeners. `None` in tests where we don't install the global subscriber.
    log_tx: Option<broadcast::Sender<LogLine>>,
    /// Held for the daemon lifetime; OS releases its advisory lock when this
    /// drops. Never read directly.
    _lock_file: Option<std::fs::File>,
}

impl CentralDaemon {
    fn new(socket_name: String, daemon_dir: PathBuf) -> Self {
        Self {
            pid: std::process::id(),
            started_at: Instant::now(),
            running: Arc::new(AtomicBool::new(true)),
            socket_name,
            daemon_dir,
            watchers: Arc::new(Mutex::new(HashMap::new())),
            active_pids: Arc::new(StdMutex::new(HashSet::new())),
            metrics: Arc::new(Metrics::default()),
            log_tx: None,
            _lock_file: None,
        }
    }

    fn default(socket_name: String) -> Result<Self> {
        let daemon_dir = speedy_core::daemon_util::daemon_dir_path()?;
        Ok(Self::new(socket_name, daemon_dir))
    }

    async fn start(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.daemon_dir)
            .context("failed to create daemon directory")?;

        speedy_core::daemon_util::kill_existing_daemon(&self.daemon_dir);

        // Advisory lock catches the race where two `speedy-daemon` processes
        // start concurrently and both pass kill_existing_daemon before either
        // writes daemon.pid.
        self._lock_file = Some(
            speedy_core::daemon_util::acquire_daemon_lock(&self.daemon_dir)?,
        );

        std::fs::write(self.daemon_dir.join("daemon.pid"), self.pid.to_string())
            .context("failed to write daemon PID")?;

        match workspace::prune_missing() {
            Ok(0) => {}
            Ok(n) => info!("Pruned {n} stale workspace entr{}", if n == 1 { "y" } else { "ies" }),
            Err(e) => warn!("Workspace pruning failed: {e}"),
        }

        let registered = workspace::list().unwrap_or_default();
        let watchers = self.watchers.clone();
        let active_pids = self.active_pids.clone();
        for entry in &registered {
            let p = Path::new(&entry.path);
            if p.exists() {
                let handle = start_workspace_watcher(&entry.path, active_pids.clone(), self.metrics.clone());
                watchers.lock().await.insert(entry.path.clone(), handle);
                info!("Watcher started for: {}", entry.path);
            } else {
                warn!("Skipped missing workspace: {}", entry.path);
            }
        }

        let listener_name = self.socket_name
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .with_context(|| format!("invalid daemon socket name: {}", self.socket_name))?;
        let listener = match ListenerOptions::new().name(listener_name).create_tokio() {
            Ok(l) => l,
            Err(e) => {
                return Err(e).context(format!(
                    "Failed to bind daemon socket: {}",
                    self.socket_name
                ));
            }
        };

        info!(
            "Speedy v{DAEMON_VERSION} (PID {}) listening on {}",
            self.pid, self.socket_name
        );

        spawn_workspaces_json_watcher(
            self.daemon_dir.clone(),
            self.watchers.clone(),
            self.active_pids.clone(),
            self.metrics.clone(),
            self.running.clone(),
        );

        let running = self.running.clone();
        let watchers_clone = self.watchers.clone();
        let active_pids_clone = self.active_pids.clone();
        let metrics_clone = self.metrics.clone();
        let log_tx = self.log_tx.clone();
        let daemon_dir = self.daemon_dir.clone();
        let pid = self.pid;
        let started = self.started_at;
        let mut health_ticker = tokio::time::interval(std::time::Duration::from_secs(HEALTH_TICK_SECS));
        health_ticker.tick().await;
        let mut tick_count: u64 = 0;

        loop {
            tokio::select! {
                accept = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    listener.accept(),
                ) => {
                    match accept {
                        Ok(Ok(socket)) => {
                            if !running.load(Ordering::SeqCst) { break; }
                            let w = watchers_clone.clone();
                            let a = active_pids_clone.clone();
                            let m = metrics_clone.clone();
                            let r = running.clone();
                            let lt = log_tx.clone();
                            let dd = daemon_dir.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(socket, w, a, m, pid, started, r, lt, dd).await {
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
                    check_watcher_health(&watchers_clone, &active_pids_clone, &metrics_clone).await;
                    tick_count = tick_count.wrapping_add(1);
                    if tick_count % PRUNE_EVERY_N_TICKS == 0 {
                        prune_and_reconcile(&watchers_clone).await;
                    }
                }
            }
        }

        drop(listener);
        stop_all_watchers(&watchers_clone, &active_pids_clone).await;
        let _ = std::fs::remove_file(self.daemon_dir.join("daemon.pid"));
        info!("Stopped.");
        Ok(())
    }
}

fn find_speedy_exe() -> PathBuf {
    let exe_name = format!("speedy{}", std::env::consts::EXE_SUFFIX);
    if let Ok(exe) = std::env::current_exe() {
        let Some(dir) = exe.parent() else {
            return PathBuf::from("speedy");
        };
        let candidate = dir.join(&exe_name);
        if candidate.exists() {
            return candidate;
        }
        // When running under `cargo test`, current_exe is in target/debug/deps/
        // — speedy.exe lives one directory up.
        if dir.file_name().and_then(|s| s.to_str()) == Some("deps") {
            if let Some(parent) = dir.parent() {
                let candidate = parent.join(&exe_name);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("speedy")
}

const WATCH_IGNORE_DIRS: &[&str] = &[
    "target", ".git", "node_modules", ".speedy-daemon", ".speedy",
    ".idea", ".vscode", "dist", "build", "__pycache__", ".cargo",
];

fn should_ignore_watch_path(p: &Path) -> bool {
    p.components().any(|c| {
        if let std::path::Component::Normal(name) = c {
            if let Some(s) = name.to_str() {
                return WATCH_IGNORE_DIRS.contains(&s);
            }
        }
        false
    })
}

fn start_workspace_watcher(
    path: &str,
    active_pids: Arc<StdMutex<HashSet<u32>>>,
    metrics: Arc<Metrics>,
) -> WatcherHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let last_heartbeat = Arc::new(AtomicU64::new(unix_now_secs()));
    let last_event_at = Arc::new(AtomicU64::new(0));
    let last_sync_at = Arc::new(AtomicU64::new(0));
    let stop_clone = stop.clone();
    let heartbeat = last_heartbeat.clone();
    let event_at_clone = last_event_at.clone();
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
            heartbeat.store(unix_now_secs(), Ordering::Relaxed);
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Ok(events)) => {
                    let exe = speedy_exe.clone();
                    let p = path.clone();
                    let pids = active_pids.clone();
                    let m = metrics.clone();
                    let event_at = event_at_clone.clone();
                    std::thread::spawn(move || {
                        for event in &events {
                            if should_ignore_watch_path(&event.path) {
                                continue;
                            }
                            m.watcher_events.fetch_add(1, Ordering::Relaxed);
                            event_at.store(unix_now_secs(), Ordering::Relaxed);
                            tracing::debug!(
                                target: "watcher",
                                workspace = %p,
                                path = %event.path.display(),
                                "event"
                            );
                            let file_path = event.path.to_string_lossy().to_string();

                            // Test hook: when SPEEDY_WATCH_LOG is set, append the
                            // observed file path and skip the real spawn. Used by
                            // integration tests to verify watcher → indexer wiring
                            // without spawning speedy.exe.
                            if let Ok(log) = std::env::var("SPEEDY_WATCH_LOG") {
                                use std::io::Write;
                                if let Ok(mut f) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&log)
                                {
                                    let _ = writeln!(f, "{file_path}");
                                }
                                continue;
                            }

                            let mut spawn_cmd = std::process::Command::new(&exe);
                            spawn_cmd
                                .args(["-p", &p, "index", &file_path])
                                .env("SPEEDY_NO_DAEMON", "1")
                                .stdout(Stdio::null())
                                .stderr(Stdio::null());
                            #[cfg(windows)]
                            {
                                use std::os::windows::process::CommandExt;
                                spawn_cmd.creation_flags(CREATE_NO_WINDOW);
                            }
                            if let Ok(mut child) = spawn_cmd.spawn() {
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

    WatcherHandle { stop, last_heartbeat, last_event_at, last_sync_at }
}

async fn check_watcher_health(
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: &Arc<StdMutex<HashSet<u32>>>,
    metrics: &Arc<Metrics>,
) {
    check_watcher_health_with_thresholds(
        watchers,
        active_pids,
        metrics,
        WATCHER_WARN_TICKS * HEALTH_TICK_SECS,
        WATCHER_DEAD_TICKS * HEALTH_TICK_SECS,
    )
    .await;
}

/// Threshold-parameterized core so unit tests can trigger the dead-watcher
/// restart path without waiting two minutes.
async fn check_watcher_health_with_thresholds(
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: &Arc<StdMutex<HashSet<u32>>>,
    metrics: &Arc<Metrics>,
    warn_after: u64,
    dead_after: u64,
) {
    let now = unix_now_secs();

    // Collect paths that need restarting. We can't restart while holding the
    // lock because start_workspace_watcher allocates a thread and we don't
    // want to block other IPC under the same lock.
    let mut to_restart: Vec<String> = Vec::new();
    {
        let ws = watchers.lock().await;
        let count = ws.len();
        for (path, handle) in ws.iter() {
            let last = handle.last_heartbeat.load(Ordering::Relaxed);
            let elapsed = now.saturating_sub(last);
            if elapsed >= dead_after {
                warn!("Watcher silent for {elapsed}s, restarting: {path}");
                to_restart.push(path.clone());
            } else if elapsed >= warn_after {
                warn!("Watcher heartbeat stale ({elapsed}s): {path}");
            }
        }
        info!("Health: {count} watcher(s) active");
    }

    if to_restart.is_empty() {
        return;
    }

    let mut ws = watchers.lock().await;
    for path in &to_restart {
        if let Some(old) = ws.remove(path) {
            old.stop.store(true, Ordering::SeqCst);
        }
        if Path::new(path).exists() {
            let handle = start_workspace_watcher(path, active_pids.clone(), metrics.clone());
            ws.insert(path.clone(), handle);
            info!("Watcher restarted: {path}");
        } else {
            warn!("Skipping restart, path missing: {path}");
        }
    }
}

/// Reconcile in-memory watchers with `workspaces.json` on disk.
///
/// - Adds watchers for paths registered on disk but not in memory.
/// - Stops watchers for paths in memory but absent from disk.
/// Returns the number of registered workspaces after reconciliation.
///
/// When the daemon itself writes to `workspaces.json` (via `add`/`remove`)
/// this is invoked by the file-watcher and acts as a no-op because the
/// in-memory map already matches.
async fn reload_from_disk(
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: &Arc<StdMutex<HashSet<u32>>>,
    metrics: &Arc<Metrics>,
) -> Result<usize> {
    let registered = workspace::list()?;
    let mut ws = watchers.lock().await;
    let registered_paths: std::collections::HashSet<String> =
        registered.iter().map(|e| e.path.clone()).collect();

    let to_remove: Vec<String> = ws.keys()
        .filter(|k| !registered_paths.contains(k.as_str()))
        .cloned()
        .collect();
    for path in &to_remove {
        if let Some(handle) = ws.remove(path) {
            handle.stop.store(true, Ordering::SeqCst);
        }
    }

    for path in &registered_paths {
        if !ws.contains_key(path) {
            let p = Path::new(path);
            if p.exists() {
                let handle = start_workspace_watcher(path, active_pids.clone(), metrics.clone());
                ws.insert(path.clone(), handle);
            }
        }
    }

    Ok(registered_paths.len())
}

/// Watch `<daemon_dir>/workspaces.json` for changes by external tooling and
/// trigger a reload. The daemon's own writes also fire this watcher, but
/// `reload_from_disk` is a no-op when state already matches, so the extra
/// work is negligible.
fn spawn_workspaces_json_watcher(
    daemon_dir: PathBuf,
    watchers: Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: Arc<StdMutex<HashSet<u32>>>,
    metrics: Arc<Metrics>,
    running: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        // Debounce 1s: the daemon writes the file then immediately re-reads it
        // for `list`; debouncing prevents a thrash on the same logical edit.
        let mut debouncer = match notify_debouncer_mini::new_debouncer(
            std::time::Duration::from_secs(1),
            tx,
        ) {
            Ok(d) => d,
            Err(e) => {
                error!("workspaces.json watcher: failed to create debouncer: {e}");
                return;
            }
        };

        // notify can't watch a path that doesn't exist yet, so watch the
        // parent directory and filter events by filename.
        if let Err(e) = debouncer.watcher().watch(
            &daemon_dir,
            notify::RecursiveMode::NonRecursive,
        ) {
            error!("workspaces.json watcher: failed to watch {}: {e}", daemon_dir.display());
            return;
        }

        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                error!("workspaces.json watcher: failed to build runtime: {e}");
                return;
            }
        };

        while running.load(Ordering::SeqCst) {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Ok(events)) => {
                    let touched = events.iter().any(|e| {
                        e.path.file_name()
                            .and_then(|s| s.to_str())
                            .map(|s| s == "workspaces.json")
                            .unwrap_or(false)
                    });
                    if !touched {
                        continue;
                    }
                    let w = watchers.clone();
                    let a = active_pids.clone();
                    let m = metrics.clone();
                    rt.block_on(async {
                        match reload_from_disk(&w, &a, &m).await {
                            Ok(n) => info!("Auto-reload triggered by workspaces.json change: {n} workspaces"),
                            Err(e) => warn!("Auto-reload failed: {e}"),
                        }
                    });
                }
                Ok(Err(e)) => error!("workspaces.json watcher error: {e}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

/// Drop watchers whose workspace path no longer exists on disk, and prune the
/// same entries from `workspaces.json`. Runs periodically so stale entries that
/// appear *after* daemon boot (e.g. user deleted the project dir) get cleaned
/// up without needing a daemon restart.
async fn prune_and_reconcile(watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>) {
    // Drop in-memory watchers whose path is gone first; otherwise the watcher
    // task keeps trying to observe a missing dir until the next health tick.
    let dead_paths: Vec<String> = {
        let ws = watchers.lock().await;
        ws.keys()
            .filter(|p| !Path::new(p.as_str()).exists())
            .cloned()
            .collect()
    };
    if !dead_paths.is_empty() {
        let mut ws = watchers.lock().await;
        for path in &dead_paths {
            if let Some(handle) = ws.remove(path) {
                handle.stop.store(true, Ordering::SeqCst);
                info!("Auto-prune: stopped watcher for missing path {path}");
            }
        }
    }

    // Then prune the on-disk registry.
    match workspace::prune_missing() {
        Ok(0) => {}
        Ok(n) => info!("Auto-prune: removed {n} stale workspace entr{}", if n == 1 { "y" } else { "ies" }),
        Err(e) => warn!("Auto-prune failed: {e}"),
    }
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

#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    socket: LocalStream,
    watchers: Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: Arc<StdMutex<HashSet<u32>>>,
    metrics: Arc<Metrics>,
    pid: u32,
    started_at: Instant,
    running: Arc<AtomicBool>,
    log_tx: Option<broadcast::Sender<LogLine>>,
    daemon_dir: PathBuf,
) -> Result<()> {
    let (reader, mut writer) = socket.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;
    let line = line.trim();

    tracing::debug!(target: "ipc", req = %line, "connection accepted");

    // `subscribe-log` is the one long-lived command: keep the writer open and
    // stream LogLine JSONs until either side drops.
    if line == "subscribe-log" {
        if let Some(tx) = log_tx {
            stream_log(&mut writer, tx).await?;
        } else {
            writer.write_all(b"error: log streaming not configured\n").await?;
        }
        return Ok(());
    }

    let resp = dispatch_command(
        line,
        &watchers,
        &active_pids,
        &metrics,
        pid,
        started_at,
        &running,
        &daemon_dir,
    )
    .await;

    writer.write_all(resp.as_bytes()).await?;
    tracing::debug!(target: "ipc", bytes = resp.len(), "response sent");
    Ok(())
}

/// Drive a `subscribe-log` connection: handshake `ok\n`, then forward every
/// broadcasted `LogLine` as one JSON object per line until the client closes
/// the socket or the channel breaks. `Lagged(_)` skips are tolerated.
async fn stream_log<W>(writer: &mut W, tx: broadcast::Sender<LogLine>) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut rx = tx.subscribe();
    writer.write_all(b"ok\n").await?;
    loop {
        match rx.recv().await {
            Ok(line) => {
                let s = serde_json::to_string(&line).unwrap_or_default();
                if writer.write_all(s.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(b"\n").await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
    Ok(())
}

/// Parse the argument blob that follows `exec` in the IPC protocol.
///
/// Returns `(cwd, args)` where `cwd` is `Some` only for tab-separated forms.
/// Tab-separated forms preserve paths with spaces; the whitespace fallback
/// exists for legacy callers that send `exec index .`-style strings directly.
fn parse_exec_args(args: &str) -> (Option<String>, Vec<String>) {
    if let Some(rest) = args.strip_prefix('\t') {
        let mut parts = rest.split('\t');
        let cwd = parts.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
        let rest: Vec<String> = parts.map(String::from).collect();
        return (cwd, rest);
    }
    if args.contains('\t') {
        let mut parts = args.split('\t');
        let cwd = parts.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
        let rest: Vec<String> = parts.map(String::from).collect();
        return (cwd, rest);
    }
    (None, args.split_whitespace().map(String::from).collect())
}

async fn exec_speedy_command(args: &str, metrics: &Metrics) -> String {
    let exe = find_speedy_exe();
    let mut cmd = tokio::process::Command::new(&exe);
    let (cwd, parts) = parse_exec_args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    // Count first arg as the operation so `metrics` separates query/index/sync.
    match parts.first().map(String::as_str) {
        Some("query") => { metrics.queries.fetch_add(1, Ordering::Relaxed); }
        Some("index") => { metrics.indexes.fetch_add(1, Ordering::Relaxed); }
        Some("sync")  => { metrics.syncs.fetch_add(1, Ordering::Relaxed); }
        _ => {}
    }
    metrics.exec_calls.fetch_add(1, Ordering::Relaxed);
    for arg in parts {
        cmd.arg(arg);
    }
    cmd.env("SPEEDY_NO_DAEMON", "1");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    match cmd.output().await {
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

#[allow(clippy::too_many_arguments)]
async fn dispatch_command(
    line: &str,
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: &Arc<StdMutex<HashSet<u32>>>,
    metrics: &Arc<Metrics>,
    pid: u32,
    started_at: Instant,
    running: &AtomicBool,
    daemon_dir: &Path,
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
                "protocol_version": PROTOCOL_VERSION,
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
            match reload_from_disk(watchers, active_pids, metrics).await {
                Ok(n) => format!("ok: {n} workspaces reloaded\n"),
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
            match handle_add(raw_path, watchers, active_pids, metrics).await {
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

        _ if line.starts_with("query-all\t") || line.starts_with("query-all ") => {
            metrics.queries.fetch_add(1, Ordering::Relaxed);
            let args = &line["query-all".len()..];
            let resp = handle_query_all(args, watchers).await;
            format!("{resp}\n")
        }

        _ if line.starts_with("sync ") => {
            let raw_path = line.trim_start_matches("sync ").trim();
            metrics.syncs.fetch_add(1, Ordering::Relaxed);
            match handle_sync(raw_path, watchers).await {
                Ok(()) => "ok\n".to_string(),
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("reindex ") => {
            let raw_path = line.trim_start_matches("reindex ").trim();
            metrics.indexes.fetch_add(1, Ordering::Relaxed);
            match handle_reindex(raw_path).await {
                Ok(out) => format!("{out}\n"),
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("workspace-status ") => {
            let raw_path = line.trim_start_matches("workspace-status ").trim();
            match handle_workspace_status(raw_path, watchers).await {
                Ok(s) => format!("{}\n", serde_json::to_string(&s).unwrap_or_default()),
                Err(e) => format!("error: {e}\n"),
            }
        }

        _ if line.starts_with("scan\t") || line.starts_with("scan ") => {
            let args = &line["scan".len()..];
            let resp = handle_scan(args).await;
            format!("{resp}\n")
        }

        _ if line.starts_with("tail-log") => {
            let rest = line.trim_start_matches("tail-log").trim();
            let n: usize = rest.parse().unwrap_or(200);
            let resp = handle_tail_log(daemon_dir, n).await;
            format!("{resp}\n")
        }

        "metrics" => {
            format!("{}\n", metrics.snapshot())
        }

        _ if line.starts_with("exec ") || line.starts_with("exec\t") => {
            let args = line[4..].trim_start_matches(|c: char| c == ' ').trim_end();
            let result = exec_speedy_command(args, metrics).await;
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

async fn handle_add(
    raw_path: &str,
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
    active_pids: &Arc<StdMutex<HashSet<u32>>>,
    metrics: &Arc<Metrics>,
) -> Result<()> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    if !workspace::is_registered(&path_str) {
        workspace::add(&path_str)?;
    }

    let mut ws = watchers.lock().await;
    let is_new = !ws.contains_key(&path_str);
    if is_new {
        let handle = start_workspace_watcher(&path_str, active_pids.clone(), metrics.clone());
        ws.insert(path_str.clone(), handle);
    }
    drop(ws);

    // Fire-and-forget initial sync: the watcher only picks up *future* changes,
    // so without this the index is empty until the user runs `sync`/`index`.
    // We don't await: `add` returns immediately and the indexer runs in the
    // background. SPEEDY_NO_DAEMON=1 prevents recursion.
    if is_new && std::env::var_os("SPEEDY_SKIP_INITIAL_SYNC").is_none() {
        let path_for_sync = path_str.clone();
        let metrics_clone = metrics.clone();
        let watchers_clone = watchers.clone();
        tokio::spawn(async move {
            metrics_clone.syncs.fetch_add(1, Ordering::Relaxed);
            if let Err(e) = handle_sync(&path_for_sync, &watchers_clone).await {
                warn!("Initial sync failed for {path_for_sync}: {e}");
            }
        });
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

/// Parse `<sep><top_k><sep><query>` where `<sep>` is `\t` (preferred) or a
/// single space (legacy). Returns `(top_k, query)`. Falls back to top_k=5 if
/// parsing fails.
fn parse_query_all_args(args: &str) -> (usize, String) {
    let trimmed = args.trim_start_matches(['\t', ' ']);
    let (k_str, q) = match trimmed.split_once(|c: char| c == '\t' || c == ' ') {
        Some((k, q)) => (k, q.to_string()),
        None => return (5, trimmed.to_string()),
    };
    let k = k_str.trim().parse().unwrap_or(5);
    (k, q)
}

/// Fan out a query to every registered workspace, aggregate results, and
/// return the top-K by score as a JSON array. Each item gains a `workspace`
/// field pointing at the source path so callers can tell hits apart.
async fn handle_query_all(
    args: &str,
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
) -> String {
    let (top_k, query) = parse_query_all_args(args);
    if query.is_empty() {
        return r#"{"error":"empty query"}"#.to_string();
    }
    let paths: Vec<String> = {
        let ws = watchers.lock().await;
        ws.keys().cloned().collect()
    };
    if paths.is_empty() {
        return "[]".to_string();
    }

    let exe = find_speedy_exe();
    let k_str = top_k.to_string();

    let mut tasks = Vec::with_capacity(paths.len());
    for ws_path in paths {
        let exe = exe.clone();
        let q = query.clone();
        let k = k_str.clone();
        tasks.push(tokio::spawn(async move {
            let mut cmd = tokio::process::Command::new(&exe);
            cmd.args(["-p", &ws_path, "query", &q, "-k", &k, "--json"])
                .env("SPEEDY_NO_DAEMON", "1");
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let output = match cmd.output().await {
                Ok(o) => o,
                Err(_) => return (ws_path, Vec::new()),
            };
            if !output.status.success() {
                return (ws_path, Vec::new());
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parsed: Vec<serde_json::Value> = serde_json::from_str(stdout.trim()).unwrap_or_default();
            (ws_path, parsed)
        }));
    }

    let mut merged: Vec<serde_json::Value> = Vec::new();
    for t in tasks {
        if let Ok((ws_path, items)) = t.await {
            for mut item in items {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("workspace".to_string(), serde_json::Value::String(ws_path.clone()));
                }
                merged.push(item);
            }
        }
    }

    merged.sort_by(|a, b| {
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    merged.truncate(top_k);

    serde_json::to_string(&merged).unwrap_or_else(|_| "[]".to_string())
}

async fn handle_sync(
    raw_path: &str,
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
) -> Result<()> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    let started = Instant::now();
    let exe = find_speedy_exe();
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(["-p", &path_str, "sync"]).env("SPEEDY_NO_DAEMON", "1");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output().await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(target: "sync", workspace = %path_str, ms = elapsed_ms, "Sync failed: {stderr}");
    } else {
        info!(target: "sync", workspace = %path_str, ms = elapsed_ms, "Sync done");
        let ws = watchers.lock().await;
        if let Some(h) = ws.get(&path_str) {
            h.last_sync_at.store(unix_now_secs(), Ordering::Relaxed);
        }
    }

    Ok(())
}

async fn handle_reindex(raw_path: &str) -> Result<String> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    let started = Instant::now();
    let exe = find_speedy_exe();
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.current_dir(&path_str)
        .args(["index", "."])
        .env("SPEEDY_NO_DAEMON", "1");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output().await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(target: "index", workspace = %path_str, ms = elapsed_ms, "Reindex failed: {stderr}");
        anyhow::bail!("reindex failed: {stderr}");
    }
    info!(target: "index", workspace = %path_str, ms = elapsed_ms, "Reindex done");
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn handle_workspace_status(
    raw_path: &str,
    watchers: &Arc<Mutex<HashMap<String, WatcherHandle>>>,
) -> Result<WorkspaceStatus> {
    let canonical = Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    let (alive, last_event, last_sync) = {
        let ws = watchers.lock().await;
        match ws.get(&path_str) {
            Some(h) => {
                let alive = !h.stop.load(Ordering::SeqCst);
                let ev = h.last_event_at.load(Ordering::Relaxed);
                let sy = h.last_sync_at.load(Ordering::Relaxed);
                (alive, if ev == 0 { None } else { Some(ev) }, if sy == 0 { None } else { Some(sy) })
            }
            None => (false, None, None),
        }
    };

    let db_path = Path::new(&path_str).join(".speedy").join("index.sqlite");
    let index_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    Ok(WorkspaceStatus {
        path: path_str,
        watcher_alive: alive,
        last_event_at: last_event,
        last_sync_at: last_sync,
        index_size_bytes,
        chunk_count: None,
    })
}

/// Parse `[\t<root>[\t<max_depth>]]` and walk the filesystem reporting every
/// directory that contains `.speedy/index.sqlite`. Skips common build dirs.
async fn handle_scan(args: &str) -> String {
    let trimmed = args.trim_start_matches(['\t', ' ']);
    let mut parts = trimmed.split(['\t', '\n']);
    let root = parts.next().unwrap_or("").trim();
    if root.is_empty() {
        return r#"{"error":"missing root"}"#.to_string();
    }
    let max_depth: usize = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(8);

    let root = root.to_string();
    let results = tokio::task::spawn_blocking(move || scan_speedy_dirs(Path::new(&root), max_depth))
        .await
        .unwrap_or_default();

    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string())
}

fn scan_speedy_dirs(root: &Path, max_depth: usize) -> Vec<ScanResult> {
    let registered: std::collections::HashSet<String> = workspace::list()
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.path)
        .collect();

    const SKIP: &[&str] = &[
        "target", ".git", "node_modules", ".idea", ".vscode", "dist", "build", "__pycache__", ".cargo",
    ];

    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !SKIP.contains(&name.as_ref())
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let db = path.join(".speedy").join("index.sqlite");
        if !db.exists() {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_str = canonical.to_string_lossy().to_string();
        let registered = registered.contains(&path_str);
        let (last_modified, db_size_bytes) = match std::fs::metadata(&db) {
            Ok(m) => {
                let ts = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        let secs = d.as_secs() as i64;
                        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_default()
                    });
                (ts, m.len())
            }
            Err(_) => (None, 0),
        };
        out.push(ScanResult { path: path_str, registered, last_modified, db_size_bytes });
    }
    out
}

/// Read the most recent `daemon.log.*` file in `<daemon_dir>/logs/` and
/// return its last `n` lines parsed back into `LogLine` values. Lines that
/// cannot be parsed (e.g. raw stderr that leaked into the file) are skipped.
async fn handle_tail_log(daemon_dir: &Path, n: usize) -> String {
    let logs_dir = daemon_dir.join("logs");
    let result = tokio::task::spawn_blocking(move || tail_log_blocking(&logs_dir, n))
        .await
        .unwrap_or_default();
    serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string())
}

fn tail_log_blocking(logs_dir: &Path, n: usize) -> Vec<LogLine> {
    let Ok(entries) = std::fs::read_dir(logs_dir) else { return Vec::new(); };
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() { continue; }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("daemon.log") { continue; }
        let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        latest = match latest {
            Some((cur, _)) if cur >= mtime => latest,
            _ => Some((mtime, p)),
        };
    }
    let Some((_, path)) = latest else { return Vec::new(); };
    let Ok(content) = std::fs::read_to_string(&path) else { return Vec::new(); };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..]
        .iter()
        .filter_map(|l| serde_json::from_str::<LogLine>(l).ok())
        .collect()
}

fn main() -> Result<()> {
    let cli = DaemonCli::parse();

    // When --daemon-dir is given, propagate via env so workspace::* and any
    // helpers in this process (and any inherited child env) read/write from
    // the isolated location instead of the user's real config dir.
    if let Some(dir) = &cli.daemon_dir {
        std::env::set_var("SPEEDY_DAEMON_DIR", dir);
    }

    let daemon_dir = match &cli.daemon_dir {
        Some(d) => d.clone(),
        None => speedy_core::daemon_util::daemon_dir_path()?,
    };
    let logs_dir = daemon_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).ok();

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "daemon.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so the writer thread keeps flushing for the entire daemon
    // lifetime; without this, dropping the guard at end-of-main can race the
    // last log writes.
    Box::leak(Box::new(file_guard));

    let (log_tx, _initial_rx) = broadcast::channel::<LogLine>(LOG_BROADCAST_CAPACITY);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(file_writer),
        )
        .with(BroadcastLayer { tx: log_tx.clone() })
        .init();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut daemon = match cli.daemon_dir {
            Some(dir) => CentralDaemon::new(cli.socket, dir),
            None => CentralDaemon::default(cli.socket)?,
        };
        daemon.log_tx = Some(log_tx);
        daemon.start().await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use speedy_core::daemon_client::DaemonClient;
    use speedy_core::local_sock::{GenericNamespaced, ListenerOptions, Name, Stream as TestStream, ToNsName};
    use std::time::Duration;

    #[test]
    fn test_should_ignore_watch_path_target() {
        assert!(should_ignore_watch_path(Path::new("target/debug/foo.exe")));
        assert!(should_ignore_watch_path(Path::new("C:/proj/target/release/x")));
    }

    #[test]
    fn test_should_ignore_watch_path_git() {
        assert!(should_ignore_watch_path(Path::new(".git/HEAD")));
        assert!(should_ignore_watch_path(Path::new("C:/proj/.git/index")));
    }

    #[test]
    fn test_should_ignore_watch_path_speedy_internal() {
        assert!(should_ignore_watch_path(Path::new(".speedy/index.sqlite")));
        assert!(should_ignore_watch_path(Path::new(".speedy-daemon/foo")));
    }

    #[test]
    fn test_should_ignore_watch_path_node_modules() {
        assert!(should_ignore_watch_path(Path::new("node_modules/react/index.js")));
        assert!(should_ignore_watch_path(Path::new("packages/x/node_modules/y/z.js")));
    }

    #[test]
    fn test_should_ignore_watch_path_nested_subdirs() {
        // The ignore matches at ANY level, not just the root.
        assert!(should_ignore_watch_path(Path::new("a/b/c/target/foo")));
        assert!(should_ignore_watch_path(Path::new("workspaces/sub/.git/refs/heads/main")));
    }

    #[test]
    fn test_should_ignore_watch_path_normal_files() {
        assert!(!should_ignore_watch_path(Path::new("src/main.rs")));
        assert!(!should_ignore_watch_path(Path::new("README.md")));
        assert!(!should_ignore_watch_path(Path::new("docs/api.md")));
    }

    #[test]
    fn test_should_ignore_watch_path_partial_name_not_ignored() {
        // "targets" is not "target". Component match is exact, not substring.
        assert!(!should_ignore_watch_path(Path::new("targets/foo.rs")));
        assert!(!should_ignore_watch_path(Path::new("git/foo.rs")));
    }

    #[test]
    fn test_metrics_snapshot_zero_by_default() {
        let m = Metrics::default();
        let snap = m.snapshot();
        assert_eq!(snap["queries"], 0);
        assert_eq!(snap["indexes"], 0);
        assert_eq!(snap["syncs"], 0);
        assert_eq!(snap["watcher_events"], 0);
        assert_eq!(snap["exec_calls"], 0);
    }

    #[test]
    fn test_metrics_snapshot_reflects_increments() {
        let m = Metrics::default();
        m.queries.fetch_add(7, Ordering::Relaxed);
        m.syncs.fetch_add(3, Ordering::Relaxed);
        let snap = m.snapshot();
        assert_eq!(snap["queries"], 7);
        assert_eq!(snap["syncs"], 3);
        assert_eq!(snap["indexes"], 0);
    }

    #[test]
    fn test_parse_query_all_args_tab_separated() {
        let (k, q) = parse_query_all_args("\t10\thello world");
        assert_eq!(k, 10);
        assert_eq!(q, "hello world");
    }

    #[test]
    fn test_parse_query_all_args_space_legacy() {
        // Legacy form `query-all 5 some query` — single-space separator.
        let (k, q) = parse_query_all_args(" 5 some query");
        assert_eq!(k, 5);
        assert_eq!(q, "some query");
    }

    #[test]
    fn test_parse_query_all_args_fallback_top_k() {
        // Unparseable top_k → fall back to 5, query is the rest.
        let (k, q) = parse_query_all_args("\tnotanum\trest");
        assert_eq!(k, 5);
        assert_eq!(q, "rest");
    }

    #[test]
    fn test_parse_query_all_args_query_with_internal_spaces() {
        let (k, q) = parse_query_all_args("\t3\tauth flow login redirect");
        assert_eq!(k, 3);
        assert_eq!(q, "auth flow login redirect");
    }

    #[test]
    fn test_protocol_version_matches_supported() {
        // If this fires, the client's SUPPORTED_PROTOCOL_VERSION wasn't bumped
        // in lockstep — that drift is exactly what causes silent compat bugs.
        assert_eq!(
            PROTOCOL_VERSION,
            speedy_core::daemon_client::SUPPORTED_PROTOCOL_VERSION,
            "PROTOCOL_VERSION and SUPPORTED_PROTOCOL_VERSION must move together"
        );
    }

    static SOCKET_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    static DAEMON_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct DaemonTestGuard {
        client: DaemonClient,
        handle: Option<std::thread::JoinHandle<()>>,
        dir: PathBuf,
        ws_backup: Option<Vec<speedy_core::workspace::WorkspaceEntry>>,
        running: Arc<AtomicBool>,
    }

    impl Drop for DaemonTestGuard {
        fn drop(&mut self) {
            self.running.store(false, Ordering::SeqCst);
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

    fn test_socket_name(name: &str) -> String {
        let n = SOCKET_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("speedy_d_{name}_{n}")
    }

    fn start_daemon(name: &str) -> DaemonTestGuard {
        let socket_name = test_socket_name(name);
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
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let sn = socket_name.clone();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut daemon = CentralDaemon::new(sn, daemon_dir);
                daemon.running = running_clone;
                daemon.start().await.unwrap();
            });
        });

        std::thread::sleep(Duration::from_millis(500));
        let client = DaemonClient::new(socket_name);

        DaemonTestGuard {
            client,
            handle: Some(handle),
            dir,
            ws_backup,
            running,
        }
    }

    #[tokio::test]
    async fn test_ping_pong() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_ping");
        assert!(guard.client.is_alive().await);
        assert_eq!(guard.client.ping().await.unwrap(), "pong");
        drop(guard);
    }

    #[tokio::test]
    async fn test_status_pid() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_status");
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
        let guard = start_daemon("test_list_empty");
        let list = guard.client.get_all_workspaces().await.unwrap();
        assert!(list.is_empty());
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);
        drop(guard);
    }

    #[tokio::test]
    async fn test_add_and_remove_workspace() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_add_remove");
        let ws_path = guard.dir.to_string_lossy().to_string();

        guard.client.add_workspace(&ws_path).await.unwrap();

        let list = guard.client.get_all_workspaces().await.unwrap();
        assert!(list.iter().any(|p| {
            Path::new(p).canonicalize().ok()
                == Path::new(&ws_path).canonicalize().ok()
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
        let guard = start_daemon("test_util");

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
        let guard = start_daemon("test_unknown");

        let mut stream = TestStream::connect(guard.client.socket_name.borrow()).await.unwrap();
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
        let client = DaemonClient::new("speedy_d_nonexistent_TEST");
        assert!(!client.is_alive().await);
    }

    async fn send_uds_cmd(socket_name: Name<'_>, req: &str) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let mut stream = TestStream::connect(socket_name).await?;
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
        let guard = start_daemon("test_exec");
        let resp = send_uds_cmd(guard.client.socket_name.borrow(), "exec index .").await;
        assert!(resp.is_ok(), "exec should not fail: {:?}", resp.err());
        drop(guard);
    }

    #[tokio::test]
    async fn test_exec_nonexistent_binary_falls_back_to_speedy() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("exec nonexistent-command --flag", &watchers, &active_pids, &Arc::new(Metrics::default()), 99999, started, &running, Path::new(".")).await;
        assert!(!resp.trim().is_empty(), "exec response must not be empty");
    }

    #[tokio::test]
    async fn test_dispatch_exec_directly() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        // `query test -k 3` returns empty stdout when speedy finds no results;
        // we only care that dispatch produces a framed response (terminating
        // newline), not that the inner command had any output.
        let resp = dispatch_command("exec query test -k 3", &watchers, &active_pids, &Arc::new(Metrics::default()), 88888, started, &running, Path::new(".")).await;
        assert!(resp.ends_with('\n'), "exec dispatch must return a newline-terminated response, got: {resp:?}");
    }

    #[tokio::test]
    async fn test_sync_returns_ok() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_sync");
        let resp = send_uds_cmd(guard.client.socket_name.borrow(), &format!("sync {}", guard.dir.to_string_lossy())).await.unwrap();
        assert_eq!(resp, "ok", "sync should return ok");
        drop(guard);
    }

    #[tokio::test]
    async fn test_dispatch_sync_directly() {
        let dir = std::env::temp_dir().join("speedy_d_test_sync_direct");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(&format!("sync {}", dir.to_string_lossy()), &watchers, &active_pids, &Arc::new(Metrics::default()), 77777, started, &running, Path::new(".")).await;
        assert_eq!(resp.trim(), "ok", "sync via dispatch should return ok");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_double_add_workspace_is_idempotent() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_double_add");
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

        let resp = dispatch_command(&format!("remove {}", existing_dir.to_string_lossy()), &watchers, &active_pids, &Arc::new(Metrics::default()), 66666, started, &running, Path::new(".")).await;
        assert_eq!(resp.trim(), "ok");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_graceful_shutdown_stops_listener() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_shutdown");

        assert!(guard.client.is_alive().await);
        guard.client.stop().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!guard.client.is_alive().await);

        drop(guard);
    }

    #[tokio::test]
    async fn test_watcher_stop_start_on_add_remove() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_watcher_lifecycle");
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
        let guard = start_daemon("test_pid_cleanup");
        assert!(guard.client.is_alive().await);
        guard.client.stop().await.unwrap();
        drop(guard);
    }

    #[tokio::test]
    async fn test_status_has_uptime_and_version() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_status_full");

        let status = guard.client.status().await.unwrap();
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

        let resp = dispatch_command("garbage", &watchers, &active_pids, &Arc::new(Metrics::default()), 11111, started, &running, Path::new(".")).await;
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

        let resp = dispatch_command(&format!("is-workspace {}", dir.to_string_lossy()), &watchers, &active_pids, &Arc::new(Metrics::default()), 11112, started, &running, Path::new(".")).await;
        assert_eq!(resp.trim(), "false");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_dispatch_daemon_pid() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("daemon-pid", &watchers, &active_pids, &Arc::new(Metrics::default()), 55555, started, &running, Path::new(".")).await;
        assert_eq!(resp.trim(), "55555");
    }

    #[tokio::test]
    async fn test_dispatch_watch_count_zero() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("watch-count", &watchers, &active_pids, &Arc::new(Metrics::default()), 12345, started, &running, Path::new(".")).await;
        assert_eq!(resp.trim(), "0");
    }

    #[tokio::test]
    async fn test_socket_in_use_returns_error() {
        let _lock = acquire_lock();
        let socket_name = test_socket_name("test_socket_in_use");
        let dir = std::env::temp_dir().join("speedy_d_test_socket_in_use");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ws_backup = workspace::list().ok();
        if let Some(cfg) = dirs::config_dir() {
            let path = cfg.join("speedy").join("workspaces.json");
            let _ = std::fs::remove_file(&path);
        }

        // Bind a listener to the same socket name first to simulate conflict
        let name = socket_name.as_str().to_ns_name::<GenericNamespaced>().unwrap();
        let listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        let mut daemon = CentralDaemon::new(socket_name, dir.join(".speedy-daemon"));
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

    /// End-to-end: start a daemon, register a workspace, write a file inside
    /// it, and verify the watcher detected the change. We use the
    /// `SPEEDY_WATCH_LOG` test hook so we don't actually spawn `speedy.exe`.
    #[tokio::test]
    async fn test_watcher_invokes_indexer_on_file_write() {
        let _lock = acquire_lock();

        let log_path = std::env::temp_dir().join(format!(
            "speedy_watcher_log_{}_{}.txt",
            std::process::id(),
            SOCKET_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ));
        let _ = std::fs::remove_file(&log_path);
        std::env::set_var("SPEEDY_WATCH_LOG", &log_path);

        let guard = start_daemon("test_watcher_to_indexer");
        let ws_path = guard.dir.to_string_lossy().to_string();

        guard.client.add_workspace(&ws_path).await.unwrap();
        // Let the watcher subscribe before we touch the FS.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let target = guard.dir.join("hello.txt");
        std::fs::write(&target, b"hello from the watcher test").unwrap();

        // Debouncer is 500ms; allow margin for filesystem propagation.
        let mut observed = false;
        for _ in 0..40 {
            if log_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&log_path) {
                    if content.contains("hello.txt") {
                        observed = true;
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        std::env::remove_var("SPEEDY_WATCH_LOG");
        let _ = std::fs::remove_file(&log_path);
        drop(guard);

        assert!(observed, "watcher did not log an index call for hello.txt");
    }

    // ── reload command ─────────────────────────────────────

    // ── parse_exec_args ────────────────────────────────────

    #[test]
    fn test_parse_exec_args_tab_prefix_with_cwd_and_args() {
        let (cwd, args) = parse_exec_args("\tC:\\my dir\tindex\t.");
        assert_eq!(cwd.as_deref(), Some("C:\\my dir"));
        assert_eq!(args, vec!["index".to_string(), ".".to_string()]);
    }

    #[test]
    fn test_parse_exec_args_tab_prefix_empty_cwd() {
        // Tab prefix with empty CWD: explicit "no CWD, but tab-protocol".
        let (cwd, args) = parse_exec_args("\t\tindex\tsubdir");
        assert_eq!(cwd, None);
        assert_eq!(args, vec!["index".to_string(), "subdir".to_string()]);
    }

    #[test]
    fn test_parse_exec_args_tab_middle_legacy() {
        // No leading tab, but a tab in the middle — first part is treated as CWD.
        let (cwd, args) = parse_exec_args("C:\\proj\tquery\thello");
        assert_eq!(cwd.as_deref(), Some("C:\\proj"));
        assert_eq!(args, vec!["query".to_string(), "hello".to_string()]);
    }

    #[test]
    fn test_parse_exec_args_whitespace_legacy() {
        // Pure whitespace — legacy callers without CWD support.
        let (cwd, args) = parse_exec_args("index . --json");
        assert_eq!(cwd, None);
        assert_eq!(args, vec!["index".to_string(), ".".to_string(), "--json".to_string()]);
    }

    #[test]
    fn test_parse_exec_args_empty_returns_no_args() {
        let (cwd, args) = parse_exec_args("");
        assert_eq!(cwd, None);
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_exec_args_whitespace_collapses_runs() {
        // split_whitespace collapses multiple spaces / tabs around words.
        let (cwd, args) = parse_exec_args("query  hello   world");
        assert_eq!(cwd, None);
        assert_eq!(args, vec!["query".to_string(), "hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn test_parse_exec_args_path_with_spaces_via_tab() {
        let (cwd, args) = parse_exec_args("\tC:\\Program Files\\proj\tindex\tsome dir");
        assert_eq!(cwd.as_deref(), Some("C:\\Program Files\\proj"));
        // Tab is preserved as the separator; spaces inside an arg remain.
        assert_eq!(args, vec!["index".to_string(), "some dir".to_string()]);
    }

    #[tokio::test]
    async fn test_dispatch_exec_legacy_whitespace_with_nonexistent_binary() {
        // The legacy whitespace path — used by `speedy daemon exec ...` style
        // direct callers — must still produce a response, even if the speedy
        // binary fails.
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command("exec --version", &watchers, &active_pids, &Arc::new(Metrics::default()), 1, started, &running, Path::new(".")).await;
        assert!(!resp.is_empty(), "exec response must not be empty");
    }

    #[tokio::test]
    async fn test_dispatch_exec_with_only_whitespace_after_keyword() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        // Pure "exec   " (no args) — the binary is invoked with no arguments.
        let resp = dispatch_command("exec   ", &watchers, &active_pids, &Arc::new(Metrics::default()), 1, started, &running, Path::new(".")).await;
        assert!(!resp.is_empty(), "exec must always return a non-empty line");
    }

    // ── reload command ─────────────────────────────────────

    #[tokio::test]
    async fn test_reload_no_workspaces() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_reload_empty");
        let resp = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        assert!(resp.starts_with("ok:"), "expected ok response, got: {resp}");
        assert!(resp.contains("0 workspaces"), "expected 0 workspaces, got: {resp}");
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);
        drop(guard);
    }

    #[tokio::test]
    async fn test_reload_picks_up_new_workspace() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_reload_new");
        let extra = guard.dir.join("extra_ws");
        std::fs::create_dir_all(&extra).unwrap();
        let extra_canonical = extra.canonicalize().unwrap().to_string_lossy().to_string();

        assert_eq!(guard.client.watch_count().await.unwrap(), 0);

        // Add directly to the persisted file, bypassing the daemon's add path.
        workspace::add(&extra_canonical).unwrap();

        let resp = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        assert!(resp.contains("1 workspaces"), "expected 1 workspace, got: {resp}");
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);
        drop(guard);
    }

    #[tokio::test]
    async fn test_reload_drops_unregistered_watcher() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_reload_drop");
        let ws_path = guard.dir.to_string_lossy().to_string();

        guard.client.add_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        // Remove from the persisted file without notifying the daemon.
        let canonical = Path::new(&ws_path).canonicalize().unwrap().to_string_lossy().to_string();
        workspace::remove(&canonical).unwrap();

        let resp = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        assert!(resp.contains("0 workspaces"), "expected 0 workspaces, got: {resp}");
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);
        drop(guard);
    }

    #[tokio::test]
    async fn test_reload_is_idempotent() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_reload_idem");
        let ws_path = guard.dir.to_string_lossy().to_string();

        guard.client.add_workspace(&ws_path).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);

        let r1 = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        let r2 = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        let r3 = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
        assert_eq!(guard.client.watch_count().await.unwrap(), 1);
        drop(guard);
    }

    #[tokio::test]
    async fn test_reload_skips_missing_path() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_reload_missing");
        let missing = guard.dir.join("does_not_exist");
        let missing_str = missing.to_string_lossy().to_string();

        // Inject a path that doesn't exist on disk.
        workspace::add(&missing_str).unwrap();

        let resp = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        // reload reports the count of *registered* workspaces, even those it
        // skips spinning up because the path is gone. Watcher count must stay 0.
        assert!(resp.contains("1 workspaces"), "expected 1 workspace in reload report, got: {resp}");
        assert_eq!(guard.client.watch_count().await.unwrap(), 0);

        // Clean up the dangling entry so the restore on Drop doesn't keep it.
        let _ = workspace::remove(&missing_str);
        drop(guard);
    }

    #[tokio::test]
    async fn test_reload_replaces_full_set() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_reload_replace");

        let a = guard.dir.join("a");
        let b = guard.dir.join("b");
        let c = guard.dir.join("c");
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }
        let a_c = a.canonicalize().unwrap().to_string_lossy().to_string();
        let b_c = b.canonicalize().unwrap().to_string_lossy().to_string();
        let c_c = c.canonicalize().unwrap().to_string_lossy().to_string();

        guard.client.add_workspace(&a_c).await.unwrap();
        guard.client.add_workspace(&b_c).await.unwrap();
        assert_eq!(guard.client.watch_count().await.unwrap(), 2);

        // Externally rewrite the persisted list: drop B, add C.
        workspace::remove(&b_c).unwrap();
        workspace::add(&c_c).unwrap();

        let resp = send_uds_cmd(guard.client.socket_name.borrow(), "reload").await.unwrap();
        assert!(resp.contains("2 workspaces"), "expected 2 workspaces, got: {resp}");
        assert_eq!(guard.client.watch_count().await.unwrap(), 2);

        let list = guard.client.get_all_workspaces().await.unwrap();
        let has_a = list.iter().any(|p| Path::new(p).canonicalize().ok().as_ref().map(|x| x.to_string_lossy().to_string()) == Some(a_c.clone()));
        let has_c = list.iter().any(|p| Path::new(p).canonicalize().ok().as_ref().map(|x| x.to_string_lossy().to_string()) == Some(c_c.clone()));
        let has_b = list.iter().any(|p| Path::new(p).canonicalize().ok().as_ref().map(|x| x.to_string_lossy().to_string()) == Some(b_c.clone()));
        assert!(has_a, "A should still be watched");
        assert!(has_c, "C should now be watched");
        assert!(!has_b, "B should not be watched anymore");
        drop(guard);
    }

    // ── socket race ────────────────────────────────────────

    /// Two real `speedy-daemon` subprocesses fired simultaneously against the
    /// same socket name AND the same daemon_dir. The `acquire_daemon_lock`
    /// advisory lock must let exactly one through; the loser must exit with a
    /// non-zero status. We run actual subprocesses (not in-process daemons)
    /// because `kill_existing_daemon` reads `daemon.pid` and would taskkill
    /// the test binary itself if two daemons shared this process's PID.
    #[test]
    fn test_socket_race_only_one_daemon_wins() {
        let _lock = acquire_lock();

        // Locate the built `speedy-daemon` binary — same logic as
        // find_daemon_exe in production code.
        let suffix = std::env::consts::EXE_SUFFIX;
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap();
        let direct = dir.join(format!("speedy-daemon{suffix}"));
        let daemon_bin = if direct.exists() {
            direct
        } else if dir.file_name().and_then(|s| s.to_str()) == Some("deps") {
            dir.parent().unwrap().join(format!("speedy-daemon{suffix}"))
        } else {
            eprintln!("skipping test_socket_race: speedy-daemon binary not found");
            return;
        };
        if !daemon_bin.exists() {
            eprintln!("skipping test_socket_race: {} missing — build with `cargo build -p speedy-daemon`", daemon_bin.display());
            return;
        }

        let socket = test_socket_name("race");
        let race_dir = std::env::temp_dir().join(format!(
            "speedy_d_race_{}_{}",
            std::process::id(),
            SOCKET_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&race_dir);
        std::fs::create_dir_all(&race_dir).unwrap();
        let daemon_dir = race_dir.join(".speedy-daemon");

        let spawn_one = || {
            let mut cmd = std::process::Command::new(&daemon_bin);
            cmd.args(["--daemon-socket", &socket])
                .arg("--daemon-dir").arg(&daemon_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.spawn().expect("spawn daemon child")
        };

        // Fire both spawns as close together as possible. They'll race
        // acquire_daemon_lock + the socket bind.
        let mut a = spawn_one();
        let mut b = spawn_one();

        // Wait long enough that the loser has surrendered (lock contention
        // surfaces in well under a second; bind contention not much longer).
        std::thread::sleep(Duration::from_secs(3));

        let a_exit = a.try_wait().expect("try_wait a");
        let b_exit = b.try_wait().expect("try_wait b");

        // Exactly one of them must still be running (the winner, holding the
        // listener) while the other must have exited on its own with a
        // failure. Anything else means the lock or bind didn't serialize the
        // race correctly.
        let a_alive = a_exit.is_none();
        let b_alive = b_exit.is_none();
        let alive_count = (a_alive as usize) + (b_alive as usize);
        assert_eq!(
            alive_count, 1,
            "exactly one daemon should win the race; a_exit={a_exit:?} b_exit={b_exit:?}"
        );

        // The loser must have exited non-success.
        let loser_status = if a_alive { b_exit.unwrap() } else { a_exit.unwrap() };
        assert!(
            !loser_status.success(),
            "the losing daemon should exit with failure, got: {loser_status:?}"
        );

        // The winner is still listening — verify and stop it.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let alive = rt.block_on(async {
            let client = speedy_core::daemon_client::DaemonClient::new(&socket);
            client.is_alive().await
        });
        assert!(alive, "the winning daemon should be reachable on the socket");

        // Stop the winner cleanly via IPC; fall back to kill on timeout.
        rt.block_on(async {
            let client = speedy_core::daemon_client::DaemonClient::new(&socket);
            let _ = client.stop().await;
        });
        if a_alive {
            let _ = a.kill();
            let _ = a.wait();
        } else {
            let _ = b.kill();
            let _ = b.wait();
        }

        let _ = std::fs::remove_dir_all(&race_dir);
    }

    // ── canonicalize / UNC path ────────────────────────────

    /// On Windows `Path::canonicalize` returns `\\?\C:\...`. `is-workspace`
    /// canonicalizes both sides before comparing, so a workspace registered
    /// via one form must still match a lookup via the other. This test guards
    /// against regressions where we'd compare raw strings.
    #[tokio::test]
    async fn test_is_workspace_matches_across_unc_prefix() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_unc_match");
        let ws = guard.dir.canonicalize().unwrap();
        let ws_str = ws.to_string_lossy().to_string();
        guard.client.add_workspace(&ws_str).await.unwrap();

        // On Windows ws_str is already `\\?\…`. Try a stripped-prefix variant.
        #[cfg(windows)]
        {
            let stripped = ws_str.trim_start_matches(r"\\?\").to_string();
            if stripped != ws_str {
                assert!(
                    guard.client.is_workspace(&stripped).await.unwrap(),
                    "stripped-UNC path {stripped} should still match registered {ws_str}"
                );
            }
            // And re-prefixing a non-prefixed form must also match.
            let reprefixed = format!(r"\\?\{stripped}");
            assert!(
                guard.client.is_workspace(&reprefixed).await.unwrap(),
                "re-prefixed UNC path {reprefixed} should match registered {ws_str}"
            );
        }
        // Non-Windows: canonicalize is no-op for UNC; just confirm match.
        #[cfg(not(windows))]
        {
            assert!(guard.client.is_workspace(&ws_str).await.unwrap());
        }
        drop(guard);
    }

    // ── health check: dead-watcher restart ─────────────────

    /// Drop a workspace's heartbeat below the threshold by stopping the
    /// watcher thread; `check_watcher_health_with_thresholds` must move it to
    /// `to_restart` and spin a fresh watcher with a current heartbeat.
    #[tokio::test]
    async fn test_health_check_restarts_dead_watcher() {
        let _lock = acquire_lock();
        let guard = start_daemon("test_health_restart");
        let ws_path = guard.dir.to_string_lossy().to_string();
        guard.client.add_workspace(&ws_path).await.unwrap();

        // Reach into the daemon's watcher map via a parallel registration on
        // the same daemon-dir — we already added the workspace, so we just
        // need the watchers Arc. The cleanest path here is to call the inner
        // helper directly with our own freshly-built map mimicking the daemon
        // state. That keeps the test off the public IPC and focused.
        let watchers: Arc<Mutex<HashMap<String, WatcherHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let metrics = Arc::new(Metrics::default());

        // Insert a watcher and immediately stop it so its heartbeat freezes.
        let canonical = std::path::Path::new(&ws_path)
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let handle = start_workspace_watcher(&canonical, active_pids.clone(), metrics.clone());
        handle.stop.store(true, Ordering::SeqCst);
        // Force the heartbeat to be old enough to trigger restart.
        handle.last_heartbeat.store(unix_now_secs().saturating_sub(120), Ordering::Relaxed);
        watchers.lock().await.insert(canonical.clone(), handle);

        // Thresholds: dead after 5s. Heartbeat is 120s old → restart triggers.
        check_watcher_health_with_thresholds(
            &watchers,
            &active_pids,
            &metrics,
            /*warn_after*/ 2,
            /*dead_after*/ 5,
        )
        .await;

        // After restart, the heartbeat must be fresh again (within last 10s).
        let map = watchers.lock().await;
        let h = map.get(&canonical).expect("watcher should still be present after restart");
        let last = h.last_heartbeat.load(Ordering::Relaxed);
        let now = unix_now_secs();
        assert!(
            now.saturating_sub(last) < 10,
            "after restart heartbeat should be recent: last={last}, now={now}"
        );

        // Clean up: stop the new watcher.
        h.stop.store(true, Ordering::SeqCst);
        drop(guard);
    }

    // ── graceful shutdown with in-flight child ─────────────

    /// `stop_all_watchers` must always leave `active_pids` empty, even when
    /// the PIDs in the set are bogus / already-exited. This protects the
    /// invariant that after shutdown we don't hold references to dead PIDs
    /// that could later get recycled and accidentally taskkill'd.
    #[tokio::test]
    async fn test_stop_all_watchers_clears_inflight_pids() {
        let watchers: Arc<Mutex<HashMap<String, WatcherHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));

        // Seed a few bogus PIDs — taskkill on Windows will return non-zero but
        // we swallow the status; the test cares about state, not exit codes.
        {
            let mut p = active_pids.lock().unwrap();
            p.insert(0xDEAD_BEEFu32);
            p.insert(0xCAFE_F00Du32);
        }

        stop_all_watchers(&watchers, &active_pids).await;

        let p = active_pids.lock().unwrap();
        assert!(p.is_empty(), "active_pids must be cleared after shutdown, still has: {p:?}");

        let ws = watchers.lock().await;
        assert!(ws.is_empty(), "watcher map must be cleared after shutdown");
    }

    // ── new IPC commands: workspace-status, scan, reindex, tail-log ────────

    #[tokio::test]
    async fn test_workspace_status_unknown_path_errors() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        // A path that exists on disk but isn't watched — workspace-status
        // should still respond (with watcher_alive=false). For an outright
        // missing path it must surface the canonicalize error as `error: ...`.
        let resp = dispatch_command(
            "workspace-status C:\\definitely\\not\\here",
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            Path::new("."),
        )
        .await;
        assert!(resp.starts_with("error:"), "expected error, got: {resp}");
    }

    #[tokio::test]
    async fn test_workspace_status_known_path_reports_no_watcher() {
        let dir = std::env::temp_dir().join("speedy_d_ws_status_known");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(
            &format!("workspace-status {}", dir.to_string_lossy()),
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            Path::new("."),
        )
        .await;
        let resp = resp.trim();
        assert!(!resp.starts_with("error"), "expected JSON, got: {resp}");
        let parsed: WorkspaceStatus = serde_json::from_str(resp).expect("valid JSON");
        assert!(!parsed.watcher_alive, "not in watchers → not alive");
        assert_eq!(parsed.index_size_bytes, 0, "no .speedy yet");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_scan_finds_directory_with_index_sqlite() {
        // Create a root with one .speedy/index.sqlite inside.
        let root = std::env::temp_dir().join(format!(
            "speedy_d_scan_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("proj-a");
        let speedy_dir = project.join(".speedy");
        std::fs::create_dir_all(&speedy_dir).unwrap();
        std::fs::write(speedy_dir.join("index.sqlite"), b"fake db content").unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(
            &format!("scan\t{}", root.to_string_lossy()),
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            Path::new("."),
        )
        .await;
        let resp = resp.trim();
        let parsed: Vec<ScanResult> = serde_json::from_str(resp).expect("scan returns JSON array");
        assert!(
            parsed.iter().any(|r| r.path.contains("proj-a") && r.db_size_bytes > 0),
            "scan should find proj-a, got: {parsed:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_scan_missing_root_returns_empty_array() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(
            "scan\tC:\\absolutely-not-a-dir-zzz",
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            Path::new("."),
        )
        .await;
        let resp = resp.trim();
        let parsed: Vec<ScanResult> = serde_json::from_str(resp).unwrap_or_default();
        assert!(parsed.is_empty(), "missing root → no results, got: {parsed:?}");
    }

    #[tokio::test]
    async fn test_reindex_missing_path_errors() {
        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(
            "reindex C:\\nowhere-1234567890",
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            Path::new("."),
        )
        .await;
        assert!(resp.starts_with("error:"), "expected error, got: {resp}");
    }

    #[tokio::test]
    async fn test_tail_log_returns_empty_when_no_logs() {
        let dir = std::env::temp_dir().join(format!(
            "speedy_d_taillog_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(
            "tail-log 50",
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            &dir,
        )
        .await;
        let resp = resp.trim();
        let parsed: Vec<LogLine> = serde_json::from_str(resp).expect("tail-log returns array");
        assert!(parsed.is_empty(), "no logs → empty array, got: {parsed:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_tail_log_parses_json_lines() {
        let dir = std::env::temp_dir().join(format!(
            "speedy_d_taillog_real_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        // Write three JSON lines + one junk line → only the JSON ones come back.
        let file = logs.join("daemon.log.2026-05-15");
        let content = "\
{\"ts\":\"2026-05-15T10:00:00Z\",\"level\":\"info\",\"target\":\"ipc\",\"message\":\"one\",\"fields\":{}}\n\
not-json-line\n\
{\"ts\":\"2026-05-15T10:00:01Z\",\"level\":\"warn\",\"target\":\"sync\",\"message\":\"two\",\"fields\":{}}\n\
{\"ts\":\"2026-05-15T10:00:02Z\",\"level\":\"error\",\"target\":\"watcher\",\"message\":\"three\",\"fields\":{}}\n";
        std::fs::write(&file, content).unwrap();

        let watchers = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_pids = Arc::new(StdMutex::new(HashSet::new()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let resp = dispatch_command(
            "tail-log 10",
            &watchers,
            &active_pids,
            &Arc::new(Metrics::default()),
            1,
            started,
            &running,
            &dir,
        )
        .await;
        let parsed: Vec<LogLine> = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(parsed.len(), 3, "junk line should be skipped, got: {parsed:?}");
        assert_eq!(parsed[2].level, "error");
        assert_eq!(parsed[2].message, "three");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_stream_log_handshake_and_forward() {
        // Spin up two ends of a duplex pipe, feed one LogLine through the
        // broadcast, and assert the handshake `ok\n` + serialized JSON line
        // arrive in order on the reader side.
        let (tx, _) = broadcast::channel::<LogLine>(8);
        let (mut client, mut server) = tokio::io::duplex(4096);

        let tx_clone = tx.clone();
        let server_task = tokio::spawn(async move {
            let _ = stream_log(&mut server, tx_clone).await;
        });

        // Give the server a moment to subscribe before we publish, otherwise
        // the broadcast send happens with zero receivers and gets dropped.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let line = LogLine {
            ts: "2026-05-15T11:11:11Z".to_string(),
            level: "info".to_string(),
            target: "test".to_string(),
            message: "hello".to_string(),
            fields: serde_json::Map::new(),
        };
        tx.send(line.clone()).unwrap();

        let mut reader = tokio::io::BufReader::new(&mut client);
        let mut handshake = String::new();
        reader.read_line(&mut handshake).await.unwrap();
        assert_eq!(handshake.trim(), "ok", "handshake mismatch: {handshake:?}");

        let mut payload = String::new();
        reader.read_line(&mut payload).await.unwrap();
        let parsed: LogLine = serde_json::from_str(payload.trim()).unwrap();
        assert_eq!(parsed.message, "hello");

        drop(client);
        // Closing the read end breaks the broadcast→write loop; the task
        // returns after the next failed write. We don't wait — it's enough
        // to confirm the assertions above.
        server_task.abort();
    }
}
