use clap::Parser;
use speedy_core::daemon_client::DaemonClient;
use speedy_core::daemon_util;
use speedy_core::workspace;
use crate::cli::{Cli, Commands, DaemonAction, WorkspaceAction};
use anyhow::Result;
use tracing::{info, warn};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

mod cli;
mod daemon;
mod db;
mod document;
mod embed;
mod file;
mod hash;
mod ignore;
mod indexer;
mod text;
mod watcher;

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

    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--as-service") {
            let service_name = args.iter()
                .position(|a| a == "--service-name")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .expect("--service-name is required with --as-service");
            return daemon::run_as_windows_service(&service_name);
        }
    }

    let cli = Cli::parse();

    if let Some(ref p) = cli.project_path {
        std::env::set_current_dir(p)?;
    }

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main(cli))
}

async fn ensure_daemon(cli: &Cli) -> Result<()> {
    if should_skip_daemon_check(cli) {
        return Ok(());
    }

    let cwd = std::env::current_dir()?.canonicalize()?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let port = cli.daemon_port;
    let client = DaemonClient::new(port);

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
    daemon_util::spawn_daemon_process(port)?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    if !workspace::is_registered(&cwd_str) {
        workspace::add(&cwd_str)?;
    }

    let client = DaemonClient::new(port);
    client.add_workspace(&cwd_str).await?;
    info!("Daemon avviato, workspace monitorato e indicizzato: {cwd_str}");
    Ok(())
}

