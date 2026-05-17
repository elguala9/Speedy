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
    /// Size of `.speedy/sac.sqlite` in bytes. 0 if the file does not exist.
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
    /// Size in bytes of `.speedy/sac.sqlite`. 0 if the file does not exist.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_status_serde_roundtrip() {
        let s = DaemonStatus {
            pid: 1234,
            uptime_secs: 99,
            workspace_count: 3,
            watcher_count: 3,
            version: "1.2.3".to_string(),
            protocol_version: 2,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: DaemonStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pid, 1234);
        assert_eq!(back.uptime_secs, 99);
        assert_eq!(back.workspace_count, 3);
        assert_eq!(back.version, "1.2.3");
        assert_eq!(back.protocol_version, 2);
    }

    #[test]
    fn test_daemon_status_legacy_missing_protocol_version_defaults_to_zero() {
        let json = r#"{"pid":1,"uptime_secs":0,"workspace_count":0,"watcher_count":0,"version":"0.1.0"}"#;
        let s: DaemonStatus = serde_json::from_str(json).unwrap();
        assert_eq!(s.protocol_version, 0, "missing field should default to 0");
    }

    #[test]
    fn test_metrics_serde_roundtrip() {
        let m = Metrics {
            queries: 10,
            indexes: 20,
            syncs: 5,
            watcher_events: 100,
            exec_calls: 7,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: Metrics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.queries, 10);
        assert_eq!(back.watcher_events, 100);
        assert_eq!(back.exec_calls, 7);
    }

    #[test]
    fn test_metrics_default_all_zeros() {
        let m = Metrics::default();
        assert_eq!(m.queries, 0);
        assert_eq!(m.indexes, 0);
        assert_eq!(m.syncs, 0);
        assert_eq!(m.watcher_events, 0);
        assert_eq!(m.exec_calls, 0);
    }

    #[test]
    fn test_workspace_status_serde_roundtrip() {
        let ws = WorkspaceStatus {
            path: "/home/user/project".to_string(),
            watcher_alive: true,
            last_event_at: Some(1700000000),
            last_sync_at: None,
            index_size_bytes: 4096,
            chunk_count: Some(42),
        };
        let json = serde_json::to_string(&ws).unwrap();
        let back: WorkspaceStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "/home/user/project");
        assert!(back.watcher_alive);
        assert_eq!(back.last_event_at, Some(1700000000));
        assert!(back.last_sync_at.is_none());
        assert_eq!(back.chunk_count, Some(42));
    }

    #[test]
    fn test_workspace_status_optional_fields_null() {
        let json = r#"{"path":"/p","watcher_alive":false,"last_event_at":null,"last_sync_at":null,"index_size_bytes":0,"chunk_count":null}"#;
        let ws: WorkspaceStatus = serde_json::from_str(json).unwrap();
        assert!(ws.last_event_at.is_none());
        assert!(ws.chunk_count.is_none());
        assert!(!ws.watcher_alive);
    }

    #[test]
    fn test_scan_result_serde_roundtrip() {
        let sr = ScanResult {
            path: "/data/project".to_string(),
            registered: false,
            last_modified: Some("2024-01-01T00:00:00Z".to_string()),
            db_size_bytes: 1024,
        };
        let json = serde_json::to_string(&sr).unwrap();
        let back: ScanResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "/data/project");
        assert!(!back.registered);
        assert_eq!(back.last_modified.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(back.db_size_bytes, 1024);
    }

    #[test]
    fn test_scan_result_no_last_modified() {
        let sr = ScanResult {
            path: "/p".to_string(),
            registered: true,
            last_modified: None,
            db_size_bytes: 0,
        };
        let json = serde_json::to_string(&sr).unwrap();
        let back: ScanResult = serde_json::from_str(&json).unwrap();
        assert!(back.last_modified.is_none());
    }

    #[test]
    fn test_log_line_serde_roundtrip() {
        let ll = LogLine {
            ts: "2024-01-01T00:00:00Z".to_string(),
            level: "info".to_string(),
            target: "speedy_daemon".to_string(),
            message: "hello world".to_string(),
            fields: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&ll).unwrap();
        let back: LogLine = serde_json::from_str(&json).unwrap();
        assert_eq!(back.level, "info");
        assert_eq!(back.message, "hello world");
        assert!(back.fields.is_empty());
    }

    #[test]
    fn test_log_line_empty_fields_omitted_on_serialize() {
        let ll = LogLine {
            ts: "t".to_string(),
            level: "debug".to_string(),
            target: "t".to_string(),
            message: "m".to_string(),
            fields: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&ll).unwrap();
        assert!(!json.contains("\"fields\""), "empty fields should be omitted: {json}");
    }

    #[test]
    fn test_log_line_with_extra_fields() {
        let json = r#"{"ts":"t","level":"warn","target":"t","message":"m","fields":{"workspace":"/home"}}"#;
        let ll: LogLine = serde_json::from_str(json).unwrap();
        assert_eq!(ll.fields["workspace"].as_str().unwrap(), "/home");
    }

    #[test]
    fn test_log_line_missing_fields_defaults_to_empty_map() {
        let json = r#"{"ts":"t","level":"debug","target":"t","message":"m"}"#;
        let ll: LogLine = serde_json::from_str(json).unwrap();
        assert!(ll.fields.is_empty());
    }
}
