use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::local_sock::{GenericNamespaced, Name, Stream as LocalStream, StreamTrait as _, ToNsName};
use crate::types::{LogLine, ScanResult, WorkspaceStatus};

pub use crate::types::DaemonStatus;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Wire-format version this client understands. If a daemon reports a higher
/// value in `status`, callers should treat it as incompatible.
///
/// v2 (2026-05-14): added `query-all` for cross-workspace search.
pub const SUPPORTED_PROTOCOL_VERSION: u32 = 2;

/// Client per comunicare con il daemon centralizzato via local socket.
pub struct DaemonClient {
    pub socket_name: Name<'static>,
}

impl DaemonClient {
    pub fn new(name: impl ToString) -> Self {
        let n = name.to_string();
        let name = n
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .expect("invalid local socket name")
            .into_owned();
        Self { socket_name: name }
    }

    pub async fn is_alive(&self) -> bool {
        match tokio::time::timeout(
            CONNECT_TIMEOUT,
            LocalStream::connect(self.socket_name.borrow()),
        ).await {
            Ok(Ok(mut stream)) => {
                // Verify the daemon actually responds — a half-open named pipe
                // can accept connect() but never read/write.
                tokio::time::timeout(CONNECT_TIMEOUT, async {
                    stream.write_all(b"ping\n").await?;
                    stream.shutdown().await?;
                    let mut reader = BufReader::new(&mut stream);
                    let mut resp = String::new();
                    reader.read_line(&mut resp).await?;
                    Ok::<_, std::io::Error>(resp.trim() == "pong")
                })
                .await
                .map(|r| r.unwrap_or(false))
                .unwrap_or(false)
            }
            _ => false,
        }
    }

    async fn cmd(&self, req: &str) -> Result<String> {
        let req = req.to_string();
        let socket_name = self.socket_name.borrow();
        tokio::time::timeout(CMD_TIMEOUT, async move {
            let mut stream = LocalStream::connect(socket_name)
                .await
                .context("Cannot connect to daemon. Is it running?")?;
            stream.write_all(format!("{req}\n").as_bytes()).await?;
            stream.shutdown().await?;

            let mut reader = BufReader::new(&mut stream);
            let mut resp = String::new();
            reader.read_line(&mut resp).await?;
            Ok::<_, anyhow::Error>(resp.trim().to_string())
        })
        .await
        .context("Daemon IPC timed out")?
    }

    // ─── Public API ───────────────────────────────────────────

    pub async fn ping(&self) -> Result<String> {
        self.cmd("ping").await
    }

    pub async fn status(&self) -> Result<DaemonStatus> {
        let resp = self.cmd("status").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn get_all_workspaces(&self) -> Result<Vec<String>> {
        let resp = self.cmd("list").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn is_workspace(&self, path: &str) -> Result<bool> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("is-workspace {}", canonical.display())).await?;
        Ok(resp == "true")
    }

