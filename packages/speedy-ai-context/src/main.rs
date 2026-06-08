use clap::Parser;
use speedy_core::daemon_client::DaemonClient;
use speedy_core::daemon_util;
use speedy_core::workspace;
use speedy_ai_context::cli::{Cli, Commands, WorkspaceAction};
use speedy_ai_context::hooks;
use anyhow::Result;
use tracing::info;

#[cfg(test)]
fn resolve_path(path: &Option<String>) -> Result<std::path::PathBuf> {
    match path {
        Some(p) => Ok(std::path::PathBuf::from(p).canonicalize()?),
        None => Ok(std::env::current_dir()?),
    }
}

fn main() -> Result<()> {
    use tracing_subscriber::prelude::*;
    let logs_dir = speedy_core::daemon_util::exe_log_dir();
    let file_appender = tracing_appender::rolling::daily(&logs_dir, "speedy-ai-context.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so the non-blocking writer thread keeps flushing for the
    // entire process lifetime. Without this, if the process exits via Ctrl+C
    // or a hard kill, the in-flight log lines can be lost — including the
    // final "index done" summary.
    Box::leak(Box::new(file_guard));

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
    let cmd_started = std::time::Instant::now();
    let result = rt.block_on(async {
        tokio::select! {
            res = async_main(cli) => res,
            _ = tokio::signal::ctrl_c() => {
                tracing::warn!(
                    target: "ai-context",
                    elapsed_ms = cmd_started.elapsed().as_millis() as u64,
                    "interrupted by user (Ctrl+C) — exiting before completion"
                );
                Err(anyhow::anyhow!("interrupted by Ctrl+C"))
            }
        }
    });

    if let Err(e) = &result {
        tracing::error!(
            target: "ai-context",
            error = %e,
            elapsed_ms = cmd_started.elapsed().as_millis() as u64,
            "command failed"
        );
    }
    result
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

    // The daemon is opt-in and never auto-started. If one happens to be
    // running we register this workspace with it; if not, we simply proceed —
    // the command runs in-process (standalone) without spawning a daemon.
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
    }

    Ok(())
}

// ── Feature helpers (read/write <workspace>/.speedy/config.toml) ─────────────

struct WorkspaceFeatures {
    speedy_indexer: bool,
    language_context: bool,
}

fn load_workspace_features(root: &std::path::Path) -> WorkspaceFeatures {
    let path = daemon_util::speedy_subdir(root).join("config.toml");
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(doc) = toml::from_str::<toml::Value>(&raw) {
            let get = |key: &str| {
                doc.get("features")
                    .and_then(|f| f.get(key))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            };
            return WorkspaceFeatures {
                speedy_indexer: get("speedy_indexer"),
                language_context: get("language_context"),
            };
        }
    }
    // Opt-in by default: nothing runs until the user enables a feature.
    WorkspaceFeatures { speedy_indexer: false, language_context: false }
}

fn save_workspace_features(root: &std::path::Path, f: &WorkspaceFeatures) -> anyhow::Result<()> {
    let dir = daemon_util::speedy_subdir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let mut doc: toml::Value = if path.exists() {
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&raw)
            .unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()))
    } else {
        toml::Value::Table(toml::value::Table::new())
    };
    let mut section = toml::value::Table::new();
    section.insert("speedy_indexer".to_string(), toml::Value::Boolean(f.speedy_indexer));
    section.insert("language_context".to_string(), toml::Value::Boolean(f.language_context));
    if let toml::Value::Table(table) = &mut doc {
        table.insert("features".to_string(), toml::Value::Table(section));
    }
    std::fs::write(path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

fn feature_key(name: &str) -> anyhow::Result<&'static str> {
    match name.to_lowercase().as_str() {
        "speedy" | "speedy_indexer" | "speedy-indexer" => Ok("speedy_indexer"),
        "slc" | "language_context" | "language-context" => Ok("language_context"),
        other => anyhow::bail!("unknown feature '{other}'. Use 'speedy' or 'slc'"),
    }
}

