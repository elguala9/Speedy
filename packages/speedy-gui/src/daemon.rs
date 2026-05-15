//! Sync ⇄ async bridge between the egui main loop (immediate-mode, sync) and
//! `speedy-core::DaemonClient` (Tokio-based, async).
//!
//! A single background Tokio runtime owns the IPC. UI code calls
//! `bridge.refresh_*()`; the background task fills in `state` and the UI reads
//! it next frame. No blocking on the UI thread.

use anyhow::Result;
use speedy_core::daemon_client::DaemonClient;
use speedy_core::types::{DaemonStatus, Metrics, ScanResult, WorkspaceStatus};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default, Clone)]
pub struct DaemonState {
    pub alive: bool,
    pub probed: bool,
    pub status: Option<DaemonStatus>,
    pub metrics: Option<Metrics>,
    pub workspaces: Vec<String>,
    pub workspace_status: std::collections::HashMap<String, WorkspaceStatus>,
    pub scan_results: Vec<ScanResult>,
    pub last_error: Option<String>,
    pub last_refresh: Option<Instant>,
    /// True while at least one IPC call is in flight (debounces spinner UI).
    pub busy: u32,
    /// Free-form transient message ("Workspace added", "Sync done", ...).
    pub toast: Option<(String, Instant, bool)>,
}

impl DaemonState {
    pub fn set_toast(&mut self, msg: impl Into<String>, ok: bool) {
        self.toast = Some((msg.into(), Instant::now(), ok));
    }
}

pub struct DaemonBridge {
    rt: tokio::runtime::Runtime,
    client: Arc<DaemonClient>,
    pub socket_name: String,
    pub state: Arc<Mutex<DaemonState>>,
}

