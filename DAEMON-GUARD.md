# Daemon Guard — Daemon Centralizzato

## Il Problema

Attualmente: **un processo daemon per ogni workspace**. Ognuno scrive il suo PID in `.speedy/daemon.json`.

- PC riavviato → tutti i PID sono stale, ogni `daemon.json` mente dicendo "running"
- `tasklist`/`kill -0` → un nuovo processo può aver riusato lo stesso PID, falso positivo
- N servizi OS da gestire, N controlli da fare

## Soluzione: Daemon Unico Centralizzato con IPC TCP

```
~/.config/speedy/
├── workspaces.json     ← lista workspace registrati
├── daemon.pid          ← PID del daemon centrale
│
TCP 127.0.0.1:42137    ← IPC socket del daemon
```

Un unico processo che:
- Monitora TUTTI i workspace contemporaneamente (thread per ognuno)
- Espone un socket TCP su `127.0.0.1:42137`
- Risponde a comandi testuali (line-based JSON)
- Si auto-avvia al primo `speedy` dopo un riavvio

## API del Daemon — Protocollo IPC

Tutto su `TCP 127.0.0.1:42137`, richiesta/risposta line-based.

```
>  richiesta\n
<  risposta\n
```

| Comando | Descrizione |
|---|---|
| `ping` | Health check base. Risponde `pong` |
| `status` | Info complete sul daemon. Risponde JSON |
| `list` | Lista workspace monitorati. Risponde `["path", ...]` |
| `is-workspace <path>` | Verifica se path è monitorato. Risponde `true`/`false` |
| `add <path>` | Aggiunge workspace, avvia watcher, fa index. Risponde `ok` |
| `remove <path>` | Ferma watcher e rimuove workspace. Risponde `ok` |
| `reindex <path>` | Forza reindex completo di un workspace. Risponde `ok` |
| `watch-count` | Numero di watcher attivi. Risponde `3` |
| `daemon-pid` | PID del daemon. Risponde `1234` |
| `stop` | Ferma il daemon (graceful shutdown) |

### Risposte Dettagliate

```
> ping
< pong

> status
< {"pid": 1234, "uptime_secs": 3600, "workspace_count": 2, "watcher_count": 2, "version": "0.1.0"}

> list
< ["C:/Users/me/project1", "C:/Users/me/project2"]

> is-workspace C:/Users/me/project1
< true

> add C:/Users/me/project3
< ok

> remove C:/Users/me/project3
< ok

> reindex C:/Users/me/project1
< ok

> watch-count
< 2

> daemon-pid
< 1234

> stop
< ok
```

## DaemonClient — Libreria per Chiamare il Daemon

Chiunque (CLI, MCP server, script esterno) chiama il daemon via `DaemonClient`.
Nessuna dipendenza: solo TCP + JSON.

