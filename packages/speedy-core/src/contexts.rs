//! Shared orchestration of the three Speedy "context" workers.
//!
//! Speedy ships three independent worker binaries, each gated by a per-workspace
//! feature flag stored in `<workspace>/.speedy/config.toml` under `[features]`:
//!
//! | feature flag       | worker binary             | what it does                |
//! |--------------------|---------------------------|-----------------------------|
//! | `speedy_indexer`   | `speedy-ai-context`       | semantic / vector index     |
//! | `language_context` | `speedy-language-context` | code-intelligence graph     |
//! | `text_context`     | `speedy-text-context`     | text-symbol index           |
//!
//! Historically the logic that fans a single user action out to all three
//! workers lived only inside `speedy-daemon`. This module hoists it into the
//! shared library so the daemon, the CLI and the GUI can all drive the same
//! orchestration — in particular so the CLI/GUI keep working **without a
//! daemon** (each spawns the workers directly, in-process).
//!
//! Every spawned child gets `SPEEDY_NO_DAEMON=1` so a worker never tries to
//! re-enter the daemon, and (on Windows) `CREATE_NO_WINDOW` so console-subsystem
//! children don't pop up a window.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;
use tracing::{error, info, warn};

/// A console-subsystem child would otherwise allocate a console window per
/// spawn. We capture stdio explicitly, so suppressing the window is correct.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── Features ────────────────────────────────────────────────────────────────

fn default_false() -> bool {
    false
}

/// Per-workspace feature toggles. Read from `<ws>/.speedy/config.toml`
/// (`[features]`) or, as a fallback, the global daemon config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Features {
    #[serde(default = "default_false")]
    pub speedy_indexer: bool,
    #[serde(default = "default_false")]
    pub language_context: bool,
    #[serde(default = "default_false")]
    pub text_context: bool,
}

impl Features {
    /// Defaults for an unconfigured workspace: every context is opt-in (off)
    /// until the user enables it explicitly.
    pub fn defaults() -> Self {
        Self {
            speedy_indexer: false,
            language_context: false,
            text_context: false,
        }
    }
}

fn global_config_path() -> PathBuf {
    let home = if let Some(p) =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        PathBuf::from(p)
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    home.join(".speedy").join("daemon-features.toml")
}

fn workspace_config_path(workspace: &str) -> PathBuf {
    // `speedy_subdir` is idempotent, so a workspace path that already ends in
    // `.speedy` resolves to `<ws>/.speedy/config.toml`, not a nested one.
    crate::daemon_util::speedy_subdir(&PathBuf::from(workspace)).join("config.toml")
}

/// Load features: workspace config (under `[features]`) if available, else global.
pub fn load_features(workspace: Option<&str>) -> Features {
    if let Some(ws) = workspace.filter(|s| !s.is_empty()) {
        let path = workspace_config_path(ws);
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(doc) = toml::from_str::<toml::Value>(&raw) {
                    if let Some(section) = doc.get("features") {
                        if let Ok(f) = section.clone().try_into::<Features>() {
                            return f;
                        }
                    }
                }
            }
        }
        return Features::defaults();
    }
    // Global fallback
    let path = global_config_path();
    if !path.exists() {
        return Features::defaults();
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Features::defaults(),
    };
    toml::from_str::<Features>(&raw).unwrap_or_else(|_| Features::defaults())
}

/// Persist a feature toggle. If `workspace` is given, writes to
/// `<workspace>/.speedy/config.toml` under `[features]`; otherwise writes to
/// the global daemon config. Other TOML sections are preserved.
pub fn set_feature(workspace: Option<&str>, name: &str, enabled: bool) -> Result<()> {
    let mut f = load_features(workspace);
    match name {
        "speedy_indexer" | "speedy-indexer" => f.speedy_indexer = enabled,
        "language_context" | "language-context" => f.language_context = enabled,
        "text_context" | "text-context" => f.text_context = enabled,
        other => anyhow::bail!("unknown feature: {other}"),
    }

    if let Some(ws) = workspace.filter(|s| !s.is_empty()) {
        let path = workspace_config_path(ws);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Merge into existing TOML, preserving other sections.
        let mut doc: toml::Value = if path.exists() {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&raw)
                .unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()))
        } else {
            toml::Value::Table(toml::value::Table::new())
        };
        let features_val = toml::Value::try_from(&f)?;
        if let toml::Value::Table(table) = &mut doc {
            table.insert("features".to_string(), features_val);
        }
        let serialized = toml::to_string_pretty(&doc)?;
        std::fs::write(path, serialized)?;
    } else {
        let path = global_config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(&f)?;
        std::fs::write(path, serialized)?;
    }
    Ok(())
}