impl DaemonBridge {
    pub fn new(socket_name: String) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()?;
        let client = Arc::new(DaemonClient::new(&socket_name));
        Ok(Self {
            rt,
            client,
            socket_name,
            state: Arc::new(Mutex::new(DaemonState::default())),
        })
    }

    fn inc_busy(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.busy = s.busy.saturating_add(1);
        }
    }

    /// Re-check that the daemon is reachable and refresh `status`, `metrics`,
    /// and the workspace list. Called periodically from `App::update`.
    pub fn refresh_all(&self) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let alive = client.is_alive().await;
            let mut snapshot = DaemonState::default();
            snapshot.alive = alive;
            snapshot.probed = true;
            snapshot.last_refresh = Some(Instant::now());

            if alive {
                match client.status().await {
                    Ok(s) => snapshot.status = Some(s),
                    Err(e) => snapshot.last_error = Some(format!("status: {e}")),
                }
                match client.metrics().await {
                    Ok(v) => {
                        snapshot.metrics = serde_json::from_value(v).ok();
                    }
                    Err(e) => snapshot.last_error = Some(format!("metrics: {e}")),
                }
                match client.get_all_workspaces().await {
                    Ok(list) => snapshot.workspaces = list,
                    Err(e) => snapshot.last_error = Some(format!("list: {e}")),
                }
            }

            if let Ok(mut s) = state.lock() {
                let preserved_toast = s.toast.take();
                let preserved_ws_status = std::mem::take(&mut s.workspace_status);
                let preserved_scan = std::mem::take(&mut s.scan_results);
                let prev_busy = s.busy;
                *s = snapshot;
                s.toast = preserved_toast;
                s.workspace_status = preserved_ws_status;
                s.scan_results = preserved_scan;
                s.busy = prev_busy.saturating_sub(1);
            }
        });
    }

    pub fn refresh_workspace_status(&self, path: String) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.workspace_status(&path).await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(ws) => {
                        s.workspace_status.insert(path, ws);
                    }
                    Err(e) => {
                        s.last_error = Some(format!("workspace-status {path}: {e}"));
                    }
                }
            }
        });
    }

    pub fn add_workspace(&self, path: String) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.add_workspace(&path).await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(()) => s.set_toast(format!("Added: {path}"), true),
                    Err(e) => {
                        s.last_error = Some(format!("add {path}: {e}"));
                        s.set_toast(format!("Add failed: {e}"), false);
                    }
                }
            }
        });
        self.refresh_all();
    }

    pub fn remove_workspace(&self, path: String) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.remove_workspace(&path).await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(()) => s.set_toast(format!("Removed: {path}"), true),
                    Err(e) => {
                        s.last_error = Some(format!("remove {path}: {e}"));
                        s.set_toast(format!("Remove failed: {e}"), false);
                    }
                }
            }
        });
        self.refresh_all();
    }

    pub fn sync_workspace(&self, path: String) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.sync(&path).await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(()) => s.set_toast(format!("Synced: {path}"), true),
                    Err(e) => {
                        s.last_error = Some(format!("sync {path}: {e}"));
                        s.set_toast(format!("Sync failed: {e}"), false);
                    }
                }
            }
        });
    }

    pub fn reindex_workspace(&self, path: String) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.reindex(&path).await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(_) => s.set_toast(format!("Reindex done: {path}"), true),
                    Err(e) => {
                        s.last_error = Some(format!("reindex {path}: {e}"));
                        s.set_toast(format!("Reindex failed: {e}"), false);
                    }
                }
            }
        });
    }

    pub fn scan(&self, root: String, max_depth: usize) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.scan(&root, Some(max_depth)).await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(list) => {
                        let n = list.len();
                        s.scan_results = list;
                        s.set_toast(format!("Scan: {n} hits"), true);
                    }
                    Err(e) => {
                        s.last_error = Some(format!("scan {root}: {e}"));
                        s.set_toast(format!("Scan failed: {e}"), false);
                    }
                }
            }
        });
    }

    pub fn reload(&self) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.reload().await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(msg) => s.set_toast(msg, true),
                    Err(e) => s.set_toast(format!("Reload failed: {e}"), false),
                }
            }
        });
        self.refresh_all();
    }

    pub fn stop_daemon(&self) {
        let client = self.client.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let r = client.stop().await;
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match r {
                    Ok(()) => s.set_toast("Daemon stopped", true),
                    Err(e) => s.set_toast(format!("Stop failed: {e}"), false),
                }
            }
        });
    }

    /// Spawn the daemon binary in a detached child process. Used by the
    /// "Start daemon" / "Restart" UI paths. Note: this is a *sync* fs/process
    /// op so we don't go through the runtime.
    pub fn spawn_daemon(&self) -> Result<()> {
        speedy_core::daemon_util::spawn_daemon_process(&self.socket_name)
    }

    /// Restart sequence: stop, wait for the pipe to die, then spawn. Runs on
    /// the runtime so the UI stays responsive.
    pub fn restart_daemon(&self) {
        let client = self.client.clone();
        let socket_name = self.socket_name.clone();
        let state = self.state.clone();
        self.inc_busy();
        self.rt.spawn(async move {
            let _ = client.stop().await;
            for _ in 0..50 {
                if !client.is_alive().await {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            let spawn_result = tokio::task::spawn_blocking(move || {
                speedy_core::daemon_util::spawn_daemon_process(&socket_name)
            })
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("join error: {e}")));
            if let Ok(mut s) = state.lock() {
                s.busy = s.busy.saturating_sub(1);
                match spawn_result {
                    Ok(()) => s.set_toast("Daemon restarted", true),
                    Err(e) => s.set_toast(format!("Restart failed: {e}"), false),
                }
            }
        });
    }

    /// Borrow the underlying tokio runtime — used by `log_stream` to spawn
    /// the long-lived subscription task on the same executor.
    pub fn runtime(&self) -> &tokio::runtime::Runtime {
        &self.rt
    }

    pub fn client(&self) -> Arc<DaemonClient> {
        self.client.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speedy_core::local_sock::{
        GenericNamespaced, Listener, ListenerOptions, ListenerTrait as _,
        StreamTrait as _, ToNsName,
    };
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    static COUNTER: AtomicU64 = AtomicU64::new(1);

    fn unique_socket(label: &str) -> String {
        let n = COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        format!("speedy_gui_test_{label}_{}_{n}", std::process::id())
    }

    fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut f: F) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn spawn_mock(rt: &tokio::runtime::Runtime, socket: String) {
        let name = socket
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
            .into_owned();
        let listener: Listener = rt.block_on(async {
            ListenerOptions::new()
                .name(name)
                .create_tokio()
                .expect("mock listener bind")
        });
        rt.spawn(async move {
            loop {
                let Ok(stream) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.split();
                    let mut br = BufReader::new(reader);
                    let mut line = String::new();
                    if br.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let resp = match line.trim() {
                        "ping" => "pong",
                        "status" => {
                            r#"{"pid":42,"uptime_secs":1,"workspace_count":0,"watcher_count":0,"version":"x","protocol_version":2}"#
                        }
                        "metrics" => {
                            r#"{"queries":0,"indexes":0,"syncs":0,"watcher_events":0,"exec_calls":0}"#
                        }
                        "list" => "[]",
                        _ => "ok",
                    };
                    let _ = writer.write_all(format!("{resp}\n").as_bytes()).await;
                    let _ = writer.shutdown().await;
                });
            }
        });
    }

    #[test]
    fn daemon_state_toast_helper_round_trips() {
        let mut s = DaemonState::default();
        s.set_toast("hello", true);
        let (msg, _, ok) = s.toast.clone().expect("toast set");
        assert_eq!(msg, "hello");
        assert!(ok);
    }

    #[test]
    fn bridge_against_dead_socket_marks_probed_not_alive() {
        let socket = unique_socket("dead");
        let bridge = DaemonBridge::new(socket).unwrap();
        bridge.refresh_all();
        let settled = wait_until(Duration::from_secs(8), || {
            let s = bridge.state.lock().unwrap();
            s.busy == 0 && s.probed
        });
        assert!(settled, "refresh_all never settled against a dead socket");
        let s = bridge.state.lock().unwrap();
        assert!(!s.alive);
        assert!(s.probed);
        assert!(s.status.is_none());
    }

    #[test]
    fn bridge_against_mock_marks_alive_and_loads_status_and_metrics() {
        // Mock listener runtime is intentionally separate from the bridge's.
        let mock_rt = tokio::runtime::Runtime::new().unwrap();
        let socket = unique_socket("live");
        spawn_mock(&mock_rt, socket.clone());
        std::thread::sleep(Duration::from_millis(150));

        let bridge = DaemonBridge::new(socket).unwrap();
        bridge.refresh_all();
        let settled = wait_until(Duration::from_secs(8), || {
            let s = bridge.state.lock().unwrap();
            s.busy == 0 && s.alive && s.status.is_some() && s.metrics.is_some()
        });
        assert!(settled, "bridge never saw mock alive");
        let s = bridge.state.lock().unwrap();
        assert_eq!(s.status.as_ref().unwrap().pid, 42);
        assert_eq!(s.status.as_ref().unwrap().protocol_version, 2);
        assert!(s.workspaces.is_empty());
        // Drop the mock runtime so the OS releases the socket.
        drop(mock_rt);
    }

    #[test]
    fn busy_counter_settles_after_multiple_overlapping_calls() {
        let socket = unique_socket("busy");
        let bridge = DaemonBridge::new(socket).unwrap();
        for _ in 0..5 {
            bridge.refresh_all();
        }
        let settled = wait_until(Duration::from_secs(10), || {
            let s = bridge.state.lock().unwrap();
            s.busy == 0
        });
        assert!(settled, "busy counter never hit 0");
    }

    #[test]
    fn workspace_status_error_on_dead_socket_surfaces_in_last_error() {
        let socket = unique_socket("ws_status_dead");
        let bridge = DaemonBridge::new(socket).unwrap();
        let dir = std::env::temp_dir().join("speedy_gui_ws_status_test");
        std::fs::create_dir_all(&dir).unwrap();
        bridge.refresh_workspace_status(dir.to_string_lossy().into());
        let settled = wait_until(Duration::from_secs(5), || {
            let s = bridge.state.lock().unwrap();
            s.busy == 0
        });
        assert!(settled);
        let s = bridge.state.lock().unwrap();
        // workspace_status against a dead socket fails → recorded as last_error.
        assert!(s.last_error.as_deref().unwrap_or("").contains("workspace-status"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

