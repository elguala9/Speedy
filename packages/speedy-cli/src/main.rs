use clap::{Parser, Subcommand};
use speedy_ai_context::db::VectorStore as _;
use speedy_core::daemon_client::DaemonClient;
use speedy_core::daemon_util;
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

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
    #[command(about = "Full reindex across all enabled contexts (ai/language/text)")]
    Reindex {
        #[arg(short = 'p', help = "Workspace path (default: current dir)")]
        path: Option<String>,
    },
    #[command(about = "Keyword search over indexed files (no embedding model required)")]
    Grep {
        #[arg(help = "FTS5 search pattern (e.g. 'fn authenticate', '\"error handling\"', 'fn*')")]
        pattern: String,
        #[arg(short = 'k', long = "top-k", default_value = "20")]
        top_k: usize,
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
    reader.read_to_string(&mut resp).await?;
    Ok(resp.trim().to_string())
}

/// Run the `speedy-ai-context` worker in-process (standalone — no daemon).
/// Stdio is inherited so the worker's own (human or `--json`) output reaches
/// the user directly, and the worker applies its own feature gating.
async fn run_ai_context_standalone(cwd: &str, json: bool, args: &[&str]) -> Result<()> {
    let exe = speedy_core::contexts::find_ai_context_exe();
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("-p").arg(cwd);
    if json {
        cmd.arg("--json");
    }
    for a in args {
        cmd.arg(a);
    }
    cmd.env("SPEEDY_NO_DAEMON", "1");
    let status = cmd
        .status()
        .await
        .with_context(|| format!("failed to spawn {}", exe.display()))?;
    if !status.success() {
        anyhow::bail!("speedy-ai-context exited with status {status}");
    }
    Ok(())
}