```rust
// ============================================================
// speedy-daemon-client/src/lib.rs
// ============================================================
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub const DAEMON_PORT: u16 = 42137;

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub pid: u32,
    pub uptime_secs: u64,
    pub workspace_count: usize,
    pub watcher_count: usize,
    pub version: String,
}

pub struct DaemonClient {
    addr: String,
}

impl DaemonClient {
    pub fn new(port: u16) -> Self {
        Self { addr: format!("127.0.0.1:{}", port) }
    }

    pub fn default() -> Self {
        Self::new(DAEMON_PORT)
    }

    /// Prova a connettersi. Se fallisce → daemon morto.
    pub async fn is_alive(&self) -> bool {
        TcpStream::connect(&self.addr).await.is_ok()
    }

    /// Invia un comando testuale, riceve risposta (bloccante sulla riga).
    async fn cmd(&self, req: &str) -> Result<String> {
        let mut stream = TcpStream::connect(&self.addr)
            .await
            .context("Cannot connect to daemon")?;
        stream.write_all(format!("{req}\n").as_bytes()).await?;
        stream.shutdown().await?;

        let mut reader = BufReader::new(&mut stream);
        let mut resp = String::new();
        reader.read_line(&mut resp).await?;
        Ok(resp.trim().to_string())
    }

    // ─── Public API ───────────────────────────────────────────

    pub async fn ping(&self) -> Result<String> {
        self.cmd("ping").await
    }

    pub async fn status(&self) -> Result<DaemonStatus> {
        let resp = self.cmd("status").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// Lista completa dei workspace monitorati dal daemon.
    pub async fn get_all_workspaces(&self) -> Result<Vec<String>> {
        let resp = self.cmd("list").await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// true se il path è attualmente monitorato dal daemon.
    pub async fn is_workspace(&self, path: &str) -> Result<bool> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("is-workspace {}", canonical.display())).await?;
        Ok(resp == "true")
    }

    /// Aggiunge un workspace al daemon: registra, avvia watcher, indicizza.
    pub async fn add_workspace(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("add {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon returned: {resp}");
        }
        Ok(())
    }

    /// Rimuove un workspace: ferma watcher, deregistra.
    pub async fn remove_workspace(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("remove {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon returned: {resp}");
        }
        Ok(())
    }

    /// Forza reindex completo di un workspace.
    pub async fn reindex(&self, path: &str) -> Result<()> {
        let canonical = Path::new(path).canonicalize()?;
        let resp = self.cmd(&format!("reindex {}", canonical.display())).await?;
        if resp != "ok" {
            anyhow::bail!("Daemon returned: {resp}");
        }
        Ok(())
    }

    /// Quanti watcher attivi.
    pub async fn watch_count(&self) -> Result<usize> {
        let resp = self.cmd("watch-count").await?;
        Ok(resp.parse()?)
    }

    /// PID del processo daemon.
    pub async fn daemon_pid(&self) -> Result<u32> {
        let resp = self.cmd("daemon-pid").await?;
        Ok(resp.parse()?)
    }

    /// Ferma il daemon.
    pub async fn stop(&self) -> Result<()> {
        self.cmd("stop").await?;
        Ok(())
    }
}
```

## Daemon — Server Side (Implementazione)