    pub async fn add_workspace(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("add {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon add_workspace error: {resp}");
        }
        Ok(())
    }

    pub async fn remove_workspace(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("remove {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon remove_workspace error: {resp}");
        }
        Ok(())
    }

    pub async fn sync(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("sync {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon sync error: {resp}");
        }
        Ok(())
    }

    /// Query every registered workspace and return the top-K aggregated
    /// results (sorted by similarity score). Each item in the returned array
    /// carries a `workspace` field naming the source workspace.
    ///
    /// Requires daemon protocol version >= 2.
    pub async fn query_all(&self, query: &str, top_k: usize) -> Result<serde_json::Value> {
        let resp = self.cmd(&format!("query-all\t{top_k}\t{query}")).await?;
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn watch_count(&self) -> Result<usize> {
        let resp = self.cmd("watch-count").await?;
        Ok(resp.parse()?)
    }

    /// Fetch cumulative counters from the daemon (queries, indexes, syncs,
    /// watcher_events, exec_calls).
    pub async fn metrics(&self) -> Result<serde_json::Value> {
        let resp = self.cmd("metrics").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn daemon_pid(&self) -> Result<u32> {
        let resp = self.cmd("daemon-pid").await?;
        Ok(resp.parse()?)
    }

    pub async fn stop(&self) -> Result<()> {
        self.cmd("stop").await?;
        Ok(())
    }

    pub async fn reload(&self) -> Result<String> {
        self.cmd("reload").await
    }

    /// Walk `root` looking for `.speedy/index.sqlite` and return one entry per
    /// hit. `max_depth` caps how deep the walker descends (default 8 on the
    /// daemon side if `None`).
    pub async fn scan(&self, root: &str, max_depth: Option<usize>) -> Result<Vec<ScanResult>> {
        let req = match max_depth {
            Some(d) => format!("scan\t{root}\t{d}"),
            None => format!("scan\t{root}"),
        };
        let resp = self.cmd(&req).await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// Force a clean re-index of the workspace. Equivalent in effect to
    /// `exec <path> index .` but tracked separately on the daemon side.
    pub async fn reindex(&self, path: &str) -> Result<String> {
        let canonical = Path::new(path).canonicalize()?;
        self.cmd(&format!("reindex {}", canonical.display())).await
    }

    pub async fn workspace_status(&self, path: &str) -> Result<WorkspaceStatus> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("workspace-status {}", canonical.display())).await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// Snapshot of the last `n` log lines from the current daemon log file.
    pub async fn tail_log(&self, n: usize) -> Result<Vec<LogLine>> {
        let resp = self.cmd(&format!("tail-log {n}")).await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// Open a long-lived connection that streams one JSON-encoded `LogLine`
    /// per `\n` from the daemon. The returned stream stays open until the
    /// caller drops the receiver or the daemon shuts down.
    ///
    /// Returned tuple: `(rx, abort)`. Dropping `abort` (or calling `abort()`
    /// on it) tears down the background task and closes the socket.
    pub async fn subscribe_log(
        &self,
    ) -> Result<(
        tokio::sync::mpsc::UnboundedReceiver<LogLine>,
        tokio::task::JoinHandle<()>,
    )> {
        let mut stream = LocalStream::connect(self.socket_name.borrow())
            .await
            .context("Cannot connect to daemon for subscribe-log")?;
        stream.write_all(b"subscribe-log\n").await?;
        // Read the first line: the daemon answers with `ok\n` once the
        // subscription is registered, then keeps streaming `LogLine` JSONs.
        let mut reader = BufReader::new(stream);
        let mut handshake = String::new();
        reader.read_line(&mut handshake).await?;
        if handshake.trim() != "ok" {
            anyhow::bail!("subscribe-log handshake failed: {}", handshake.trim());
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let parsed: Result<LogLine, _> = serde_json::from_str(line.trim());
                        if let Ok(item) = parsed {
                            if tx.send(item).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok((rx, handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_sock::{Listener, ListenerOptions, ListenerTrait as _};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SOCKET_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn unique_socket(label: &str) -> String {
        let n = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("speedy_dc_test_{label}_{}_{n}", std::process::id())
    }

    /// Spawn a one-shot tokio task that accepts connections and replies to
    /// each line of input with the response produced by `responder`. The
    /// task lives until `cancel_rx` resolves.
    fn spawn_mock_server(
        socket: String,
        responder: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap().into_owned();
        let listener: Listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .expect("bind mock listener");
        let responder = std::sync::Arc::new(responder);

        tokio::spawn(async move {
            loop {
                let accept = listener.accept().await;
                let stream = match accept {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let r = responder.clone();
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.split();
                    let mut buf_reader = BufReader::new(reader);
                    let mut line = String::new();
                    if buf_reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    if let Some(resp) = r(line.trim()) {
                        let _ = writer.write_all(format!("{resp}\n").as_bytes()).await;
                        let _ = writer.shutdown().await;
                    }
                });
            }
        })
    }

    async fn wait_listener_ready(socket: &str) {
        // Give the listener a tick to actually bind before the client connects.
        for _ in 0..20 {
            let name = socket.to_ns_name::<GenericNamespaced>().unwrap();
            if LocalStream::connect(name).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn test_ping_returns_pong() {
        let socket = unique_socket("ping");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert_eq!(req, "ping");
            Some("pong".to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert_eq!(client.ping().await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn test_status_parses_json_into_struct() {
        let socket = unique_socket("status");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert_eq!(req, "status");
            Some(r#"{"pid":12345,"uptime_secs":42,"workspace_count":2,"watcher_count":2,"version":"9.9.9","protocol_version":1}"#.to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        let s = client.status().await.unwrap();
        assert_eq!(s.pid, 12345);
        assert_eq!(s.uptime_secs, 42);
        assert_eq!(s.workspace_count, 2);
        assert_eq!(s.watcher_count, 2);
        assert_eq!(s.version, "9.9.9");
        assert_eq!(s.protocol_version, 1);
    }

    #[tokio::test]
    async fn test_status_missing_protocol_version_defaults_to_zero() {
        let socket = unique_socket("status_legacy");
        let _server = spawn_mock_server(socket.clone(), |_req| {
            Some(r#"{"pid":1,"uptime_secs":0,"workspace_count":0,"watcher_count":0,"version":"0.1.0"}"#.to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        let s = client.status().await.unwrap();
        assert_eq!(s.protocol_version, 0, "legacy daemon (no field) → default 0");
    }

    #[tokio::test]
    async fn test_status_invalid_json_errors() {
        let socket = unique_socket("status_bad");
        let _server = spawn_mock_server(socket.clone(), |_| Some("not json".to_string()));
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert!(client.status().await.is_err());
    }

    #[tokio::test]
    async fn test_get_all_workspaces_parses_array() {
        let socket = unique_socket("list");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert_eq!(req, "list");
            Some(r#"["C:\\one","C:\\two"]"#.to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        let list = client.get_all_workspaces().await.unwrap();
        assert_eq!(list, vec!["C:\\one".to_string(), "C:\\two".to_string()]);
    }

    #[tokio::test]
    async fn test_get_all_workspaces_empty() {
        let socket = unique_socket("list_empty");
        let _server = spawn_mock_server(socket.clone(), |_| Some("[]".to_string()));
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert!(client.get_all_workspaces().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_is_workspace_true_and_false() {
        let socket = unique_socket("isws");
        let _server = spawn_mock_server(socket.clone(), |req| {
            if req.contains("yes") { Some("true".to_string()) } else { Some("false".to_string()) }
        });
        wait_listener_ready(&socket).await;

        // is_workspace canonicalizes the path. Use an actual dir.
        let dir_yes = std::env::temp_dir().join("speedy_dc_yes");
        let dir_no = std::env::temp_dir().join("speedy_dc_no");
        std::fs::create_dir_all(&dir_yes).unwrap();
        std::fs::create_dir_all(&dir_no).unwrap();

        let client = DaemonClient::new(&socket);
        // The mock matches on the substring "yes" in the request line; the
        // canonical path of dir_yes contains "yes" because the dir name does.
        assert!(client.is_workspace(dir_yes.to_str().unwrap()).await.unwrap());
        assert!(!client.is_workspace(dir_no.to_str().unwrap()).await.unwrap());

        let _ = std::fs::remove_dir_all(&dir_yes);
        let _ = std::fs::remove_dir_all(&dir_no);
    }

    #[tokio::test]
    async fn test_add_workspace_ok() {
        let socket = unique_socket("add_ok");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert!(req.starts_with("add "), "expected 'add', got: {req}");
            Some("ok".to_string())
        });
        wait_listener_ready(&socket).await;

        let dir = std::env::temp_dir().join("speedy_dc_add_ok");
        std::fs::create_dir_all(&dir).unwrap();
        let client = DaemonClient::new(&socket);
        assert!(client.add_workspace(dir.to_str().unwrap()).await.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_add_workspace_error_bails() {
        let socket = unique_socket("add_err");
        let _server = spawn_mock_server(socket.clone(), |_| Some("error: nope".to_string()));
        wait_listener_ready(&socket).await;

        let dir = std::env::temp_dir().join("speedy_dc_add_err");
        std::fs::create_dir_all(&dir).unwrap();
        let client = DaemonClient::new(&socket);
        let err = client.add_workspace(dir.to_str().unwrap()).await.unwrap_err().to_string();
        assert!(err.contains("error: nope"), "unexpected error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_remove_workspace_ok_and_err() {
        let socket = unique_socket("rm");
        let _server = spawn_mock_server(socket.clone(), |req| {
            if req.contains("good") { Some("ok".to_string()) } else { Some("error: not found".to_string()) }
        });
        wait_listener_ready(&socket).await;

        let good = std::env::temp_dir().join("speedy_dc_good");
        let bad = std::env::temp_dir().join("speedy_dc_bad");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::create_dir_all(&bad).unwrap();

        let client = DaemonClient::new(&socket);
        assert!(client.remove_workspace(good.to_str().unwrap()).await.is_ok());
        let err = client.remove_workspace(bad.to_str().unwrap()).await.unwrap_err().to_string();
        assert!(err.contains("error: not found"));

        let _ = std::fs::remove_dir_all(&good);
        let _ = std::fs::remove_dir_all(&bad);
    }

    #[tokio::test]
    async fn test_sync_ok_and_err() {
        let socket = unique_socket("sync");
        let _server = spawn_mock_server(socket.clone(), |req| {
            if req.contains("yes") { Some("ok".to_string()) } else { Some("error: bad".to_string()) }
        });
        wait_listener_ready(&socket).await;

        let ok_dir = std::env::temp_dir().join("speedy_dc_sync_yes");
        let err_dir = std::env::temp_dir().join("speedy_dc_sync_no");
        std::fs::create_dir_all(&ok_dir).unwrap();
        std::fs::create_dir_all(&err_dir).unwrap();

        let client = DaemonClient::new(&socket);
        assert!(client.sync(ok_dir.to_str().unwrap()).await.is_ok());
        assert!(client.sync(err_dir.to_str().unwrap()).await.is_err());

        let _ = std::fs::remove_dir_all(&ok_dir);
        let _ = std::fs::remove_dir_all(&err_dir);
    }

    #[tokio::test]
    async fn test_watch_count_parses_number() {
        let socket = unique_socket("wc");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert_eq!(req, "watch-count");
            Some("17".to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert_eq!(client.watch_count().await.unwrap(), 17);
    }

    #[tokio::test]
    async fn test_watch_count_invalid_response_errors() {
        let socket = unique_socket("wc_bad");
        let _server = spawn_mock_server(socket.clone(), |_| Some("not_a_number".to_string()));
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert!(client.watch_count().await.is_err());
    }

    #[tokio::test]
    async fn test_daemon_pid_parses() {
        let socket = unique_socket("dpid");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert_eq!(req, "daemon-pid");
            Some("4321".to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert_eq!(client.daemon_pid().await.unwrap(), 4321);
    }

    #[tokio::test]
    async fn test_stop_sends_stop() {
        let socket = unique_socket("stop");
        let saw_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_stop_clone = saw_stop.clone();
        let _server = spawn_mock_server(socket.clone(), move |req| {
            if req == "stop" {
                saw_stop_clone.store(true, Ordering::SeqCst);
            }
            Some("ok".to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        client.stop().await.unwrap();
        assert!(saw_stop.load(Ordering::SeqCst), "server never saw 'stop'");
    }

    #[tokio::test]
    async fn test_is_alive_true() {
        let socket = unique_socket("alive");
        let _server = spawn_mock_server(socket.clone(), |req| {
            assert_eq!(req, "ping");
            Some("pong".to_string())
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert!(client.is_alive().await);
    }

    #[tokio::test]
    async fn test_is_alive_false_when_no_listener() {
        let socket = unique_socket("dead");
        let client = DaemonClient::new(&socket);
        assert!(!client.is_alive().await);
    }

    #[tokio::test]
    async fn test_is_alive_false_when_wrong_response() {
        let socket = unique_socket("wrong");
        let _server = spawn_mock_server(socket.clone(), |_| Some("garbage".to_string()));
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        assert!(!client.is_alive().await, "is_alive should reject non-pong responses");
    }

    #[tokio::test]
    async fn test_is_alive_false_when_server_never_replies() {
        // Bind a listener that accepts but never writes: simulates a half-open
        // pipe. is_alive must fall back to the timeout and return false.
        let socket = unique_socket("hang");
        let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap().into_owned();
        let listener: Listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        let _guard = tokio::spawn(async move {
            loop {
                if let Ok(stream) = listener.accept().await {
                    // Hold the stream open without reading or writing.
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        drop(stream);
                    });
                }
            }
        });

        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        let start = std::time::Instant::now();
        let alive = client.is_alive().await;
        let elapsed = start.elapsed();
        assert!(!alive, "half-open pipe must not report alive");
        assert!(elapsed < Duration::from_secs(5), "is_alive should bail within timeout, took {elapsed:?}");
    }

    #[tokio::test]
    async fn test_cmd_connect_refused_returns_error() {
        // No listener on this socket → connect must fail.
        let socket = unique_socket("refused");
        let client = DaemonClient::new(&socket);
        assert!(client.ping().await.is_err());
    }

    #[tokio::test]
    async fn test_cmd_returns_error_on_eof() {
        // Server accepts then immediately drops the connection — no response
        // line. The client must surface that as an error, not hang.
        let socket = unique_socket("eof");
        let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap().into_owned();
        let listener: Listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        let _server = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(stream) => drop(stream),
                    Err(_) => return,
                }
            }
        });
        wait_listener_ready(&socket).await;

        let client = DaemonClient::new(&socket);
        // ping() reads the response and trims it — empty trimmed response is
        // not "pong", so the call returns Ok("") rather than Err.
        // Instead, status() expects JSON; empty body will fail JSON parse.
        let result = client.status().await;
        assert!(result.is_err(), "status should error when server closes early: {result:?}");
    }
}