fn set_feature_value(f: &mut WorkspaceFeatures, key: &str, value: bool) {
    match key {
        "speedy_indexer" => f.speedy_indexer = value,
        "language_context" => f.language_context = value,
        _ => {}
    }
}

/// Whether the file indexer is enabled for the current workspace.
/// Reads `[features].speedy_indexer` from `.speedy/config.toml`; defaults to
/// `false` (opt-in). The index/sync/reembed write paths no-op when disabled so
/// that a fresh workspace — or a git hook firing on a workspace where the user
/// has not opted in — does no work.
fn speedy_indexer_enabled() -> bool {
    // An explicit user action (CLI command / GUI button) sets SPEEDY_FORCE to
    // bypass the opt-in gate — pressing a button must always do the work.
    if env_flag("SPEEDY_FORCE") {
        return true;
    }
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    load_workspace_features(&root).speedy_indexer
}

/// Path to this workspace's AI-context SQLite DB (the file `Indexer::new`
/// would create when it opens the store).
fn ai_db_path() -> std::path::PathBuf {
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    daemon_util::workspace_data_dir(&root).join("sac.sqlite")
}

/// Whether a read-only command (query / context / --read) may open the store.
/// Opening it *creates* `sac.sqlite`, so when the indexer is disabled we only
/// allow it if the DB already exists — otherwise a status/query on a workspace
/// the user never opted into would leave a ghost database behind.
fn ai_read_allowed() -> bool {
    speedy_indexer_enabled() || ai_db_path().exists()
}

/// True when an env var is set to a truthy value (`1`/`true`, case-insensitive).
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

/// Print the "feature disabled" notice and signal the caller to stop.
fn report_indexer_disabled(command: &str, json: bool) {
    info!(target: "ai-context", command, "skipped (speedy_indexer disabled)");
    if json {
        println!(
            "{}",
            serde_json::json!({ "skipped": true, "reason": "speedy_indexer disabled" })
        );
    } else {
        println!("speedy_indexer disabled — skipping {command}. Enable with: speedy enable speedy");
    }
}

fn should_skip_daemon_check(cli: &Cli) -> bool {
    if cli.workspaces || cli.daemons {
        return true;
    }

    if let Some(cmd) = &cli.command {
        return matches!(cmd,
            // Per flow.md §6 + §7: lanciando `speedy-ai-context.exe` a mano,
            // i sub-comandi puntuali (index/query/context/sync/reembed)
            // devono girare interamente in-process — niente auto-spawn del
            // daemon, niente registrazione automatica del workspace. Il
            // path "via daemon" è invece `speedy-cli` → daemon → exec, che
            // spawna comunque `speedy-ai-context.exe` con SPEEDY_NO_DAEMON=1.
            Commands::Index { .. }
            | Commands::Query { .. }
            | Commands::Context
            | Commands::Sync
            | Commands::Reembed
            | Commands::Daemon
            | Commands::Workspace { .. }
            | Commands::InstallHooks { .. }
            | Commands::UninstallHooks { .. }
            | Commands::Enable { .. }
            | Commands::Disable { .. }
            | Commands::Features
            | Commands::ClearIndex
        );
    }

    cli.command.is_none()
}

