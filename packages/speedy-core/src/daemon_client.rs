use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub const DAEMON_PORT: u16 = 42137;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub pid: u32,
    pub uptime_secs: u64,
    pub workspace_count: usize,
    pub watcher_count: usize,
    pub version: String,
}

/// Client per comunicare con il daemon centralizzato via TCP.
/// Può essere usato da CLI, MCP server, script esterni, chiunque.
pub struct DaemonClient {
    addr: String,
}

impl DaemonClient {
    pub fn new(port: u16) -> Self {
        Self { addr: format!("127.0.0.1:{port}") }
    }

    pub fn default() -> Self {
        Self::new(DAEMON_PORT)
    }

    /// Prova a connettersi al daemon. Se fallisce → daemon morto.
    pub async fn is_alive(&self) -> bool {
        TcpStream::connect(&self.addr).await.is_ok()
    }

    /// Invia un comando, riceve la risposta (prima riga).
    async fn cmd(&self, req: &str) -> Result<String> {
        let mut stream = TcpStream::connect(&self.addr)
            .await
            .context("Cannot connect to daemon. Is it running?")?;
        stream.write_all(format!("{req}\n").as_bytes()).await?;
        stream.shutdown().await?;

        let mut reader = BufReader::new(&mut stream);
        let mut resp = String::new();
        reader.read_line(&mut resp).await?;
        Ok(resp.trim().to_string())
    }

    // ─── Public API ───────────────────────────────────────────

    /// Ping base. Risponde "pong".
    pub async fn ping(&self) -> Result<String> {
        self.cmd("ping").await
    }

    /// Stato completo del daemon.
    pub async fn status(&self) -> Result<DaemonStatus> {
        let resp = self.cmd("status").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// Lista completa dei workspace monitorati.
    pub async fn get_all_workspaces(&self) -> Result<Vec<String>> {
        let resp = self.cmd("list").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// true se il path è attualmente monitorato.
    pub async fn is_workspace(&self, path: &str) -> Result<bool> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("is-workspace {}", canonical.display())).await?;
        Ok(resp == "true")
    }

    /// Aggiunge un workspace: registra, avvia watcher, indicizza.
    pub async fn add_workspace(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("add {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon add_workspace error: {resp}");
        }
        Ok(())
    }

    /// Rimuove un workspace: ferma watcher, deregistra.
    pub async fn remove_workspace(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("remove {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon remove_workspace error: {resp}");
        }
        Ok(())
    }

    /// Forza reindex completo.
    pub async fn reindex(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("reindex {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon reindex error: {resp}");
        }
        Ok(())
    }

    /// Numero di watcher attivi.
    pub async fn watch_count(&self) -> Result<usize> {
        let resp = self.cmd("watch-count").await?;
        Ok(resp.parse()?)
    }

    /// PID del processo daemon.
    pub async fn daemon_pid(&self) -> Result<u32> {
        let resp = self.cmd("daemon-pid").await?;
        Ok(resp.parse()?)
    }

    /// Ferma il daemon (graceful shutdown).
    pub async fn stop(&self) -> Result<()> {
        self.cmd("stop").await?;
        Ok(())
    }
}
