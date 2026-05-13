use clap::Parser;
use speedy::cli::{Cli, Commands, DaemonAction, WorkspaceAction};
use speedy::daemon;
use speedy::indexer;
use speedy::watcher;
use speedy::workspace;
use anyhow::Result;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn resolve_path(path: &Option<String>) -> Result<std::path::PathBuf> {
    match path {
        Some(p) => Ok(std::path::PathBuf::from(p).canonicalize()?),
        None => Ok(std::env::current_dir()?),
    }
}

fn main() -> Result<()> {
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

async fn async_main(cli: Cli) -> Result<()> {
    let config = speedy::config::Config::load();

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
        daemon::list_all_daemons().await?;
        return Ok(());
    }

    if let Some(path) = cli.daemon_stop {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        return daemon::stop_daemon(&root).await;
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
        daemon::create_daemon(&root).await?;
        println!("Workspace created: {}", path_str);
        return Ok(());
    }

    if let Some(path) = cli.workspace_delete {
        let root = std::path::PathBuf::from(path).canonicalize()?;
        let path_str = root.to_string_lossy().to_string();
        daemon::delete_daemon(&root).await?;
        workspace::remove(&path_str)?;
        println!("Workspace deleted: {}", path_str);
        return Ok(());
    }

    if let Some(prompt) = cli.read {
        let indexer = indexer::Indexer::new(&config).await?;
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
            let indexer = indexer::Indexer::new(&config).await?;
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
            let indexer = indexer::Indexer::new(&config).await?;
            let stats = indexer.index_directory(subdir).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!("Indexed {} files, {} chunks", stats.files, stats.chunks);
            }
        }
        Some(Commands::Query { query, top_k }) => {
            let indexer = indexer::Indexer::new(&config).await?;
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
                #[cfg(windows)]
                {
                    let exe = std::env::current_exe()?;
                    let cwd = std::env::current_dir()?;
                    let child = std::process::Command::new(&exe)
                        .args(["-p", &cwd.to_string_lossy(), "watch", subdir])
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
                        .args(["-p", &cwd.to_string_lossy(), "watch", subdir])
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
            let indexer = indexer::Indexer::new(&config).await?;
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
            let indexer = indexer::Indexer::new(&config).await?;
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
        Some(Commands::Daemon { action }) => match action {
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
                daemon::create_daemon(&root).await?;
                println!("Workspace created: {}", path_str);
            }
            WorkspaceAction::Delete { path } => {
                let root = std::path::PathBuf::from(path).canonicalize()?;
                let path_str = root.to_string_lossy().to_string();
                daemon::delete_daemon(&root).await?;
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