async fn async_main(cli: Cli) -> Result<()> {
    ensure_daemon(&cli).await?;

    let config = speedy_core::config::Config::load();

    if cli.workspaces {
        let workspaces = workspace::list()?;
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
        return Ok(());
    }

    if cli.daemons {
        let client = DaemonClient::new(resolve_socket_name(&cli));
        let alive = client.is_alive().await;
        if cli.json {
            let list = if alive { client.get_all_workspaces().await.unwrap_or_default() } else { Vec::new() };
            let out = serde_json::json!({ "alive": alive, "workspaces": list });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else if !alive {
            println!("Daemon not running.");
        } else {
            let list = client.get_all_workspaces().await?;
            if list.is_empty() {
                println!("No daemon workspaces.");
            } else {
                for ws in &list {
                    println!("[active] {ws}");
                }
            }
        }
        return Ok(());
    }

    if let Some(prompt) = cli.read {
        if !ai_read_allowed() {
            report_indexer_disabled("read", cli.json);
            return Ok(());
        }
        let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
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
        if !speedy_indexer_enabled() {
            report_indexer_disabled("modify", cli.json);
            return Ok(());
        }
        if let Some(file) = cli.file {
            tokio::fs::write(&file, &content).await?;
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
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
        Some(Commands::Index { subdir, clear }) => {
            if !speedy_indexer_enabled() {
                report_indexer_disabled("index", cli.json);
                return Ok(());
            }
            let started = std::time::Instant::now();
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
            if *clear {
                indexer.db.clear_all_chunks().await?;
            }
            let stats = indexer.index_directory(subdir).await?;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            info!(
                target: "ai-context",
                command = "index",
                subdir = %subdir,
                files = stats.files,
                chunks = stats.chunks,
                elapsed_ms,
                "index done"
            );
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!("Indexed {} files, {} chunks in {} ms", stats.files, stats.chunks, elapsed_ms);
            }
        }
        Some(Commands::Query { query, top_k }) => {
            if !ai_read_allowed() {
                report_indexer_disabled("query", cli.json);
                return Ok(());
            }
            let started = std::time::Instant::now();
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
            let k = top_k.unwrap_or(5);
            let results = indexer.query(query, k).await?;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            info!(
                target: "ai-context",
                command = "query",
                query = %query,
                top_k = k,
                results = results.len(),
                elapsed_ms,
                "query done"
            );
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
            if !ai_read_allowed() {
                report_indexer_disabled("context", cli.json);
                return Ok(());
            }
            let started = std::time::Instant::now();
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
            let ctx = indexer.project_context().await?;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            info!(
                target: "ai-context",
                command = "context",
                files = ctx.file_count,
                chunks = ctx.chunk_count,
                elapsed_ms,
                "context done"
            );
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
            if !speedy_indexer_enabled() {
                report_indexer_disabled("sync", cli.json);
                return Ok(());
            }
            let started = std::time::Instant::now();
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
            let stats = indexer.sync_all().await?;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            info!(
                target: "ai-context",
                command = "sync",
                added = stats.files,
                updated = stats.chunks,
                removed = stats.removed,
                elapsed_ms,
                "sync done"
            );
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!(
                    "Synced: {} added, {} updated, {} removed in {} ms",
                    stats.files, stats.chunks, stats.removed, elapsed_ms
                );
            }
        }
        Some(Commands::Reembed) => {
            if !speedy_indexer_enabled() {
                report_indexer_disabled("reembed", cli.json);
                return Ok(());
            }
            let started = std::time::Instant::now();
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
            let stats = indexer.reembed().await?;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            info!(
                target: "ai-context",
                command = "reembed",
                model = %indexer.model,
                files = stats.files,
                chunks = stats.chunks,
                elapsed_ms,
                "reembed done"
            );
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!(
                    "Reembedded with model '{}': {} files, {} chunks in {}ms",
                    indexer.model, stats.files, stats.chunks, stats.duration_ms
                );
            }
        }
        Some(Commands::Daemon) => {
            let socket = resolve_socket_name(&cli);
            daemon_util::spawn_daemon_process(&socket)?;
            if cli.json {
                println!("{}", serde_json::json!({ "started": true, "socket": socket }));
            } else {
                println!("Daemon started on socket {socket}");
            }
        }
        Some(Commands::Workspace { action }) => match action {
            WorkspaceAction::List => {
                let workspaces = workspace::list()?;
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
        },
        Some(Commands::InstallHooks { path, force }) => {
            let root = match path {
                Some(p) => p.clone(),
                None => std::env::current_dir()?,
            };
            let report = hooks::install_hooks(&root, *force)?;
            if cli.json {
                let out = serde_json::json!({
                    "installed": report.installed,
                    "skipped": report.skipped.iter().map(|(n, r)| serde_json::json!({"hook": n, "reason": r})).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for name in &report.installed {
                    println!("installed  {name}");
                }
                for (name, reason) in &report.skipped {
                    println!("skipped    {name}  ({reason})");
                }
                if report.installed.is_empty() && report.skipped.is_empty() {
                    println!("Nothing to install.");
                } else if !report.skipped.is_empty() {
                    println!("Tip: use --force to overwrite skipped hooks.");
                }
            }
        }
        Some(Commands::UninstallHooks { path }) => {
            let root = match path {
                Some(p) => p.clone(),
                None => std::env::current_dir()?,
            };
            let removed = hooks::uninstall_hooks(&root)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&removed)?);
            } else if removed.is_empty() {
                println!("No Speedy-managed hooks found.");
            } else {
                for name in &removed {
                    println!("removed    {name}");
                }
            }
        }
        Some(Commands::Enable { feature }) => {
            let key = feature_key(feature)?;
            let root = std::env::current_dir()?;
            let mut f = load_workspace_features(&root);
            set_feature_value(&mut f, key, true);
            save_workspace_features(&root, &f)?;
            if cli.json {
                println!("{}", serde_json::json!({ "enabled": key, "ok": true }));
            } else {
                println!("● {key} enabled");
            }
        }
        Some(Commands::Disable { feature }) => {
            let key = feature_key(feature)?;
            let root = std::env::current_dir()?;
            let mut f = load_workspace_features(&root);
            set_feature_value(&mut f, key, false);
            save_workspace_features(&root, &f)?;
            if cli.json {
                println!("{}", serde_json::json!({ "disabled": key, "ok": true }));
            } else {
                println!("○ {key} disabled");
            }
        }
        Some(Commands::Features) => {
            let root = std::env::current_dir()?;
            let f = load_workspace_features(&root);
            if cli.json {
                println!("{}", serde_json::json!({
                    "speedy_indexer": f.speedy_indexer,
                    "language_context": f.language_context,
                }));
            } else {
                let bullet = |on: bool| if on { "●" } else { "○" };
                println!("{} speedy_indexer   (file indexer)", bullet(f.speedy_indexer));
                println!("{} language_context (code intelligence)", bullet(f.language_context));
            }
        }
        Some(Commands::ClearIndex) => {
            // Nothing to clear (and no DB to create) when the indexer is off and
            // no index exists yet.
            if !ai_read_allowed() {
                report_indexer_disabled("clear-index", cli.json);
                return Ok(());
            }
            let indexer = speedy_ai_context::indexer::Indexer::new(&config).await?;
            indexer.db.clear_all_chunks().await?;
            if cli.json {
                println!("{}", serde_json::json!({ "cleared": true }));
            } else {
                println!("Index cleared.");
            }
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
    fn test_skip_daemon_check_install_hooks() {
        let cli = make_cli(&["speedy", "install-hooks"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_uninstall_hooks() {
        let cli = make_cli(&["speedy", "uninstall-hooks"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_install_hooks_with_force() {
        let cli = make_cli(&["speedy", "install-hooks", "--force"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_install_hooks_with_path() {
        let tmp = std::env::temp_dir();
        let path_str = tmp.to_string_lossy().to_string();
        let cli = make_cli(&["speedy", "install-hooks", "--path", &path_str]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_no_command() {
        let cli = make_cli(&["speedy"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_skips_index() {
        // `index` is a standalone one-shot — it must not auto-spawn or talk
        // to the daemon. See the comment in should_skip_daemon_check.
        let cli = make_cli(&["speedy", "index"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_skips_query() {
        // Per flow.md §6 + §7: subcomando one-shot da CLI → standalone.
        let cli = make_cli(&["speedy", "query", "test"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_skips_sync() {
        let cli = make_cli(&["speedy", "sync"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_skips_context() {
        let cli = make_cli(&["speedy", "context"]);
        assert!(should_skip_daemon_check(&cli));
    }

    #[test]
    fn test_skip_daemon_check_skips_reembed() {
        let cli = make_cli(&["speedy", "reembed"]);
        assert!(should_skip_daemon_check(&cli));
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
