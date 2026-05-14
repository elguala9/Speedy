use clap::Parser;
use speedy_core::daemon_client::DaemonClient;
use speedy_core::daemon_util;
use speedy_core::workspace;
use crate::cli::{Cli, Commands, WorkspaceAction};
use anyhow::Result;
use tracing::{info, warn};

mod cli;
mod db;
mod document;
mod embed;
mod file;
mod hash;
mod ignore;
mod indexer;
mod text;

fn resolve_path(path: &Option<String>) -> Result<std::path::PathBuf> {
    match path {
        Some(p) => Ok(std::path::PathBuf::from(p).canonicalize()?),
        None => Ok(std::env::current_dir()?),
    }
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

fn resolve_socket_name(cli: &Cli) -> String {
    cli.daemon_socket.clone()
        .unwrap_or_else(daemon_util::default_daemon_socket_name)
}

async fn ensure_daemon(cli: &Cli) -> Result<()> {
    if std::env::var_os("SPEEDY_NO_DAEMON").is_some() {
        return Ok(());
    }
    if should_skip_daemon_check(cli) {
        return Ok(());
    }

    let cwd = std::env::current_dir()?.canonicalize()?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let socket = resolve_socket_name(cli);
    let client = DaemonClient::new(&socket);

    if client.is_alive().await {
        let workspaces = client.get_all_workspaces().await?;
        if workspaces.iter().any(|w| {
            std::path::Path::new(w).canonicalize().ok().as_ref() == Some(&cwd)
        }) {
            return Ok(());
        }

        if !workspace::is_registered(&cwd_str) {
            workspace::add(&cwd_str)?;
        }
        client.add_workspace(&cwd_str).await?;
        info!("Workspace aggiunto al daemon: {cwd_str}");
        return Ok(());
    }

    warn!("Daemon non risponde. Avvio...");
    let daemon_dir = daemon_util::daemon_dir_path()?;
    daemon_util::kill_existing_daemon(&daemon_dir);
    daemon_util::spawn_daemon_process(&socket)?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    if !workspace::is_registered(&cwd_str) {
        workspace::add(&cwd_str)?;
    }

    let client = DaemonClient::new(&socket);
    client.add_workspace(&cwd_str).await?;
    info!("Daemon avviato, workspace monitorato e indicizzato: {cwd_str}");
    Ok(())
}

fn should_skip_daemon_check(cli: &Cli) -> bool {
    if cli.workspaces || cli.daemons {
        return true;
    }

    if let Some(cmd) = &cli.command {
        return matches!(cmd,
            Commands::Daemon
            | Commands::Workspace { .. }
        );
    }

    cli.command.is_none()
}

async fn async_main(cli: Cli) -> Result<()> {
    ensure_daemon(&cli).await?;

    let config = speedy_core::config::Config::load();

    if cli.workspaces {
        let workspaces = workspace::list()?;
        if workspaces.is_empty() {
            println!("No workspaces found.");
        } else {
            for ws in &workspaces {
                println!("{}", ws.path);
            }
        }
        return Ok(());
    }

    if cli.daemons {
        let client = DaemonClient::new(resolve_socket_name(&cli));
        if client.is_alive().await {
            let list = client.get_all_workspaces().await?;
            if list.is_empty() {
                println!("No daemon workspaces.");
            } else {
                for ws in &list {
                    println!("[active] {ws}");
                }
            }
        } else {
            println!("Daemon not running.");
        }
        return Ok(());
    }

    if let Some(prompt) = cli.read {
        let indexer = crate::indexer::Indexer::new(&config).await?;
        let results = indexer.query(&prompt, 5).await?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results)?);
        } else {
            for r in &results {
                println!("[score={:.4}] {}:{}", r.score, r.path, r.line);
                println!("  {}", r.text);
                println!();
            }
        }
        return Ok(());
    }

    if let Some(content) = cli.modify {
        if let Some(file) = cli.file {
            tokio::fs::write(&file, &content).await?;
            let indexer = crate::indexer::Indexer::new(&config).await?;
            let chunks = indexer.index_file(&file).await?;
            if cli.json {
                let output = serde_json::json!({"file": file, "chunks": chunks});
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Written to {file}: {chunks} chunks indexed");
            }
        } else {
            println!("Workspace modification requested: {content}");
            println!("Use --file <file> to target a specific file.");
        }
        return Ok(());
    }

    match &cli.command {
        Some(Commands::Index { subdir }) => {
            let indexer = crate::indexer::Indexer::new(&config).await?;
            let stats = indexer.index_directory(subdir).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!("Indexed {} files, {} chunks", stats.files, stats.chunks);
            }
        }
        Some(Commands::Query { query, top_k }) => {
            let indexer = crate::indexer::Indexer::new(&config).await?;
            let k = top_k.unwrap_or(5);
            let results = indexer.query(query, k).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for r in &results {
                    println!("[score={:.4}] {}:{}", r.score, r.path, r.line);
                    println!("  {}", r.text);
                    println!();
                }
            }
        }
        Some(Commands::Context) => {
            let indexer = crate::indexer::Indexer::new(&config).await?;
            let ctx = indexer.project_context().await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&ctx)?);
            } else {
                println!("Project: {}", ctx.root);
                println!("Files indexed: {}", ctx.file_count);
                println!("Total chunks: {}", ctx.chunk_count);
                println!("Last indexed: {}", ctx.last_indexed);
            }
        }
        Some(Commands::Sync) => {
            let indexer = crate::indexer::Indexer::new(&config).await?;
            let stats = indexer.sync_all().await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!(
                    "Synced: {} added, {} updated, {} removed",
                    stats.files, stats.chunks, stats.removed
                );
            }
        }
        Some(Commands::Daemon) => {
            let socket = resolve_socket_name(&cli);
            daemon_util::spawn_daemon_process(&socket)?;
            println!("Daemon started on socket {socket}");
        }
        Some(Commands::Workspace { action }) => match action {
            WorkspaceAction::List => {
                let workspaces = workspace::list()?;
                if workspaces.is_empty() {
                    println!("No workspaces found.");
                } else {
                    for ws in &workspaces {
                        println!("{}", ws.path);
                    }
                }
            }
        },
        None => {
            anyhow::bail!("No command specified. Use --help for usage.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use crate::cli::Commands;
    use speedy_core::local_sock::{GenericNamespaced, ListenerOptions, ListenerTrait as _, StreamTrait as _, ToNsName};

    #[test]
    fn test_cli_assert() {
        Cli::command().debug_assert();
    }

    fn make_cli(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    #[test]
    fn test_skip_daemon_check_workspaces_flag() {
        let cli = make_cli(&["speedy", "--workspaces"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemons_flag() {
        let cli = make_cli(&["speedy", "--daemons"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_subcommand() {
        let cli = make_cli(&["speedy", "daemon"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_workspace_subcommand() {
        let cli = make_cli(&["speedy", "workspace", "list"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_no_command() {
        let cli = make_cli(&["speedy"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_index() {
        let cli = make_cli(&["speedy", "index"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_query() {
        let cli = make_cli(&["speedy", "query", "test"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_sync() {
        let cli = make_cli(&["speedy", "sync"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_context() {
        let cli = make_cli(&["speedy", "context"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_resolve_path_with_some() {
        let dir = std::env::temp_dir().join("speedy_test_resolve");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = resolve_path(&Some(dir.to_string_lossy().to_string())).unwrap();
        assert!(result.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_path_with_none_returns_current_dir() {
        let result = resolve_path(&None).unwrap();
        assert_eq!(result, std::env::current_dir().unwrap());
    }

    #[test]
    fn test_resolve_path_with_nonexistent_returns_error() {
        let result = resolve_path(&Some("/nonexistent-path-12345".to_string()));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ensure_daemon_daemon_alive_skip_spawn() {
        let socket_name = "speedy_test_ensure_daemon_alive".to_string();
        let dir = std::env::temp_dir().join("speedy_test_ensure_daemon");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).ok();

        let name = socket_name.as_str().to_ns_name::<GenericNamespaced>().unwrap();
        let listener = ListenerOptions::new().name(name).create_tokio().unwrap();
        let _handle = tokio::spawn(async move {
            loop {
                let socket = listener.accept().await.unwrap();
                use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                let (reader, mut writer) = socket.split();
                let mut buf_reader = tokio::io::BufReader::new(reader);
                let mut line = String::new();
                if buf_reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    continue;
                }
                let resp = match line.trim() {
                    "ping" => "pong",
                    "list" => "[]",
                    "is-workspace" => "false",
                    _ => "ok",
                };
                let _ = writer.write_all(format!("{resp}\n").as_bytes()).await;
            }
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut cli = make_cli(&["speedy", "index"]);
        cli.daemon_socket = Some(socket_name);
        let result = ensure_daemon(&cli).await;
        assert!(result.is_ok(), "ensure_daemon failed: {:?}", result.err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    static ENSURE_DAEMON_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn locate_built_bin(bin: &str) -> Option<std::path::PathBuf> {
        let exe = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
        let p = workspace_root().join("target").join("debug").join(exe);
        if p.exists() { Some(p) } else { None }
    }

    #[tokio::test]
    async fn test_ensure_daemon_spawns_when_daemon_dead() {
        let _lock = ENSURE_DAEMON_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let socket_name = format!(
            "speedy_test_spawn_success_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("speedy_test_spawn_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let daemon_dir = root.join("daemon");
        std::fs::create_dir_all(&daemon_dir).unwrap();
        std::fs::write(daemon_dir.join("daemon.pid"), "99999").unwrap();

        let workdir = root.join("workdir");
        std::fs::create_dir_all(&workdir).unwrap();

        if locate_built_bin("speedy-daemon").is_none() {
            eprintln!(
                "skipping test_ensure_daemon_spawns_when_daemon_dead: \
                 speedy-daemon binary not found at target/debug — \
                 build it first with `cargo build -p speedy-daemon`"
            );
            return;
        }

        let prev_cwd = std::env::current_dir().ok();
        let prev_daemon_dir = std::env::var_os("SPEEDY_DAEMON_DIR");
        std::env::set_current_dir(&workdir).unwrap();
        std::env::set_var("SPEEDY_DAEMON_DIR", &daemon_dir);

        let mut cli = make_cli(&["speedy", "index"]);
        cli.daemon_socket = Some(socket_name.clone());

        let result = ensure_daemon(&cli).await;

        let client = speedy_core::daemon_client::DaemonClient::new(&socket_name);
        let _ = client.stop().await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        if let Some(prev) = prev_cwd {
            let _ = std::env::set_current_dir(prev);
        }
        match prev_daemon_dir {
            Some(v) => std::env::set_var("SPEEDY_DAEMON_DIR", v),
            None => std::env::remove_var("SPEEDY_DAEMON_DIR"),
        }
        let _ = std::fs::remove_dir_all(&root);

        assert!(result.is_ok(), "ensure_daemon should spawn and connect: {:?}", result.err());
    }
}
