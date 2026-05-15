use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};

use speedy_language_context::cli::{Cli, Commands};
use speedy_language_context::graph::GraphStore;
use speedy_language_context::indexer::Indexer;
use speedy_language_context::{mcp, search, skeleton};

fn main() -> Result<()> {
    // tracing → stderr by default; stdout is reserved for MCP traffic.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let root = resolve_root(&cli.workspace_path)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main(cli, root))
}

fn resolve_root(p: &Option<PathBuf>) -> Result<PathBuf> {
    let raw = match p {
        Some(p) => p.clone(),
        None => std::env::current_dir().context("getting current dir")?,
    };
    raw.canonicalize().or_else(|_| Ok(raw))
}

async fn async_main(cli: Cli, root: PathBuf) -> Result<()> {
    match cli.command {
        Commands::Index => cmd_index(&root, cli.json).await,
        Commands::Update { files } => cmd_update(&root, &files, cli.json).await,
        Commands::Status => cmd_status(&root, cli.json),
        Commands::Serve => mcp::run_server(root).await,
        Commands::Skeleton { files, detail } => cmd_skeleton(&root, &files, &detail),
        Commands::Search { query, top_k } => cmd_search(&root, &query, top_k, cli.json),
    }
}

async fn cmd_index(root: &Path, as_json: bool) -> Result<()> {
    let indexer = Indexer::new(root)?;
    let stats = indexer.full_index().await?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!(
            "indexed {} files, skipped {}, {} symbols, {} ms",
            stats.files_indexed, stats.files_skipped, stats.symbols_found, stats.duration_ms
        );
    }
    Ok(())
}

async fn cmd_update(root: &Path, files: &[PathBuf], as_json: bool) -> Result<()> {
    let indexer = Indexer::new(root)?;
    let stats = indexer.index_files(files).await?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!(
            "updated {} files, skipped {}, {} symbols, {} ms",
            stats.files_indexed, stats.files_skipped, stats.symbols_found, stats.duration_ms
        );
    }
    Ok(())
}

fn cmd_status(root: &Path, as_json: bool) -> Result<()> {
    let store = GraphStore::open(root)?;
    let files = store.file_count()?;
    let symbols = store.symbol_count()?;
    let edges = store.edge_count()?;
    let last_indexed = store.get_meta("last_indexed_at")?.unwrap_or_else(|| "never".to_string());
    if as_json {
        let v = serde_json::json!({
            "files": files,
            "symbols": symbols,
            "edges": edges,
            "last_indexed": last_indexed,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("files:        {files}");
        println!("symbols:      {symbols}");
        println!("edges:        {edges}");
        println!("last indexed: {last_indexed}");
    }
    Ok(())
}

fn cmd_skeleton(root: &Path, files: &[String], detail: &str) -> Result<()> {
    let store = GraphStore::open(root)?;
    let detail = detail.parse()?;
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let out = skeleton::get_skeleton(&store, root, &refs, detail)?;
    println!("{out}");
    Ok(())
}

fn cmd_search(root: &Path, query: &str, top_k: usize, as_json: bool) -> Result<()> {
    let store = GraphStore::open(root)?;
    let results = search::search(&store, query, top_k)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for r in &results {
            println!(
                "{:>6.1}  [{}] {}::{} (line {}) — {}",
                r.score,
                r.kind,
                r.file,
                r.symbol_name,
                r.start_line + 1,
                r.signature.replace('\n', " ")
            );
        }
        if results.is_empty() {
            println!("no matches");
        }
    }
    Ok(())
}