fn should_skip_daemon_check(cli: &Cli) -> bool {
    if cli.workspaces {
        return true;
    }

    if cli.daemons
        || cli.daemon_stop.is_some()
        || cli.daemon_restart.is_some()
        || cli.daemon_delete.is_some()
        || cli.daemon_create.is_some()
        || cli.daemon_status.is_some()
    {
        return true;
    }

    if cli.workspace_create.is_some() || cli.workspace_delete.is_some() {
        return true;
    }

    if let Some(cmd) = &cli.command {
        return matches!(cmd,
            Commands::Daemon { .. }
            | Commands::Workspace { .. }
        );
    }

    if cli.command.is_none() {
        return true;
    }

    false
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
        let client = DaemonClient::new(cli.daemon_port);
        if client.is_alive().await {
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
                Err(_) => {
                    daemon::list_all_daemons().await?;
                }
            }
        } else {
            daemon::list_all_daemons().await?;
        }
        return Ok(());
    }

    if cli.daemon_stop.is_some() {
        let client = DaemonClient::new(cli.daemon_port);
        client.stop().await?;
        println!("Daemon stopped.");
        return Ok(());
    }

    if let Some(path) = cli.daemon_restart {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        return daemon::restart_daemon(&root).await;
    }

    if let Some(path) = cli.daemon_delete {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        return daemon::delete_daemon(&root).await;
    }

    if let Some(path) = cli.daemon_create {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        return daemon::create_daemon(&root).await;
    }

    if let Some(path) = cli.daemon_status {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        let original = std::env::current_dir()?;
        std::env::set_current_dir(&root)?;
        let result = daemon::status_cmd().await;
        std::env::set_current_dir(original)?;
        return result;
    }

    if let Some(path) = cli.force {
        let root = resolve_path(&Some(path))?;
        return daemon::force_scan(&root).await;
    }

    if let Some(path) = cli.workspace_create {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        let path_str = root.to_string_lossy().to_string();
        workspace::add(&path_str)?;
        let client = DaemonClient::new(cli.daemon_port);
        if client.is_alive().await {
            let _ = client.add_workspace(&path_str).await;
        }
        println!("Workspace created: {}", path_str);
        return Ok(());
    }

    if let Some(path) = cli.workspace_delete {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        let path_str = root.to_string_lossy().to_string();
        let client = DaemonClient::new(cli.daemon_port);
        if client.is_alive().await {
            let _ = client.remove_workspace(&path_str).await;
        }
        workspace::remove(&path_str)?;
        println!("Workspace deleted: {}", path_str);
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
        Some(Commands::Watch { subdir, detach }) => {
            if *detach {
                let port_flag = format!("{}", cli.daemon_port);
                #[cfg(windows)]
                {
                    let exe = std::env::current_exe()?;
                    let cwd = std::env::current_dir()?;
                    let child = std::process::Command::new(&exe)
                        .args(["--daemon-port", &port_flag, "-p", &cwd.to_string_lossy(), "watch", subdir])
                        .creation_flags(0x08000000)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()?;
                    println!("Watcher detached (PID: {}).", child.id());
                }
                #[cfg(not(windows))]
                {
                    let exe = std::env::current_exe()?;
                    let cwd = std::env::current_dir()?;
                    let child = std::process::Command::new(&exe)
                        .args(["--daemon-port", &port_flag, "-p", &cwd.to_string_lossy(), "watch", subdir])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()?;
                    println!("Watcher detached (PID: {}).", child.id());
                }
            } else {
                println!("Watching {subdir}... (run with --detach for background)");
                watcher::start_watcher(subdir, &config).await?;
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
        Some(Commands::Daemon { action: None }) => {
            daemon_util::spawn_daemon_process(cli.daemon_port)?;
            println!("Daemon started on port {}", cli.daemon_port);
        }
        Some(Commands::Daemon { action: Some(action) }) => match action {
            DaemonAction::Install { path } => {
                daemon::install(path.clone()).await?;
            }
            DaemonAction::Uninstall => {
                daemon::uninstall().await?;
            }
            DaemonAction::Status => {
                daemon::status_cmd().await?;
            }
            DaemonAction::List => {
                daemon::list_all_daemons().await?;
            }
            DaemonAction::Stop { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                daemon::stop_daemon(&root).await?;
            }
            DaemonAction::Restart { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                daemon::restart_daemon(&root).await?;
            }
            DaemonAction::Delete { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                daemon::delete_daemon(&root).await?;
            }
            DaemonAction::Create { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                daemon::create_daemon(&root).await?;
            }
        },
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
            WorkspaceAction::Create { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                let path_str = root.to_string_lossy().to_string();
                workspace::add(&path_str)?;
                let client = DaemonClient::new(cli.daemon_port);
                if client.is_alive().await {
                    let _ = client.add_workspace(&path_str).await;
                }
                println!("Workspace created: {}", path_str);
            }
            WorkspaceAction::Delete { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                let path_str = root.to_string_lossy().to_string();
                let client = DaemonClient::new(cli.daemon_port);
                if client.is_alive().await {
                    let _ = client.remove_workspace(&path_str).await;
                }
                workspace::remove(&path_str)?;
                println!("Workspace deleted: {}", path_str);
            }
        },
        Some(Commands::Force { path }) => {
            let root = resolve_path(path)?;
            daemon::force_scan(&root).await?;
        }
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

    #[test]
    fn test_cli_assert() {
        Cli::command().debug_assert();
    }

    // ── should_skip_daemon_check tests ──────────────

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
    fn test_skip_daemon_check_daemon_stop() {
        let cli = make_cli(&["speedy", "--daemon-stop", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_restart() {
        let cli = make_cli(&["speedy", "--daemon-restart", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_delete() {
        let cli = make_cli(&["speedy", "--daemon-delete", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_create() {
        let cli = make_cli(&["speedy", "--daemon-create", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_status() {
        let cli = make_cli(&["speedy", "--daemon-status", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_workspace_create() {
        let cli = make_cli(&["speedy", "--workspace-create", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_workspace_delete() {
        let cli = make_cli(&["speedy", "--workspace-delete", "/tmp"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_daemon_subcommand() {
        let cli = make_cli(&["speedy", "daemon", "list"]);
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
    fn test_skip_daemon_check_does_not_skip_watch() {
        let cli = make_cli(&["speedy", "watch"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_does_not_skip_force() {
        let cli = make_cli(&["speedy", "force", "-p", "/tmp"]);
        assert!(!should_skip_daemon_check(&cli));
    }

    // ── resolve_path tests ──────────────────────────

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

    // ── ensure_daemon test ──────────────────────────

    #[tokio::test]
    async fn test_ensure_daemon_daemon_alive_skip_spawn() {
        let port = 42601;
        let dir = std::env::temp_dir().join("speedy_test_ensure_daemon");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).ok();

        // Mock daemon that responds to IPC commands
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
        let _handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                let (reader, mut writer) = socket.split();
                let mut buf_reader = tokio::io::BufReader::new(reader);
                let mut line = String::new();
                // Read one command, respond, then close (just like real daemon)
                if buf_reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    continue; // EOF means client just checked alive, keep listening
                }
                let resp = match line.trim() {
                    "list" => "[]",
                    "is-workspace" => "false",
                    _ => "ok",
                };
                let _ = writer.write_all(format!("{resp}\n").as_bytes()).await;
            }
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut cli = make_cli(&["speedy", "index"]);
        cli.daemon_port = port;
        let result = ensure_daemon(&cli).await;
        assert!(result.is_ok(), "ensure_daemon failed: {:?}", result.err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Daemon { action: None } parsing test ───────

    #[test]
    fn test_daemon_action_none() {
        let cli = make_cli(&["speedy", "daemon"]);
        assert!(matches!(cli.command, Some(Commands::Daemon { action: None })));
    }

    #[tokio::test]
    async fn test_ensure_daemon_daemon_dead_spawn_fails() {
        let port = 42989;
        let dir = std::env::temp_dir().join("speedy_test_ensure_dead");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).ok();

        let daemon_dir = daemon_util::daemon_dir_path().unwrap();
        let pid_path = daemon_dir.join("daemon.pid");
        let pid_backup = std::fs::read_to_string(&pid_path).ok();
        let _ = std::fs::remove_file(&pid_path);

        let mut cli = make_cli(&["speedy", "index"]);
        cli.daemon_port = port;

        let result = ensure_daemon(&cli).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("daemon executable"), "expected daemon error, got: {err}");

        if let Some(content) = pid_backup {
            let _ = std::fs::write(&pid_path, content);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
