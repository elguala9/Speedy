use clap::{Parser, Subcommand};
use speedy_core::daemon_client::DaemonClient;
use speedy_core::daemon_util;
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::warn;

use speedy_core::local_sock::{GenericNamespaced, Stream as LocalStream, StreamTrait as _, ToNsName};

#[derive(Parser)]
#[command(name = "speedy-cli", version, about = "Local Semantic File System - Thin Client")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short = 'p', long = "path", help = "Project root (default: current dir)")]
    project_path: Option<String>,

    #[arg(long = "daemon-socket", help = "Daemon socket name")]
    daemon_socket: Option<String>,

    #[arg(global = true, long, help = "Output in JSON format")]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Index a directory into the vector database")]
    Index {
        #[arg(default_value = ".")]
        subdir: String,
    },
    #[command(about = "Query the index with semantic search")]
    Query {
        query: String,
        #[arg(short = 'k', long = "top-k", default_value = "5")]
        top_k: usize,
        #[arg(long = "all", help = "Query across all registered workspaces and aggregate top-K")]
        all: bool,
    },
    #[command(about = "Show project context summary")]
    Context,
    #[command(about = "Sync filesystem changes to the database incrementally")]
    Sync,
    #[command(about = "Drop and rebuild every chunk's embedding (use after changing SPEEDY_MODEL)")]
    Reembed,
    #[command(about = "Force reindex of a workspace")]
    Force {
        #[arg(short = 'p', help = "Workspace path (default: current dir)")]
        path: Option<String>,
    },
    #[command(about = "Daemon management")]
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    #[command(about = "Workspace management")]
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    #[command(about = "Show daemon status")]
    Status,
    #[command(about = "List all daemon workspaces")]
    List,
    #[command(about = "Stop the daemon")]
    Stop,
    #[command(about = "Ping the daemon")]
    Ping,
}

#[derive(Subcommand)]
enum WorkspaceAction {
    #[command(about = "List all workspaces")]
    List,
    #[command(about = "Add a workspace")]
    Add {
        #[arg(help = "Workspace path")]
        path: String,
    },
    #[command(about = "Remove a workspace")]
    Remove {
        #[arg(help = "Workspace path")]
        path: String,
    },
}

fn resolve_socket_name(cli: &Cli) -> String {
    cli.daemon_socket.clone()
        .unwrap_or_else(daemon_util::default_daemon_socket_name)
}

async fn send_raw_cmd(socket_name: &str, req: &str) -> Result<String> {
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .context("invalid socket name")?;
    let mut stream = LocalStream::connect(name)
        .await
        .context("Cannot connect to daemon. Is it running?")?;
    stream.write_all(format!("{req}\n").as_bytes()).await?;
    stream.shutdown().await?;

    let mut reader = BufReader::new(&mut stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).await?;
    Ok(resp.trim().to_string())
}