```rust
// ============================================================
// speedy-core/src/daemon_central.rs
// ============================================================
use crate::config::Config;
use crate::indexer::Indexer;
use crate::workspace;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

pub const DAEMON_PORT: u16 = 42137;
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Stato interno del daemon centrale.
pub struct CentralDaemon {
    pub pid: u32,
    pub started_at: Instant,
    pub workspaces: Arc<Mutex<HashMap<String, WorkspaceWatcher>>>,
}

struct WorkspaceWatcher {
    path: String,
    /// JoinHandle per fermare il watcher
    handle: tokio::task::JoinHandle<()>,
}

impl CentralDaemon {
    pub fn new() -> Self {
        Self {
            pid: std::process::id(),
            started_at: Instant::now(),
            workspaces: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // ─── Avvio ────────────────────────────────────────────────

    pub async fn start(&self) -> Result<()> {
        let daemon_dir = daemon_dir_path()?;
        std::fs::create_dir_all(&daemon_dir)?;

        // Pulisci eventuale daemon morto precedente
        kill_existing_daemon(&daemon_dir);
        std::fs::write(daemon_dir.join("daemon.pid"), self.pid.to_string())?;

        // Carica workspace registrati e avvia watcher
        let registered = workspace::list()?;
        let ws_map = self.workspaces.clone();
        for entry in &registered {
            let path = Path::new(&entry.path);
            if path.exists() {
                let handle = spawn_watcher(&entry.path).await?;
                ws_map.lock().await.insert(entry.path.clone(), WorkspaceWatcher {
                    path: entry.path.clone(),
                    handle,
                });
            }
        }

        // Avvia IPC server
        let listener = TcpListener::bind(format!("127.0.0.1:{DAEMON_PORT}"))
            .await
            .context("Failed to bind daemon port")?;

        println!("Speedy daemon v{DAEMON_VERSION} (PID {}) listening on port {DAEMON_PORT}", self.pid);

        loop {
            let (socket, _) = listener.accept().await?;
            let ws_map = self.workspaces.clone();
            let started = self.started_at;
            let pid = self.pid;
            tokio::spawn(async move {
                if let Err(e) = handle_connection(socket, ws_map, pid, started).await {
                    eprintln!("Daemon IPC error: {e}");
                }
            });
        }
    }
}

// ─── Utility ────────────────────────────────────────────────

fn daemon_dir_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("no config dir")?
        .join("speedy");
    Ok(dir)
}

pub fn kill_existing_daemon(daemon_dir: &Path) {
    let pid_path = daemon_dir.join("daemon.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            #[cfg(windows)] {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"]).status();
            }
            #[cfg(not(windows))] {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()]).status();
            }
        }
    }
    let _ = std::fs::remove_file(&pid_path);
}

// ─── Watcher per Workspace ──────────────────────────────────

async fn spawn_watcher(path: &str) -> Result<tokio::task::JoinHandle<()>> {
    let path = path.to_string();
    let handle = tokio::spawn(async move {
        let config = Config::from_env();
        std::env::set_current_dir(&path).ok();
        match Indexer::new(&config).await {
            Ok(indexer) => {
                // Avvia file watcher (notify) per questo percorso
                if let Err(e) = run_file_watcher(&path, indexer).await {
                    eprintln!("Watcher error for {path}: {e}");
                }
            }
            Err(e) => eprintln!("Failed to create indexer for {path}: {e}"),
        }
    });
    Ok(handle)
}

async fn run_file_watcher(path: &str, _indexer: Indexer) -> Result<()> {
    // Logica notify-based watcher (uguale all'attuale watcher.rs)
    // ma specifica per un singolo workspace
    Ok(())
}

// ─── IPC Connection Handler ─────────────────────────────────

async fn handle_connection(
    mut socket: TcpStream,
    workspaces: Arc<Mutex<HashMap<String, WorkspaceWatcher>>>,
    pid: u32,
    started_at: Instant,
) -> Result<()> {
    let (reader, mut writer) = socket.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;
    let line = line.trim();

    let resp = match line {
        "ping" => "pong\n".to_string(),

        "status" => {
            let ws = workspaces.lock().await;
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
            let ws = workspaces.lock().await;
            let paths: Vec<&String> = ws.keys().collect();
            format!("{}\n", serde_json::to_string(&paths)?)
        }

        _ if line.starts_with("is-workspace ") => {
            let path = line.trim_start_matches("is-workspace ");
            let ws = workspaces.lock().await;
            let canonical = Path::new(path).canonicalize().ok();
            let found = canonical
                .and_then(|c| ws.keys().find(|k| {
                    Path::new(k).canonicalize().ok().as_ref() == Some(&c)
                }))
                .is_some();
            format!("{found}\n")
        }

        _ if line.starts_with("add ") => {
            let path = line.trim_start_matches("add ");
            let canonical = Path::new(path).canonicalize()?;
            let path_str = canonical.to_string_lossy().to_string();

            // Registra nei workspace persistenti
            if !workspace::is_registered(&path_str) {
                workspace::add(&path_str)?;
            }

            // Avvia watcher se non già attivo
            let mut ws = workspaces.lock().await;
            if !ws.contains_key(&path_str) {
                let handle = spawn_watcher(&path_str).await?;
                ws.insert(path_str.clone(), WorkspaceWatcher {
                    path: path_str.clone(),
                    handle,
                });
                // Reindex iniziale
                let config = Config::from_env();
                std::env::set_current_dir(&path_str).ok();
                let indexer = Indexer::new(&config).await?;
                let _ = indexer.sync_all().await;
            }
            "ok\n".to_string()
        }

        _ if line.starts_with("remove ") => {
            let path = line.trim_start_matches("remove ");
            let mut ws = workspaces.lock().await;
            if let Some(watcher) = ws.remove(path) {
                watcher.handle.abort();
            }
            let _ = workspace::remove(path);
            "ok\n".to_string()
        }

        _ if line.starts_with("reindex ") => {
            let path = line.trim_start_matches("reindex ");
            let config = Config::from_env();
            std::env::set_current_dir(path).ok();
            let indexer = Indexer::new(&config).await?;
            let _ = indexer.sync_all().await;
            "ok\n".to_string()
        }

        "watch-count" => {
            let ws = workspaces.lock().await;
            format!("{}\n", ws.len())
        }

        "daemon-pid" => {
            format!("{pid}\n")
        }

        "stop" => {
            // Graceful shutdown
            std::process::exit(0);
        }

        _ => {
            format!("error: unknown command: {line}\n")
        }
    };

    writer.write_all(resp.as_bytes()).await?;
    Ok(())
}
```

## Pre-flight Check: `ensure_daemon()`

