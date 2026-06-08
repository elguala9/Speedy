use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::info;
use tracing_subscriber::prelude::*;

use speedy_language_context::cli::{Cli, Commands};
use speedy_language_context::features::Features;
use speedy_language_context::graph::GraphStore;
use speedy_language_context::indexer::Indexer;
use speedy_language_context::{mcp, search, skeleton};

fn main() -> Result<()> {
    // tracing → stderr (stdout is reserved for MCP traffic) + rolling daily
    // file in `<exe dir>/logs/speedy-language-context.log.YYYY-MM-DD` so every
    // CLI call leaves a durable record (duration, file counts, ...).
    let logs_dir = speedy_core::daemon_util::exe_log_dir();
    let file_appender = tracing_appender::rolling::daily(&logs_dir, "speedy-language-context.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so the writer flushes for the entire process lifetime.
    Box::leak(Box::new(file_guard));

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_writer(file_writer),
        )
        .init();

    let cli = Cli::parse();
    let root = resolve_root(&cli.workspace_path)?;

    let rt = tokio::runtime::Runtime::new()?;
    let cmd_started = Instant::now();
    let result = rt.block_on(async {
        tokio::select! {
            res = async_main(cli, root) => res,
            _ = tokio::signal::ctrl_c() => {
                tracing::warn!(
                    target: "language-context",
                    elapsed_ms = cmd_started.elapsed().as_millis() as u64,
                    "interrupted by user (Ctrl+C) — exiting before completion"
                );
                Err(anyhow::anyhow!("interrupted by Ctrl+C"))
            }
        }
    });

    if let Err(e) = &result {
        tracing::error!(
            target: "language-context",
            error = %e,
            elapsed_ms = cmd_started.elapsed().as_millis() as u64,
            "command failed"
        );
    }
    result
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
        Commands::Sync => cmd_sync(&root, cli.json).await,
        Commands::Update { files } => cmd_update(&root, &files, cli.json).await,
        Commands::Status => cmd_status(&root, cli.json),
        Commands::Serve => mcp::run_server(root).await,
        Commands::Skeleton { files, detail } => cmd_skeleton(&root, &files, &detail),
        Commands::Search { query, top_k } => cmd_search(&root, &query, top_k, cli.json),
        Commands::ClearIndex => cmd_clear_index(&root, cli.json),
    }
}

/// Whether the code-intelligence (language_context) feature is enabled for
/// this workspace. Defaults to `false` (opt-in) — the index/update write paths
/// no-op when disabled so a fresh workspace or a git hook does no work.
fn language_context_enabled(root: &Path) -> bool {
    // An explicit user action (CLI / GUI button) sets SPEEDY_FORCE to bypass the
    // opt-in gate — an explicit index must always run.
    if std::env::var("SPEEDY_FORCE").map(|v| { let v = v.trim(); v == "1" || v.eq_ignore_ascii_case("true") }).unwrap_or(false) {
        return true;
    }
    Features::load(root).language_context
}

fn report_language_context_disabled(command: &str, as_json: bool) {
    info!(target: "language-context", command, "skipped (language_context disabled)");
    if as_json {
        println!(
            "{}",
            serde_json::json!({ "skipped": true, "reason": "language_context disabled" })
        );
    } else {
        println!(
            "language_context disabled — skipping {command}. Enable with: speedy enable slc"
        );
    }
}

async fn cmd_index(root: &Path, as_json: bool) -> Result<()> {
    if !language_context_enabled(root) {
        report_language_context_disabled("index", as_json);
        return Ok(());
    }
    let started = Instant::now();
    let indexer = Indexer::new(root)?;
    let stats = indexer.full_index().await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        target: "language-context",
        command = "index",
        workspace = %root.display(),
        files = stats.files_indexed,
        skipped = stats.files_skipped,
        symbols = stats.symbols_found,
        elapsed_ms,
        "index done"
    );
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

/// Incremental sync: re-parses only files whose content hash changed and prunes
/// deleted files. Unlike `cmd_index` it does not wipe the hash registry, so an
/// unchanged workspace is a near-instant no-op.
async fn cmd_sync(root: &Path, as_json: bool) -> Result<()> {
    if !language_context_enabled(root) {
        report_language_context_disabled("sync", as_json);
        return Ok(());
    }
    let started = Instant::now();
    let indexer = Indexer::new(root)?;
    let stats = indexer.sync().await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        target: "language-context",
        command = "sync",
        workspace = %root.display(),
        files = stats.files_indexed,
        skipped = stats.files_skipped,
        symbols = stats.symbols_found,
        elapsed_ms,
        "sync done"
    );
    if as_json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!(
            "synced {} files, skipped {}, {} symbols, {} ms",
            stats.files_indexed, stats.files_skipped, stats.symbols_found, stats.duration_ms
        );
    }
    Ok(())
}

async fn cmd_update(root: &Path, files: &[PathBuf], as_json: bool) -> Result<()> {
    if !language_context_enabled(root) {
        report_language_context_disabled("update", as_json);
        return Ok(());
    }
    let started = Instant::now();
    let indexer = Indexer::new(root)?;
    let stats = indexer.index_files(files).await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        target: "language-context",
        command = "update",
        workspace = %root.display(),
        requested = files.len(),
        files = stats.files_indexed,
        skipped = stats.files_skipped,
        symbols = stats.symbols_found,
        elapsed_ms,
        "update done"
    );
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
    let started = Instant::now();
    let store = GraphStore::open(root)?;
    let files = store.file_count()?;
    let symbols = store.symbol_count()?;
    let edges = store.edge_count()?;
    let last_indexed = store.get_meta("last_indexed_at")?.unwrap_or_else(|| "never".to_string());
    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        target: "language-context",
        command = "status",
        workspace = %root.display(),
        files,
        symbols,
        edges,
        elapsed_ms,
        "status done"
    );
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
    let started = Instant::now();
    let store = GraphStore::open(root)?;
    let detail = detail.parse()?;
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let out = skeleton::get_skeleton(&store, root, &refs, detail)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        target: "language-context",
        command = "skeleton",
        workspace = %root.display(),
        files = files.len(),
        elapsed_ms,
        "skeleton done"
    );
    println!("{out}");
    Ok(())
}

fn cmd_clear_index(root: &Path, as_json: bool) -> Result<()> {
    let store = GraphStore::open(root)?;
    store.clear_all()?;
    if as_json {
        println!("{}", serde_json::json!({ "cleared": true }));
    } else {
        println!("Index cleared.");
    }
    Ok(())
}

fn cmd_search(root: &Path, query: &str, top_k: usize, as_json: bool) -> Result<()> {
    let started = Instant::now();
    let store = GraphStore::open(root)?;
    let results = search::search(&store, query, top_k)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        target: "language-context",
        command = "search",
        workspace = %root.display(),
        query = %query,
        top_k,
        results = results.len(),
        elapsed_ms,
        "search done"
    );
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