/// Cross-workspace semantic query without a daemon: replicate the daemon's
/// `query-all` fan-out by running the ai-context worker once per registered
/// workspace, tagging each hit with its workspace, then merging and ranking.
async fn standalone_query_all(query: &str, top_k: usize) -> Result<serde_json::Value> {
    let workspaces = speedy_core::workspace::list()?;
    let exe = speedy_core::contexts::find_ai_context_exe();
    let k = top_k.to_string();
    let mut merged: Vec<serde_json::Value> = Vec::new();
    for ws in workspaces {
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.args(["-p", &ws.path, "query", query, "-k", &k, "--json"])
            .env("SPEEDY_NO_DAEMON", "1");
        let output = match cmd.output().await {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(stdout.trim()).unwrap_or_default();
        for mut item in parsed {
            if let Some(obj) = item.as_object_mut() {
                obj.insert(
                    "workspace".to_string(),
                    serde_json::Value::String(ws.path.clone()),
                );
            }
            merged.push(item);
        }
    }
    merged.sort_by(|a, b| {
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    merged.truncate(top_k);
    Ok(serde_json::Value::Array(merged))
}

fn main() -> Result<()> {
    use tracing_subscriber::prelude::*;
    let logs_dir = speedy_core::daemon_util::exe_log_dir();
    let file_appender = tracing_appender::rolling::daily(&logs_dir, "speedy-cli.log");
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(false).with_writer(std::io::stderr))
        .with(tracing_subscriber::fmt::layer().with_target(true).with_writer(file_writer))
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
    use speedy_core::local_sock::{GenericNamespaced, ListenerOptions, ListenerTrait as _, ToNsName};

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

    #[test]
    fn test_parse_reindex() {
        let cli = Cli::parse_from(["speedy-cli", "reindex"]);
        assert!(matches!(cli.command, Some(Commands::Reindex { path: None })));
    }

    #[test]
    fn test_parse_reindex_with_path() {
        let cli = Cli::parse_from(["speedy-cli", "reindex", "-p", "/tmp/proj"]);
        if let Some(Commands::Reindex { path }) = cli.command {
            assert_eq!(path, Some("/tmp/proj".to_string()));
        } else {
            panic!("expected Reindex command");
        }
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
        // read_to_string on an empty stream returns 0 bytes → empty string after trim.
        assert!(resp.is_empty(), "expected empty response on EOF, got: {resp:?}");
    }
}

async fn async_main(cli: Cli) -> Result<()> {
    let socket = resolve_socket_name(&cli);
    let client = DaemonClient::new(&socket);

    // Probe the daemon once. When it is up we route through it (it orchestrates
    // the three context workers); when it is down we run standalone, driving the
    // workers in-process via `speedy_core::contexts`. The daemon is opt-in and
    // is never auto-started.
    let alive = client.is_alive().await;

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
            if alive {
                let resp = send_raw_cmd(&socket, &exec_cmd(&["index", subdir])).await?;
                println!("{resp}");
            } else {
                run_ai_context_standalone(&cwd_str, json, &["index", subdir]).await?;
            }
        }
        Some(Commands::Query { query, top_k, all }) => {
            if *all {
                let aggregated = if alive {
                    client.query_all(query, *top_k).await?
                } else {
                    standalone_query_all(query, *top_k).await?
                };
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
                if alive {
                    let resp = send_raw_cmd(&socket, &exec_cmd(&["query", query, "-k", &k])).await?;
                    println!("{resp}");
                } else {
                    run_ai_context_standalone(&cwd_str, json, &["query", query, "-k", &k]).await?;
                }
            }
        }
        Some(Commands::Context) => {
            if alive {
                let resp = send_raw_cmd(&socket, &exec_cmd(&["context"])).await?;
                println!("{resp}");
            } else {
                run_ai_context_standalone(&cwd_str, json, &["context"]).await?;
            }
        }
        Some(Commands::Sync) => {
            if alive {
                let resp = send_raw_cmd(&socket, &exec_cmd(&["sync"])).await?;
                println!("{resp}");
            } else {
                run_ai_context_standalone(&cwd_str, json, &["sync"]).await?;
            }
        }
        Some(Commands::Reembed) => {
            if alive {
                let resp = send_raw_cmd(&socket, &exec_cmd(&["reembed"])).await?;
                println!("{resp}");
            } else {
                run_ai_context_standalone(&cwd_str, json, &["reembed"]).await?;
            }
        }
        Some(Commands::Force { path }) => {
            let target = path.clone().unwrap_or_else(|| cwd_str.clone());
            if alive {
                let resp = send_raw_cmd(&socket, &format!("sync {target}")).await?;
                println!("{resp}");
            } else {
                run_ai_context_standalone(&target, json, &["sync"]).await?;
            }
        }
        Some(Commands::Reindex { path }) => {
            let target = path.clone().unwrap_or_else(|| cwd_str.clone());
            // Full fan-out across all enabled contexts. Daemon when up, else the
            // same orchestration in-process.
            let summary = if alive {
                client.reindex(&target).await?
            } else {
                speedy_core::contexts::reindex_workspace(&target).await?
            };
            if cli.json {
                println!("{}", serde_json::json!({ "reindex": summary }));
            } else {
                println!("{summary}");
            }
        }
        // Daemon introspection commands require a live daemon by design: when
        // it is down they fail with a connection error (callers/tests rely on
        // this). We deliberately do not synthesize a success here.
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
                if alive {
                    client.add_workspace(path).await?;
                } else {
                    // No daemon: write the registry directly. A daemon started
                    // later reloads workspaces.json from disk.
                    speedy_core::workspace::add(path)?;
                }
                if cli.json {
                    println!("{}", serde_json::json!({ "added": true, "path": path }));
                } else {
                    println!("Workspace added: {path}");
                }
            }
            WorkspaceAction::Remove { path } => {
                if alive {
                    client.remove_workspace(path).await?;
                } else {
                    speedy_core::workspace::remove(path)?;
                }
                if cli.json {
                    println!("{}", serde_json::json!({ "removed": true, "path": path }));
                } else {
                    println!("Workspace removed: {path}");
                }
            }
        },
        Some(Commands::Grep { pattern, top_k }) => {
            let db = speedy_ai_context::db::SqliteVectorStore::new(&cwd_str)
                .await
                .context("cannot open local index — run 'speedy-cli index' first")?;
            let results = db.text_search(pattern, *top_k).await
                .with_context(|| format!("text search failed for pattern: {pattern:?}"))?;
            if cli.json {
                println!("{}", serde_json::to_string(&results)?);
            } else if results.is_empty() {
                println!("No matches for: {pattern}");
            } else {
                for r in &results {
                    let snippet = &r.text[..r.text.len().min(120)];
                    println!("{}:{} — {snippet}", r.path, r.line);
                }
            }
        }
        None => {
            anyhow::bail!("No command specified. Use --help for usage.");
        }
    }
    Ok(())
}