Questa funzione va chiamata all'inizio di ogni comando `speedy`.

```rust
// ============================================================
// speedy-core/src/bin/speedy.rs
// ============================================================

use speedy::daemon_client::DaemonClient;

async fn ensure_daemon() -> Result<()> {
    let client = DaemonClient::default();
    let cwd = std::env::current_dir()?.canonicalize()?;
    let cwd_str = cwd.to_string_lossy().to_string();

    if client.is_alive().await {
        // Daemon vivo → verifica workspace
        let workspaces = client.get_all_workspaces().await?;
        if workspaces.contains(&cwd_str) {
            // ✅ Tutto ok
            return Ok(());
        }

        // Workspace non monitorato → aggiungilo
        if !workspace::is_registered(&cwd_str) {
            workspace::add(&cwd_str)?;
        }
        client.add_workspace(&cwd_str).await?;
        println!("✓ Workspace aggiunto al daemon.");
        return Ok(());
    }

    // Daemon morto → riavvia
    eprintln!("⚠ Daemon non risponde. Avvio...");
    let daemon_dir = dirs::config_dir()
        .context("no config dir")?
        .join("speedy");
    kill_existing_daemon(&daemon_dir);
    spawn_daemon_process()?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Registra workspace se necessario
    if !workspace::is_registered(&cwd_str) {
        workspace::add(&cwd_str)?;
    }

    // Reindex
    let client = DaemonClient::default();
    client.add_workspace(&cwd_str).await?;
    println!("✓ Daemon avviato, workspace indicizzato.");
    Ok(())
}

fn spawn_daemon_process() -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(&exe)
            .arg("daemon")
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
    }
    #[cfg(not(windows))] {
        std::process::Command::new(&exe)
            .arg("daemon")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
    }
    Ok(())
}
```

## Entry Point del Daemon

```rust
// in cli.rs
#[derive(Subcommand)]
pub enum Commands {
    // ... comandi esistenti ...
    /// Avvia il daemon centralizzato in background
    Daemon,  // non ha sub-azioni, è il daemon stesso
}
```

```rust
// in bin/speedy.rs
Some(Commands::Daemon) => {
    let daemon = CentralDaemon::new();
    daemon.start().await?;
}
```

## DaemonClient Chiamabile da Ovunque

Il `DaemonClient` può essere usato da **qualunque codice Rust** che abbia accesso alla crate,
incluso il **MCP server** (`speedy-mcp`) o script esterni.

```rust
// Uso esterno (es. in speedy-mcp/src/main.rs)
use speedy::daemon_client::DaemonClient;

async fn mcp_handler() {
    let client = DaemonClient::default();

    // Health check
    if !client.is_alive().await {
        // Fai partire il daemon
    }

    // Lista workspace
    let workspaces = client.get_all_workspaces().await?;

    // Verifica workspace specifico
    if client.is_workspace("C:/my-project").await? {
        println!("è monitorato!");
    }

    // Aggiungi workspace
    client.add_workspace("C:/new-project").await?;

    // Reindex
    client.reindex("C:/my-project").await?;

    // Stats
    let status = client.status().await?;
    println!("Watcher attivi: {}", status.watcher_count);
}
```

## Schema Comunicazione Completo