// ── Worker-binary resolvers ───────────────────────────────────────────────────

/// Locate a worker executable: next to the running binary first (daemon, CLI
/// and GUI are installed in the same directory), then one level up when running
/// from `target/debug/deps/` under `cargo test`, then on `PATH`.
fn find_worker_exe(stem: &str) -> Option<PathBuf> {
    let exe_name = format!("{stem}{}", std::env::consts::EXE_SUFFIX);
    if let Ok(self_exe) = std::env::current_exe() {
        if let Some(dir) = self_exe.parent() {
            let candidate = dir.join(&exe_name);
            if candidate.is_file() {
                return Some(candidate);
            }
            if dir.file_name().and_then(|s| s.to_str()) == Some("deps") {
                if let Some(parent) = dir.parent() {
                    let candidate = parent.join(&exe_name);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var)
            .map(|dir| dir.join(&exe_name))
            .find(|p| p.is_file())
    })
}

/// Path to the `speedy-ai-context` worker. Falls back to the bare name (resolved
/// via `PATH` at spawn time) when it cannot be located up front.
pub fn find_ai_context_exe() -> PathBuf {
    find_worker_exe("speedy-ai-context").unwrap_or_else(|| PathBuf::from("speedy-ai-context"))
}

/// Path to the `speedy-language-context` worker, or `None` if not found.
pub fn find_language_context_exe() -> Option<PathBuf> {
    find_worker_exe("speedy-language-context")
}

/// Path to the `speedy-text-context` worker, or `None` if not found.
pub fn find_text_context_exe() -> Option<PathBuf> {
    find_worker_exe("speedy-text-context")
}

/// Locate any sibling Speedy executable by stem (e.g. `"speedy-cli"`): next to
/// the running binary first, then on `PATH`. Used by the GUI to spawn the CLI.
pub fn find_sibling_exe(stem: &str) -> Option<PathBuf> {
    find_worker_exe(stem)
}

// ── Orchestration ─────────────────────────────────────────────────────────────

/// Incremental sync of a workspace across **all enabled contexts**. Each worker
/// runs its own incremental `sync` command, which uses the shared hash registry
/// (mtime + content hash) to re-process only the files that actually changed and
/// to prune files deleted from disk — so an unchanged workspace is a near-instant
/// no-op and sync is never slower than a full index.
///
/// Which contexts run is decided exactly like [`reindex_workspace`] — the ones
/// the user enabled (see [`contexts_to_run`]). `force` marks an explicit user
/// action (the GUI/CLI "Sync" button): it sets `SPEEDY_FORCE` so each worker
/// bypasses its own opt-in gate. Automatic syncs (daemon initial sync / watcher)
/// pass `force = false`. Returns `true` when at least one context ran and
/// succeeded.
pub async fn sync_workspace(raw_path: &str, force: bool) -> Result<bool> {
    let canonical = std::path::Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    let features = load_features(Some(&path_str));
    let (run_ai, run_slc, run_text) = contexts_to_run(&features);

    if !run_ai && !run_slc && !run_text {
        info!(target: "sync", workspace = %path_str, "Sync skipped (no context enabled)");
        return Ok(false);
    }

    let started = Instant::now();
    let mut any_ok = false;

    // ── AI-context (semantic / vector) ──────────────────────────────────────
    if run_ai {
        let exe = find_ai_context_exe();
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.args(["-p", &path_str, "sync"]).env("SPEEDY_NO_DAEMON", "1");
        if force {
            cmd.env("SPEEDY_FORCE", "1");
        }
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        match cmd.output().await {
            Ok(o) if o.status.success() => {
                any_ok = true;
                info!(target: "sync", workspace = %path_str, stdout = %String::from_utf8_lossy(&o.stdout).trim(), "AI-context sync done");
            }
            Ok(o) => error!(target: "sync", workspace = %path_str, stderr = %String::from_utf8_lossy(&o.stderr).trim(), "AI-context sync failed"),
            Err(e) => error!(target: "sync", workspace = %path_str, error = %e, "failed to spawn AI-context sync"),
        }
    }

    // ── Language context (symbol graph) ─────────────────────────────────────
    if run_slc {
        match find_language_context_exe() {
            Some(exe) => {
                let mut cmd = tokio::process::Command::new(&exe);
                cmd.arg("--path").arg(&path_str).arg("sync").env("SPEEDY_NO_DAEMON", "1");
                if force {
                    cmd.env("SPEEDY_FORCE", "1");
                }
                #[cfg(windows)]
                cmd.creation_flags(CREATE_NO_WINDOW);
                match cmd.output().await {
                    Ok(o) if o.status.success() => {
                        any_ok = true;
                        info!(target: "sync", workspace = %path_str, stdout = %String::from_utf8_lossy(&o.stdout).trim(), "SLC sync done");
                    }
                    Ok(o) => error!(target: "sync", workspace = %path_str, stderr = %String::from_utf8_lossy(&o.stderr).trim(), "SLC sync failed"),
                    Err(e) => error!(target: "sync", workspace = %path_str, error = %e, "failed to spawn SLC sync"),
                }
            }
            None => warn!(target: "sync", workspace = %path_str, "speedy-language-context not found — skipping SLC sync"),
        }
    }

    // ── Text-symbol index ───────────────────────────────────────────────────
    if run_text {
        match find_text_context_exe() {
            Some(exe) => {
                let mut cmd = tokio::process::Command::new(&exe);
                cmd.arg("sync").arg(&path_str).env("SPEEDY_NO_DAEMON", "1");
                if force {
                    cmd.env("SPEEDY_FORCE", "1");
                }
                #[cfg(windows)]
                cmd.creation_flags(CREATE_NO_WINDOW);
                match cmd.output().await {
                    Ok(o) if o.status.success() => {
                        any_ok = true;
                        info!(target: "sync", workspace = %path_str, stdout = %String::from_utf8_lossy(&o.stdout).trim(), "text sync done");
                    }
                    Ok(o) => error!(target: "sync", workspace = %path_str, stderr = %String::from_utf8_lossy(&o.stderr).trim(), "text sync failed"),
                    Err(e) => error!(target: "sync", workspace = %path_str, error = %e, "failed to spawn text sync"),
                }
            }
            None => warn!(target: "sync", workspace = %path_str, "speedy-text-context not found — skipping text sync"),
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(target: "sync", workspace = %path_str, ms = elapsed_ms, run_ai, run_slc, run_text, any_ok, "Sync complete");
    Ok(any_ok)
}

/// Incremental **per-file** update across all enabled contexts. Unlike
/// [`sync_workspace`] (which walks the whole tree) this touches only the files
/// it's handed — the cheap path for the git `post-commit` hook, which already
/// knows exactly which files the commit changed. Each worker's own hash check
/// still skips a file whose content is unchanged.
///
/// `files` may be absolute or relative to the workspace root; entries that no
/// longer exist on disk are dropped (a delete is handled by the per-context
/// `update`, which removes stale entries). Which contexts run is decided by
/// [`contexts_to_run`]; `force` sets `SPEEDY_FORCE` so workers bypass their
/// opt-in gate. Returns `true` if at least one context ran successfully.
pub async fn update_files(raw_path: &str, files: &[String], force: bool) -> Result<bool> {
    let canonical = std::path::Path::new(raw_path).canonicalize()?;
    let root = canonical.to_string_lossy().to_string();

    let features = load_features(Some(&root));
    let (run_ai, run_slc, run_text) = contexts_to_run(&features);
    if !run_ai && !run_slc && !run_text {
        info!(target: "update", workspace = %root, "Update skipped (no context enabled)");
        return Ok(false);
    }

    // Resolve every input to an absolute path. Keep deletions (path no longer a
    // file) too — the per-context `update` prunes those from each index.
    let resolved: Vec<String> = files
        .iter()
        .map(|f| {
            let p = std::path::Path::new(f);
            let abs = if p.is_absolute() { p.to_path_buf() } else { canonical.join(p) };
            abs.to_string_lossy().to_string()
        })
        .collect();
    if resolved.is_empty() {
        return Ok(false);
    }

    let ai_exe = run_ai.then(find_ai_context_exe);
    let slc_exe = if run_slc { find_language_context_exe() } else { None };
    let text_exe = if run_text { find_text_context_exe() } else { None };

    let started = Instant::now();
    let mut any_ok = false;

    for file in &resolved {
        // ── AI-context: incremental single-file (re)index ──
        if let Some(ref exe) = ai_exe {
            let mut cmd = tokio::process::Command::new(exe);
            cmd.args(["-p", &root, "index", file]).env("SPEEDY_NO_DAEMON", "1");
            if force {
                cmd.env("SPEEDY_FORCE", "1");
            }
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            match cmd.output().await {
                Ok(o) if o.status.success() => any_ok = true,
                Ok(o) => error!(target: "update", workspace = %root, file = %file, stderr = %String::from_utf8_lossy(&o.stderr).trim(), "AI-context update failed"),
                Err(e) => error!(target: "update", workspace = %root, file = %file, error = %e, "failed to spawn AI-context update"),
            }
        }
        // ── Language context: symbol-graph update ──
        if let Some(ref exe) = slc_exe {
            let mut cmd = tokio::process::Command::new(exe);
            cmd.arg("--path").arg(&root).arg("update").arg(file).env("SPEEDY_NO_DAEMON", "1");
            if force {
                cmd.env("SPEEDY_FORCE", "1");
            }
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            match cmd.output().await {
                Ok(o) if o.status.success() => any_ok = true,
                Ok(o) => error!(target: "update", workspace = %root, file = %file, stderr = %String::from_utf8_lossy(&o.stderr).trim(), "SLC update failed"),
                Err(e) => error!(target: "update", workspace = %root, file = %file, error = %e, "failed to spawn SLC update"),
            }
        }
        // ── Text-symbol index update ──
        if let Some(ref exe) = text_exe {
            let mut cmd = tokio::process::Command::new(exe);
            cmd.arg("update").arg(&root).arg(file).env("SPEEDY_NO_DAEMON", "1");
            if force {
                cmd.env("SPEEDY_FORCE", "1");
            }
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            match cmd.output().await {
                Ok(o) if o.status.success() => any_ok = true,
                Ok(o) => error!(target: "update", workspace = %root, file = %file, stderr = %String::from_utf8_lossy(&o.stderr).trim(), "text update failed"),
                Err(e) => error!(target: "update", workspace = %root, file = %file, error = %e, "failed to spawn text update"),
            }
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(target: "update", workspace = %root, ms = elapsed_ms, files = resolved.len(), run_ai, run_slc, run_text, any_ok, "Per-file update complete");
    Ok(any_ok)
}

/// Decide which contexts a manual reindex runs: exactly the ones the user
/// enabled. An explicit "Index" must respect the checkboxes, so nothing extra
/// runs. When the user has enabled *nothing*, the caller
/// ([`reindex_workspace`]) surfaces a warning instead of indexing everything —
/// that way the action never silently creates databases for contexts the user
/// did not opt into. Returns `(run_ai, run_slc, run_text)`.
fn contexts_to_run(f: &Features) -> (bool, bool, bool) {
    (f.speedy_indexer, f.language_context, f.text_context)
}

/// Full reindex of a workspace: fans out to all three context workers in
/// sequence (see `contexts_to_run` for which ones). AI-context runs under a hard
/// wall-clock cap so a wedged child can never block the caller; SLC and
/// text-context always get a chance to run even if AI-context fails. Returns a
/// human-readable summary (`"ai-context: ok | slc: ok | text: disabled"`).
pub async fn reindex_workspace(raw_path: &str) -> Result<String> {
    let canonical = std::path::Path::new(raw_path).canonicalize()?;
    let path_str = canonical.to_string_lossy().to_string();

    // Hard wall-clock cap on the ai-context reindex. The child must NEVER be
    // able to wedge the caller: if it stops making progress (deadlock on a
    // lock, infinite loop, runaway embed loop), we kill it and continue with
    // SLC. 30 min is generous because the first index on a big repo can blow
    // through thousands of sequential Ollama embed calls.
    const AI_CONTEXT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);
    const AI_CONTEXT_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);

    // A manual reindex runs exactly the contexts the user enabled (see
    // `contexts_to_run`). Automatic indexing — daemon initial-sync and the
    // watcher — also respects the opt-in flags via their own paths.
    let features = load_features(Some(&path_str));
    let (run_ai, run_slc, run_text) = contexts_to_run(&features);

    // With nothing enabled there is nothing to do. Bail with a clear message so
    // the GUI/CLI tells the user to pick a feature instead of silently creating
    // databases for contexts they never opted into.
    if !run_ai && !run_slc && !run_text {
        anyhow::bail!(
            "No context enabled for this workspace — enable AI Context, \
             Language Context or Text Context first."
        );
    }

    let started = Instant::now();

    // AI-context reindex — when the indexer is enabled (or nothing is, see above).
    let (stdout, ai_ok, ai_timed_out) = if run_ai {
        let exe = find_ai_context_exe();
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.current_dir(&path_str)
            .args(["index", "--clear", "."])
            .env("SPEEDY_NO_DAEMON", "1")
            // Explicit reindex: bypass the worker's own opt-in gate.
            .env("SPEEDY_FORCE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        info!(target: "index", workspace = %path_str, timeout_s = AI_CONTEXT_TIMEOUT.as_secs(), "AI-context reindex starting");

        let child = cmd.spawn()?;
        let workspace_for_heartbeat = path_str.clone();
        let started_for_heartbeat = started;
        let heartbeat = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(AI_CONTEXT_HEARTBEAT);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await; // first tick fires immediately, skip it
            loop {
                ticker.tick().await;
                let secs = started_for_heartbeat.elapsed().as_secs();
                info!(
                    target: "index",
                    workspace = %workspace_for_heartbeat,
                    elapsed_s = secs,
                    "AI-context reindex still running (Ollama embed loop is the usual bottleneck)"
                );
            }
        });

        let wait_fut = child.wait_with_output();
        let (output_opt, timed_out) = match tokio::time::timeout(AI_CONTEXT_TIMEOUT, wait_fut).await {
            Ok(Ok(o)) => (Some(o), false),
            Ok(Err(e)) => {
                error!(target: "index", workspace = %path_str, error = %e, "AI-context wait error");
                (None, false)
            }
            Err(_) => {
                // Timeout fired. `kill_on_drop` reaps the process when `child`
                // is dropped, but `wait_with_output` already moved it; the kill
                // propagates via the OS once the pipes close.
                (None, true)
            }
        };
        heartbeat.abort();

        let elapsed_ms = started.elapsed().as_millis() as u64;

        let (stdout, stderr, exit_code, ok) = match output_opt {
            Some(o) => (
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
                o.status.code(),
                o.status.success(),
            ),
            None => (String::new(), String::new(), None, false),
        };

        if ok {
            info!(target: "index", workspace = %path_str, ms = elapsed_ms, stdout = %stdout.trim(), "AI-context reindex done");
        } else if timed_out {
            error!(
                target: "index",
                workspace = %path_str,
                ms = elapsed_ms,
                timeout_s = AI_CONTEXT_TIMEOUT.as_secs(),
                "AI-context reindex TIMED OUT and was killed — continuing with SLC if enabled"
            );
        } else {
            // Don't bail: SLC (speedy-language-context) is an independent feature
            // and must still get a chance to run even when AI-context fails on
            // this workspace (e.g. a panic on a single bad file). We log loudly
            // and proceed.
            error!(
                target: "index",
                workspace = %path_str,
                ms = elapsed_ms,
                exit_code = ?exit_code,
                stdout = %stdout.trim(),
                stderr = %stderr.trim(),
                "AI-context reindex failed (continuing with SLC if enabled)"
            );
        }

        (stdout, ok, timed_out)
    } else {
        info!(target: "index", workspace = %path_str, "AI-context reindex skipped (speedy_indexer disabled)");
        (String::new(), true, false)
    };

    let mut slc_ok: Option<bool> = None;
    let mut slc_err: Option<String> = None;
    if run_slc {
        match find_language_context_exe() {
            Some(slc_exe) => {
                // Clear the SLC graph DB before a full re-index so deleted files
                // don't leave stale symbols behind.
                let mut slc_clear_cmd = tokio::process::Command::new(&slc_exe);
                slc_clear_cmd
                    .arg("--path").arg(&path_str)
                    .arg("clear-index")
                    .env("SPEEDY_NO_DAEMON", "1")
                    .env("SPEEDY_FORCE", "1")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                #[cfg(windows)]
                slc_clear_cmd.creation_flags(CREATE_NO_WINDOW);
                let _ = slc_clear_cmd.output().await;

                info!(target: "index", workspace = %path_str, exe = %slc_exe.display(), "SLC index starting");
                let slc_started = Instant::now();
                let mut slc_cmd = tokio::process::Command::new(&slc_exe);
                slc_cmd
                    .arg("--path").arg(&path_str)
                    .arg("index")
                    .env("SPEEDY_NO_DAEMON", "1")
                    .env("SPEEDY_FORCE", "1")
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                #[cfg(windows)]
                slc_cmd.creation_flags(CREATE_NO_WINDOW);
                match slc_cmd.output().await {
                    Ok(o) => {
                        let slc_ms = slc_started.elapsed().as_millis() as u64;
                        let so = String::from_utf8_lossy(&o.stdout);
                        let se = String::from_utf8_lossy(&o.stderr);
                        if o.status.success() {
                            info!(
                                target: "index",
                                workspace = %path_str,
                                ms = slc_ms,
                                stdout = %so.trim(),
                                "SLC index done"
                            );
                            slc_ok = Some(true);
                        } else {
                            error!(
                                target: "index",
                                workspace = %path_str,
                                ms = slc_ms,
                                exit_code = ?o.status.code(),
                                stderr = %se.trim(),
                                stdout = %so.trim(),
                                "SLC index failed"
                            );
                            slc_ok = Some(false);
                            slc_err = Some(se.trim().to_string());
                        }
                    }
                    Err(e) => {
                        error!(target: "index", workspace = %path_str, error = %e, "failed to spawn SLC index");
                        slc_ok = Some(false);
                        slc_err = Some(e.to_string());
                    }
                }
            }
            None => {
                warn!(
                    target: "index",
                    workspace = %path_str,
                    "speedy-language-context executable not found next to the binary or in PATH — skipping SLC index"
                );
            }
        }
    } else {
        info!(target: "index", workspace = %path_str, "SLC index skipped (language_context feature disabled)");
    }

    // Text-symbol index — its `index` command clears and rebuilds on its own,
    // so no separate clear step is needed.
    let mut text_ok: Option<bool> = None;
    let mut text_err: Option<String> = None;
    if run_text {
        match find_text_context_exe() {
            Some(text_exe) => {
                info!(target: "index", workspace = %path_str, exe = %text_exe.display(), "text index starting");
                let text_started = Instant::now();
                let mut text_cmd = tokio::process::Command::new(&text_exe);
                text_cmd
                    .arg("index")
                    .arg(&path_str)
                    .env("SPEEDY_NO_DAEMON", "1")
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                #[cfg(windows)]
                text_cmd.creation_flags(CREATE_NO_WINDOW);
                match text_cmd.output().await {
                    Ok(o) => {
                        let text_ms = text_started.elapsed().as_millis() as u64;
                        let se = String::from_utf8_lossy(&o.stderr);
                        if o.status.success() {
                            info!(target: "index", workspace = %path_str, ms = text_ms, "text index done");
                            text_ok = Some(true);
                        } else {
                            error!(
                                target: "index",
                                workspace = %path_str,
                                ms = text_ms,
                                exit_code = ?o.status.code(),
                                stderr = %se.trim(),
                                "text index failed"
                            );
                            text_ok = Some(false);
                            text_err = Some(se.trim().to_string());
                        }
                    }
                    Err(e) => {
                        error!(target: "index", workspace = %path_str, error = %e, "failed to spawn text index");
                        text_ok = Some(false);
                        text_err = Some(e.to_string());
                    }
                }
            }
            None => {
                warn!(
                    target: "index",
                    workspace = %path_str,
                    "speedy-text-context executable not found next to the binary or in PATH — skipping text index"
                );
            }
        }
    } else {
        info!(target: "index", workspace = %path_str, "text index skipped (text_context feature disabled)");
    }

    // Decide overall result. If AI-context failed AND SLC didn't succeed,
    // surface an error to the caller. Otherwise return a summary so the GUI
    // toast tells the user which half ran.
    let ai_status = if !run_ai {
        "skipped"
    } else if ai_ok {
        "ok"
    } else if ai_timed_out {
        "timeout"
    } else {
        "failed"
    };
    let summary = format!(
        "ai-context: {} | slc: {} | text: {}",
        ai_status,
        match slc_ok {
            Some(true) => "ok".to_string(),
            Some(false) => format!("failed ({})", slc_err.as_deref().unwrap_or("see logs")),
            None => if run_slc { "not-found".to_string() } else { "disabled".to_string() },
        },
        match text_ok {
            Some(true) => "ok".to_string(),
            Some(false) => format!("failed ({})", text_err.as_deref().unwrap_or("see logs")),
            None => if run_text { "not-found".to_string() } else { "disabled".to_string() },
        }
    );

    // Only bail if something that was supposed to run actually failed.
    if run_ai && !ai_ok && slc_ok != Some(true) {
        anyhow::bail!("reindex failed — {summary}");
    }

    Ok(if run_ai && ai_ok { stdout.trim().to_string() } else { summary })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_default_all_off() {
        let f = Features::defaults();
        assert!(!f.speedy_indexer && !f.language_context && !f.text_context);
    }

    #[test]
    fn load_features_unconfigured_workspace_is_defaults() {
        let dir = std::env::temp_dir().join(format!("speedy_ctx_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = load_features(Some(&dir.to_string_lossy()));
        assert!(!f.speedy_indexer && !f.language_context && !f.text_context);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_feature_roundtrip_workspace_config() {
        let dir = std::env::temp_dir().join(format!("speedy_ctx_set_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ws = dir.to_string_lossy().to_string();

        set_feature(Some(&ws), "language_context", true).unwrap();
        let f = load_features(Some(&ws));
        assert!(f.language_context, "language_context should be enabled after set_feature");
        assert!(!f.speedy_indexer, "untouched flags stay off");

        set_feature(Some(&ws), "language_context", false).unwrap();
        assert!(!load_features(Some(&ws)).language_context);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_feature_unknown_name_errors() {
        assert!(set_feature(None, "bogus_feature", true).is_err());
    }

    #[test]
    fn unconfigured_reindex_runs_nothing() {
        // An explicit "Index" must respect the checkboxes: with nothing enabled
        // (the default) no context runs. `reindex_workspace` turns this into a
        // warning rather than indexing everything.
        let (run_ai, run_slc, run_text) = contexts_to_run(&Features::defaults());
        assert!(!run_ai && !run_slc && !run_text);
    }

    #[test]
    fn reindex_respects_partial_selection() {
        // When the user has enabled something specific, only that runs.
        let f = Features { speedy_indexer: true, language_context: false, text_context: false };
        assert_eq!(contexts_to_run(&f), (true, false, false));
        let f = Features { speedy_indexer: false, language_context: true, text_context: true };
        assert_eq!(contexts_to_run(&f), (false, true, true));
    }

    #[tokio::test]
    async fn sync_disabled_is_noop_false() {
        let dir = std::env::temp_dir().join(format!("speedy_ctx_sync_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ran = sync_workspace(&dir.to_string_lossy(), false).await.unwrap();
        assert!(!ran, "automatic sync must be a no-op when speedy_indexer is disabled");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
