//! Shared serde types for daemon ⇄ client communication.
//!
//! Kept in `speedy-core` so the daemon, the CLI clients, and the GUI all
//! deserialize the same shapes without duplicating the field names.

use serde::{Deserialize, Serialize};

/// One snapshot of the daemon process. Returned by `status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub pid: u32,
    pub uptime_secs: u64,
    pub workspace_count: usize,
    pub watcher_count: usize,
    pub version: String,
    #[serde(default)]
    pub protocol_version: u32,
}

/// Cumulative counters since daemon start. Returned by `metrics`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub queries: u64,
    pub indexes: u64,
    pub syncs: u64,
    pub watcher_events: u64,
    pub exec_calls: u64,
}

/// Per-workspace runtime info. Returned by `workspace-status <path>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceStatus {
    pub path: String,
    pub watcher_alive: bool,
    /// Unix seconds. `None` if never observed.
    pub last_event_at: Option<u64>,
    /// Unix seconds when the last `sync` finished. `None` if never run.
    pub last_sync_at: Option<u64>,
    /// Size of `.speedy/index.sqlite` in bytes. 0 if the file does not exist.
    pub index_size_bytes: u64,
    /// Number of chunk rows. None if unknown (e.g. DB not openable from the
    /// daemon without spawning speedy.exe — kept optional for forward-compat).
    pub chunk_count: Option<u64>,
}

/// One entry from `scan <root>`: a directory that contains a `.speedy/`
/// subdirectory with an index database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub path: String,
    /// `true` if `path` appears in `workspaces.json`.
    pub registered: bool,
    /// RFC3339 timestamp of the index DB's last modification, or `None` if
    /// the OS did not report it.
    pub last_modified: Option<String>,
    /// Size in bytes of `.speedy/index.sqlite`. 0 if the file does not exist.
    pub db_size_bytes: u64,
}

/// One structured log event from the daemon. Sent over the wire by
/// `subscribe-log` (one JSON line per event) and stored on disk in the
/// rolling JSON log file with the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    /// RFC3339 timestamp.
    pub ts: String,
    /// `trace` | `debug` | `info` | `warn` | `error`.
    pub level: String,
    /// `tracing` event target (module path by default).
    pub target: String,
    /// The free-form `message` field.
    pub message: String,
    /// Extra structured fields recorded on the event.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub fields: serde_json::Map<String, serde_json::Value>,
}