```
┌──────────────────────────────────────────────────────────────────┐
│                        ECOSISTEMA SPEEDY                         │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐   │
│  │  CLI speedy   │    │  MCP Server   │    │  Script esterno   │   │
│  │  (bin/speedy) │    │  (speedy-mcp) │    │  (python, bash..) │   │
│  └──────┬───────┘    └──────┬───────┘    └────────┬─────────┘   │
│         │                   │                      │             │
│         └─────────┬─────────┴──────────┬───────────┘             │
│                   │                     │                        │
│            ┌──────▼─────────────────────▼──────┐                │
│            │       DaemonClient (lib)           │                │
│            │  - is_alive()                      │                │
│            │  - ping()                          │                │
│            │  - status()                        │                │
│            │  - get_all_workspaces()            │                │
│            │  - is_workspace(path)              │                │
│            │  - add_workspace(path)             │                │
│            │  - remove_workspace(path)          │                │
│            │  - reindex(path)                   │                │
│            │  - watch_count()                   │                │
│            │  - daemon_pid()                    │                │
│            │  - stop()                          │                │
│            └──────────────┬─────────────────────┘                │
│                           │                                      │
│                    TCP 127.0.0.1:42137                            │
│                           │                                      │
│            ┌──────────────▼─────────────────────┐                │
│            │      CentralDaemon (server)        │                │
│            │  - IPC TCP handler                 │                │
│            │  - N watcher thread (1 per ws)     │                │
│            │  - workspace registry              │                │
│            │  - PID file management             │                │
│            │  - auto-recovery                   │                │
│            └────────────────────────────────────┘                │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

## Flusso Completo: Riavvio PC → Primo Comando

```
PC riparte
  │
  ├─ ~/.config/speedy/workspaces.json  ← intatto (su disco)
  ├─ ~/.config/speedy/daemon.pid       ← stale (processo morto)
  │
  └─ $ speedy query "cose"
      │
      ├─ ensure_daemon()
      │   ├─ DaemonClient::is_alive()?  ← TCP connect FAILS
      │   ├─ kill_existing_daemon()     ← pulisce PID stale
      │   ├─ spawn_daemon_process()     ← avvia CentralDaemon
      │   │   ├─ kill_existing_daemon() ← pulizia ulteriore
      │   │   ├─ salva PID in daemon.pid
      │   │   ├─ carica workspaces.json
      │   │   ├─ per ogni ws: spawna thread watcher
      │   │   └─ avvia IPC TCP listener su :42137
      │   │
      │   ├─ wait 2 secondi
      │   ├─ DaemonClient::add_workspace(CWD)
      │   │   ├─ workspace::add(CWD)
      │   │   ├─ spawna watcher per CWD
      │   │   └─ indexer.sync_all()  ← reindex completo
      │   │
      │   └─ ✅ daemon vivo, workspace monitorato
      │
      └─ comando query eseguito
```

## API Pubbliche del DaemonClient (Riepilogo)

| Metodo | Input | Output | Cosa fa |
|---|---|---|---|
| `is_alive()` | — | `bool` | TCP connect a `127.0.0.1:42137` |
| `ping()` | — | `Result<String>` | `→ "pong"` |
| `status()` | — | `Result<DaemonStatus>` | PID, uptime, conteggi |
| `get_all_workspaces()` | — | `Result<Vec<String>>` | Lista path monitorati |
| `is_workspace(path)` | `&str` | `Result<bool>` | Path è monitorato? |
| `add_workspace(path)` | `&str` | `Result<()>` | Registra + watcher + index |
| `remove_workspace(path)` | `&str` | `Result<()>` | Ferma watcher + deregistra |
| `reindex(path)` | `&str` | `Result<()>` | Forza reindex completo |
| `watch_count()` | — | `Result<usize>` | Numero watcher attivi |
| `daemon_pid()` | — | `Result<u32>` | PID del processo daemon |
| `stop()` | — | `Result<()>` | Graceful shutdown |

## Vantaggi

| Aspetto | N daemon (prima) | 1 daemon centrale (dopo) |
|---|---|---|
| Riavvio PC | N servizi morti da diagnosticare | 1 TCP connect fallita → riavvio automatico |
| Verifica stato | `tasklist`×N + `sc query`×N (PID può mentire) | 1 TCP connect → `ping` (prova certa) |
| Aggiunta workspace | Crea servizio OS (lento, admin) | `add` via TCP (istantaneo) |
| Risorse | N processi, N connessioni DB | 1 processo, N thread, 1 DB |
| Debug | N log file sparsi | Log centralizzato |

## Note Implementative

- **Porta**: `42137` — fissa, su localhost
- **Protocollo**: line-based, UTF-8, JSON per dati strutturati
- **Retrocompatibilità**: i vecchi `.speedy/daemon.json` vanno ignorati dal nuovo sistema
- **Migrazione**: `speedy daemon migrate` → kill vecchi servizi Windows, avvia daemon centrale
- **Graceful shutdown**: su `SIGTERM`/`SIGINT`, ferma tutti i watcher, salva stato
- **PID file**: `~/.config/speedy/daemon.pid` — usato solo per cleanup su riavvio
- **Dipendenze**: nessuna nuova — TCP è nella stdlib di Tokio
