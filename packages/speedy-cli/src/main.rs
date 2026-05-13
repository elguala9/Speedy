use clap::{Parser, Subcommand};
use speedy_core::daemon_client::DaemonClient;
use speedy_core::daemon_util;
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::warn;

#[derive(Parser)]
#[command(name = "speedy-cli", version, about = "Local Semantic File System - Thin Client")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short = 'p', long = "path", help = "Project root (default: current dir)")]
    project_path: Option<String>,

    #[arg(long = "daemon-port", help = "Daemon TCP port (default: 42137)", default_value = "42137")]
    daemon_port: u16,

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
    },
    #[command(about = "Show project context summary")]
    Context,
    #[command(about = "Sync filesystem changes to the database incrementally")]
    Sync,
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

async fn send_raw_cmd(port: u16, req: &str) -> Result<String> {
    let addr = format!("127.0.0.1:{port}");
    let mut stream = TcpStream::connect(&addr)
        .await
        .context("Cannot connect to daemon. Is it running?")?;
    stream.write_all(format!("{req}\n").as_bytes()).await?;
    stream.shutdown().await?;

    let mut reader = BufReader::new(&mut stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).await?;
    Ok(resp.trim().to_string())
}

async fn ensure_daemon(port: u16) -> Result<()> {
    let client = DaemonClient::new(port);
    if client.is_alive().await {
        return Ok(());
    }

    warn!("Daemon non risponde. Avvio...");
    let daemon_dir = daemon_util::daemon_dir_path()?;
    daemon_util::kill_existing_daemon(&daemon_dir);
    daemon_util::spawn_daemon_process(port)?;
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
    fn test_parse_daemon_port() {
        let cli = Cli::parse_from(["speedy-cli", "--daemon-port", "42138", "context"]);
        assert_eq!(cli.daemon_port, 42138);
    }

    #[test]
    fn test_parse_daemon_port_default() {
        let cli = Cli::parse_from(["speedy-cli", "sync"]);
        assert_eq!(cli.daemon_port, 42137);
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

    // ── send_raw_cmd tests with mock TCP server ──────

    #[tokio::test]
    async fn test_send_raw_cmd_returns_response() {
        let port = 42501;
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            use tokio::io::AsyncReadExt;
            let n = socket.read(&mut buf).await.unwrap();
            let msg = String::from_utf8_lossy(&buf[..n]);
            assert_eq!(msg, "ping\n");
            use tokio::io::AsyncWriteExt;
            socket.write_all(b"pong\n").await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let resp = send_raw_cmd(port, "ping").await.unwrap();
        assert_eq!(resp, "pong");

        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_send_raw_cmd_connection_refused() {
        let port = 42502;
        let result = send_raw_cmd(port, "ping").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Cannot connect") || err.contains("refused") || err.contains("denied"));
    }

    #[tokio::test]
    async fn test_ensure_daemon_when_alive() {
        let port = 42503;
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
        let _handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let result = ensure_daemon(port).await;
        assert!(result.is_ok());
    }

    // ── Workspace integration tests ──────────────────

}

async fn async_main(cli: Cli) -> Result<()> {
    ensure_daemon(cli.daemon_port).await?;
    let client = DaemonClient::new(cli.daemon_port);

    match &cli.command {
        Some(Commands::Index { subdir }) => {
            let resp = send_raw_cmd(cli.daemon_port, &format!("exec index {subdir}")).await?;
            println!("{resp}");
        }
        Some(Commands::Query { query, top_k }) => {
            let resp = send_raw_cmd(cli.daemon_port, &format!("exec query \"{query}\" -k {top_k}")).await?;
            if cli.json {
                println!("{resp}");
            } else {
                println!("{resp}");
            }
        }
        Some(Commands::Context) => {
            let resp = send_raw_cmd(cli.daemon_port, "exec context").await?;
            println!("{resp}");
        }
        Some(Commands::Sync) => {
            let resp = send_raw_cmd(cli.daemon_port, "exec sync").await?;
            println!("{resp}");
        }
        Some(Commands::Force { path }) => {
            let target = path.as_deref().unwrap_or(".");
            let resp = send_raw_cmd(cli.daemon_port, &format!("reindex {target}")).await?;
            println!("{resp}");
        }
        Some(Commands::Daemon { action }) => match action {
            DaemonAction::Status => {
                match client.status().await {
                    Ok(s) => {
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
                    Err(e) => println!("Daemon status error: {e}"),
                }
            }
            DaemonAction::List => {
                match client.get_all_workspaces().await {
                    Ok(list) => {
                        if list.is_empty() {
                            println!("No daemon workspaces.");
                        } else {
                            for ws in &list {
                                println!("[active] {ws}");
                            }
                        }
                    }
                    Err(e) => println!("Error: {e}"),
                }
            }
            DaemonAction::Stop => {
                client.stop().await?;
                println!("Daemon stopped.");
            }
            DaemonAction::Ping => {
                match client.ping().await {
                    Ok(resp) => println!("{resp}"),
                    Err(e) => println!("Error: {e}"),
                }
            }
        },
        Some(Commands::Workspace { action }) => match action {
            WorkspaceAction::List => {
                let workspaces = speedy_core::workspace::list()?;
                if workspaces.is_empty() {
                    println!("No workspaces found.");
                } else {
                    for ws in &workspaces {
                        println!("{}", ws.path);
                    }
                }
            }
            WorkspaceAction::Add { path } => {
                client.add_workspace(path).await?;
                println!("Workspace added: {path}");
            }
            WorkspaceAction::Remove { path } => {
                client.remove_workspace(path).await?;
                println!("Workspace removed: {path}");
            }
        },
        None => {
            anyhow::bail!("No command specified. Use --help for usage.");
        }
    }
    Ok(())
}