async fn ensure_daemon(socket_name: &str) -> Result<()> {
    let client = DaemonClient::new(socket_name);
    if client.is_alive().await {
        return Ok(());
    }

    warn!("Daemon non risponde. Avvio...");
    let daemon_dir = daemon_util::daemon_dir_path()?;
    daemon_util::kill_existing_daemon(&daemon_dir);
    daemon_util::spawn_daemon_process(socket_name)?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    if let Some(ref p) = cli.project_path {
        std::env::set_current_dir(p)?;
    }

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main(cli))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use speedy_core::local_sock::{GenericNamespaced, ListenerOptions, ListenerTrait as _, StreamTrait as _, ToNsName};

    #[test]
    fn test_cli_assert() {
        Cli::command().debug_assert();
    }

    // ── Command parsing tests ─────────────────────────

    #[test]
    fn test_parse_index() {
        let cli = Cli::parse_from(["speedy-cli", "index"]);
        assert!(matches!(cli.command, Some(Commands::Index { .. })));
    }

    #[test]
    fn test_parse_index_with_subdir() {
        let cli = Cli::parse_from(["speedy-cli", "index", "src"]);
        assert!(matches!(cli.command, Some(Commands::Index { subdir }) if subdir == "src"));
    }

    #[test]
    fn test_parse_query() {
        let cli = Cli::parse_from(["speedy-cli", "query", "test query"]);
        assert!(matches!(cli.command, Some(Commands::Query { .. })));
    }

    #[test]
    fn test_parse_query_with_top_k() {
        let cli = Cli::parse_from(["speedy-cli", "query", "search", "-k", "10"]);
        if let Some(Commands::Query { top_k, .. }) = cli.command {
            assert_eq!(top_k, 10);
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn test_parse_context() {
        let cli = Cli::parse_from(["speedy-cli", "context"]);
        assert!(matches!(cli.command, Some(Commands::Context)));
    }

    #[test]
    fn test_parse_sync() {
        let cli = Cli::parse_from(["speedy-cli", "sync"]);
        assert!(matches!(cli.command, Some(Commands::Sync)));
    }

    #[test]
    fn test_parse_force() {
        let cli = Cli::parse_from(["speedy-cli", "force"]);
        assert!(matches!(cli.command, Some(Commands::Force { .. })));
    }

    #[test]
    fn test_parse_force_with_path() {
        let cli = Cli::parse_from(["speedy-cli", "force", "-p", "/tmp"]);
        if let Some(Commands::Force { path }) = cli.command {
            assert_eq!(path, Some("/tmp".to_string()));
        } else {
            panic!("expected Force command");
        }
    }

    #[test]
    fn test_parse_daemon_status() {
        let cli = Cli::parse_from(["speedy-cli", "daemon", "status"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: DaemonAction::Status })));
    }

    #[test]
    fn test_parse_daemon_list() {
        let cli = Cli::parse_from(["speedy-cli", "daemon", "list"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: DaemonAction::List })));
    }

    #[test]
    fn test_parse_daemon_stop() {
        let cli = Cli::parse_from(["speedy-cli", "daemon", "stop"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: DaemonAction::Stop })));
    }

    #[test]
    fn test_parse_daemon_ping() {
        let cli = Cli::parse_from(["speedy-cli", "daemon", "ping"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: DaemonAction::Ping })));
    }

    #[test]
    fn test_parse_workspace_list() {
        let cli = Cli::parse_from(["speedy-cli", "workspace", "list"]);
        assert!(matches!(cli.command, Some(Commands::Workspace { action: WorkspaceAction::List })));
    }

    #[test]
    fn test_parse_workspace_add() {
        let cli = Cli::parse_from(["speedy-cli", "workspace", "add", "/tmp/test"]);
        if let Some(Commands::Workspace { action: WorkspaceAction::Add { path } }) = cli.command {
            assert_eq!(path, "/tmp/test");
        } else {
            panic!("expected Workspace Add command");
        }
    }

    #[test]
    fn test_parse_workspace_remove() {
        let cli = Cli::parse_from(["speedy-cli", "workspace", "remove", "/tmp/test"]);
        if let Some(Commands::Workspace { action: WorkspaceAction::Remove { path } }) = cli.command {
            assert_eq!(path, "/tmp/test");
        } else {
            panic!("expected Workspace Remove command");
        }
    }

    #[test]
    fn test_parse_no_command() {
        let cli = Cli::parse_from(["speedy-cli"]);
        assert!(cli.command.is_none());
    }

    // ── Global flag tests ─────────────────────────────

    #[test]
    fn test_parse_json_flag() {
        let cli = Cli::parse_from(["speedy-cli", "--json", "context"]);
        assert!(cli.json);
    }

    #[test]
    fn test_parse_path_flag() {
        let cli = Cli::parse_from(["speedy-cli", "-p", "/my/proj", "index"]);
        assert_eq!(cli.project_path, Some("/my/proj".to_string()));
    }

    #[test]
    fn test_parse_path_long_flag() {
        let cli = Cli::parse_from(["speedy-cli", "--path", "/my/proj", "sync"]);
        assert_eq!(cli.project_path, Some("/my/proj".to_string()));
    }

    #[test]
    fn test_parse_daemon_socket() {
        let cli = Cli::parse_from(["speedy-cli", "--daemon-socket", "my-daemon", "context"]);
        assert_eq!(cli.daemon_socket, Some("my-daemon".to_string()));
    }

    #[test]
    fn test_parse_daemon_socket_default() {
        let cli = Cli::parse_from(["speedy-cli", "sync"]);
        assert!(cli.daemon_socket.is_none());
    }

    #[test]
    fn test_json_flag_with_sync() {
        let cli = Cli::parse_from(["speedy-cli", "--json", "sync"]);
        assert!(cli.json);
    }

    #[test]
    fn test_json_false_by_default() {
        let cli = Cli::parse_from(["speedy-cli", "context"]);
        assert!(!cli.json);
    }

    // ── send_raw_cmd tests with mock local socket server ────

    fn test_socket_name(label: &str) -> String {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("speedy_cli_test_{label}_{n}")
    }

    #[tokio::test]
    async fn test_send_raw_cmd_returns_response() {
        let name = test_socket_name("send_raw");
        let ns_name = name.as_str().to_ns_name::<GenericNamespaced>().unwrap();
        let listener = ListenerOptions::new().name(ns_name).create_tokio().unwrap();
        let handle = tokio::spawn(async move {
            let socket = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut reader, mut writer) = socket.split();
            let mut buf = [0u8; 1024];
            let n = reader.read(&mut buf).await.unwrap();
            let msg = String::from_utf8_lossy(&buf[..n]);
            assert_eq!(msg, "ping\n");
            writer.write_all(b"pong\n").await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let resp = send_raw_cmd(&name, "ping").await.unwrap();
        assert_eq!(resp, "pong");

        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_send_raw_cmd_connection_refused() {
        let result = send_raw_cmd("speedy_cli_test_refused_NONEXISTENT", "ping").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Cannot connect") || err.contains("refused") || err.contains("denied"));
    }

    #[tokio::test]
    async fn test_ensure_daemon_when_alive() {
        let name = test_socket_name("ensure");
        let ns_name = name.as_str().to_ns_name::<GenericNamespaced>().unwrap();
        let listener = ListenerOptions::new().name(ns_name).create_tokio().unwrap();
        let _handle = tokio::spawn(async move {
            let socket = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut reader, mut writer) = socket.split();
            let mut buf = [0u8; 1024];
            let _ = reader.read(&mut buf).await;
            let _ = writer.write_all(b"pong\n").await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let result = ensure_daemon(&name).await;
        assert!(result.is_ok());
    }

    // ── should_skip_daemon_check ─────────────────────

    #[test]
    fn test_skip_daemon_check_workspace_subcommand() {
        let cli = Cli::parse_from(["speedy-cli", "workspace", "list"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_subcommand() {
        let cli = Cli::parse_from(["speedy-cli", "daemon", "ping"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_no_command() {
        let cli = Cli::parse_from(["speedy-cli"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_index() {
        let cli = Cli::parse_from(["speedy-cli", "index"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_query() {
        let cli = Cli::parse_from(["speedy-cli", "query", "x"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_context() {
        let cli = Cli::parse_from(["speedy-cli", "context"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_sync() {
        let cli = Cli::parse_from(["speedy-cli", "sync"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_force() {
        let cli = Cli::parse_from(["speedy-cli", "force"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    // ── resolve_socket_name ──────────────────────────

    #[test]
    fn test_resolve_socket_name_uses_cli_when_set() {
        let cli = Cli::parse_from(["speedy-cli", "--daemon-socket", "explicit-sock", "context"]);
        assert_eq!(resolve_socket_name(&cli), "explicit-sock");
    }

    #[test]
    fn test_resolve_socket_name_uses_default_when_unset() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SPEEDY_DEFAULT_SOCKET").ok();
        std::env::remove_var("SPEEDY_DEFAULT_SOCKET");

        let cli = Cli::parse_from(["speedy-cli", "context"]);
        assert_eq!(resolve_socket_name(&cli), "speedy-daemon");

        if let Some(v) = prev { std::env::set_var("SPEEDY_DEFAULT_SOCKET", v); }
    }

    // ── send_raw_cmd: error paths ────────────────────

    #[tokio::test]
    async fn test_send_raw_cmd_invalid_socket_name_errors() {
        // Empty string is not a valid namespace name on Windows or Linux.
        let result = send_raw_cmd("", "ping").await;
        assert!(result.is_err(), "empty socket name must error");
    }

    #[tokio::test]
    async fn test_send_raw_cmd_handles_eof_response() {
        let name = test_socket_name("eof");
        let ns_name = name.as_str().to_ns_name::<GenericNamespaced>().unwrap();
        let listener = ListenerOptions::new().name(ns_name).create_tokio().unwrap();
        let _handle = tokio::spawn(async move {
            if let Ok(socket) = listener.accept().await {
                // Close immediately without writing a response.
                drop(socket);
            }
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let resp = send_raw_cmd(&name, "ping").await.unwrap_or_default();
        // read_line on an empty stream returns 0 bytes → empty string after trim.
        assert!(resp.is_empty(), "expected empty response on EOF, got: {resp:?}");
    }
}

fn should_skip_daemon_check(cli: &Cli) -> bool {
    match &cli.command {
        Some(Commands::Daemon { .. }) | Some(Commands::Workspace { .. }) => true,
        None => true,
        _ => false,
    }
}

async fn async_main(cli: Cli) -> Result<()> {
    let socket = resolve_socket_name(&cli);
    if !should_skip_daemon_check(&cli) {
        ensure_daemon(&socket).await?;
    }
    let client = DaemonClient::new(&socket);

    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy().to_string();

    let json = cli.json;
    let exec_cmd = |args: &[&str]| -> String {
        let mut s = String::from("exec\t");
        s.push_str(&cwd_str);
        if json {
            s.push('\t');
            s.push_str("--json");
        }
        for a in args {
            s.push('\t');
            s.push_str(a);
        }
        s
    };

    match &cli.command {
        Some(Commands::Index { subdir }) => {
            let resp = send_raw_cmd(&socket, &exec_cmd(&["index", subdir])).await?;
            println!("{resp}");
        }
        Some(Commands::Query { query, top_k, all }) => {
            if *all {
                let aggregated = client.query_all(query, *top_k).await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&aggregated)?);
                } else if let Some(items) = aggregated.as_array() {
                    if items.is_empty() {
                        println!("No matches across registered workspaces.");
                    } else {
                        for item in items {
                            let score = item.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let path = item.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                            let line = item.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                            let ws = item.get("workspace").and_then(|v| v.as_str()).unwrap_or("?");
                            let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                            println!("[score={score:.4}] [{ws}] {path}:{line}");
                            println!("  {text}");
                            println!();
                        }
                    }
                } else {
                    println!("{aggregated}");
                }
            } else {
                let k = top_k.to_string();
                let resp = send_raw_cmd(&socket, &exec_cmd(&["query", query, "-k", &k])).await?;
                println!("{resp}");
            }
        }
        Some(Commands::Context) => {
            let resp = send_raw_cmd(&socket, &exec_cmd(&["context"])).await?;
            println!("{resp}");
        }
        Some(Commands::Sync) => {
            let resp = send_raw_cmd(&socket, &exec_cmd(&["sync"])).await?;
            println!("{resp}");
        }
        Some(Commands::Reembed) => {
            let resp = send_raw_cmd(&socket, &exec_cmd(&["reembed"])).await?;
            println!("{resp}");
        }
        Some(Commands::Force { path }) => {
            let target = path.clone().unwrap_or_else(|| cwd_str.clone());
            let resp = send_raw_cmd(&socket, &format!("sync {target}")).await?;
            println!("{resp}");
        }
        Some(Commands::Daemon { action }) => match action {
            DaemonAction::Status => {
                let s = client.status().await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&s)?);
                } else {
                    println!("PID: {}", s.pid);
                    println!("Uptime: {}s", s.uptime_secs);
                    println!("Workspaces: {}", s.workspace_count);
                    println!("Watchers: {}", s.watcher_count);
                    println!("Version: {}", s.version);
                }
            }
            DaemonAction::List => {
                let list = client.get_all_workspaces().await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else if list.is_empty() {
                    println!("No daemon workspaces.");
                } else {
                    for ws in &list {
                        println!("[active] {ws}");
                    }
                }
            }
            DaemonAction::Stop => {
                client.stop().await?;
                if cli.json {
                    println!("{}", serde_json::json!({ "stopped": true }));
                } else {
                    println!("Daemon stopped.");
                }
            }
            DaemonAction::Ping => {
                let resp = client.ping().await?;
                if cli.json {
                    println!("{}", serde_json::json!({ "response": resp }));
                } else {
                    println!("{resp}");
                }
            }
        },
        Some(Commands::Workspace { action }) => match action {
            WorkspaceAction::List => {
                let workspaces = speedy_core::workspace::list()?;
                if cli.json {
                    let paths: Vec<&String> = workspaces.iter().map(|w| &w.path).collect();
                    println!("{}", serde_json::to_string_pretty(&paths)?);
                } else if workspaces.is_empty() {
                    println!("No workspaces found.");
                } else {
                    for ws in &workspaces {
                        println!("{}", ws.path);
                    }
                }
            }
            WorkspaceAction::Add { path } => {
                client.add_workspace(path).await?;
                if cli.json {
                    println!("{}", serde_json::json!({ "added": true, "path": path }));
                } else {
                    println!("Workspace added: {path}");
                }
            }
            WorkspaceAction::Remove { path } => {
                client.remove_workspace(path).await?;
                if cli.json {
                    println!("{}", serde_json::json!({ "removed": true, "path": path }));
                } else {
                    println!("Workspace removed: {path}");
                }
            }
        },
        None => {
            anyhow::bail!("No command specified. Use --help for usage.");
        }
    }
    Ok(())
}
