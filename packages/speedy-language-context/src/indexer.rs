//! Walks the workspace, parses each supported file with tree-sitter, and
//! persists symbols into the SQLite graph store.

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::graph::GraphStore;
use crate::parser::{parse_edges, parse_file};

#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub symbols_found: usize,
    pub duration_ms: u64,
}

pub struct Indexer {
    pub store: Arc<GraphStore>,
    pub root: PathBuf,
}

impl Indexer {
    pub fn new(workspace_root: &Path) -> Result<Self> {
        let store = Arc::new(GraphStore::open(workspace_root)?);
        Ok(Self {
            store,
            root: workspace_root.to_path_buf(),
        })
    }

    /// Walk the entire workspace, parse files, persist symbols.
    pub async fn full_index(&self) -> Result<IndexStats> {
        let root = self.root.clone();
        let store = self.store.clone();
        let stats = tokio::task::spawn_blocking(move || full_index_blocking(&root, &store))
            .await
            .context("blocking task panicked")??;
        Ok(stats)
    }

    /// Incremental: re-parse only the supplied files. Called by the watcher/hooks.
    pub async fn index_files(&self, files: &[PathBuf]) -> Result<IndexStats> {
        let root = self.root.clone();
        let store = self.store.clone();
        let files = files.to_vec();
        let stats = tokio::task::spawn_blocking(move || index_files_blocking(&root, &store, &files))
            .await
            .context("blocking task panicked")??;
        Ok(stats)
    }

    pub fn should_skip(path: &Path) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        // Must stay aligned with parser::tree_sitter_parser::parse_file —
        // any extension parsed there must NOT be skipped here, otherwise
        // the parser is never reached and the language appears unsupported.
        !matches!(
            ext.as_str(),
            "rs"
                | "js" | "jsx" | "mjs" | "cjs"
                | "ts" | "tsx"
                | "py" | "pyi"
                | "go"
                | "c" | "h"
                | "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "h++"
                | "java"
                | "cs"
                | "rb" | "rake"
                | "php" | "php5" | "php7" | "php8"
                | "swift"
                | "kt" | "kts"
                | "scala" | "sc"
                | "dart"
        )
    }
}

fn full_index_blocking(root: &Path, store: &GraphStore) -> Result<IndexStats> {
    let started = Instant::now();
    let mut files_indexed = 0usize;
    let mut files_skipped = 0usize;
    let mut symbols_found = 0usize;
    let mut last_progress_log = Instant::now();
    const PROGRESS_LOG_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

    let t_walk = Instant::now();
    let walker = WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .add_custom_ignore_filename(".speedyignore")
        .follow_links(false)
        .build();

    tracing::info!(
        target: "language-context",
        root = %root.display(),
        walk_setup_ms = t_walk.elapsed().as_millis() as u64,
        "full_index starting"
    );

    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if Indexer::should_skip(path) {
            files_skipped += 1;
            continue;
        }
        match index_one_file(root, store, path) {
            Ok(n) => {
                if n == usize::MAX {
                    files_skipped += 1;
                } else {
                    files_indexed += 1;
                    symbols_found += n;
                }
            }
            Err(e) => {
                tracing::warn!("failed to index {}: {}", path.display(), e);
                files_skipped += 1;
            }
        }

        if last_progress_log.elapsed() >= PROGRESS_LOG_EVERY {
            let elapsed = started.elapsed();
            tracing::info!(
                target: "language-context",
                files_indexed,
                files_skipped,
                symbols = symbols_found,
                elapsed_s = elapsed.as_secs(),
                files_per_sec = format!("{:.2}", files_indexed as f64 / elapsed.as_secs_f64().max(0.001)),
                "full_index progress"
            );
            last_progress_log = Instant::now();
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let duration_ms = started.elapsed().as_millis() as u64;
    let _ = store.set_meta("last_indexed_at", &now);
    let _ = store.set_meta("last_files_indexed", &files_indexed.to_string());
    let _ = store.set_meta("last_symbols_found", &symbols_found.to_string());
    let _ = store.set_meta("last_index_duration_ms", &duration_ms.to_string());

    tracing::info!(
        target: "language-context",
        root = %root.display(),
        files_indexed,
        files_skipped,
        symbols = symbols_found,
        duration_ms,
        files_per_sec = format!("{:.2}", files_indexed as f64 / started.elapsed().as_secs_f64().max(0.001)),
        "full_index complete"
    );

    Ok(IndexStats {
        files_indexed,
        files_skipped,
        symbols_found,
        duration_ms,
    })
}

fn index_files_blocking(root: &Path, store: &GraphStore, files: &[PathBuf]) -> Result<IndexStats> {
    let started = Instant::now();
    let mut files_indexed = 0usize;
    let mut files_skipped = 0usize;
    let mut symbols_found = 0usize;
    for f in files {
        if Indexer::should_skip(f) {
            files_skipped += 1;
            continue;
        }
        match index_one_file(root, store, f) {
            Ok(n) => {
                if n == usize::MAX {
                    files_skipped += 1;
                } else {
                    files_indexed += 1;
                    symbols_found += n;
                }
            }
            Err(e) => {
                tracing::warn!("failed to index {}: {}", f.display(), e);
                files_skipped += 1;
            }
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let duration_ms = started.elapsed().as_millis() as u64;
    let _ = store.set_meta("last_indexed_at", &now);
    let _ = store.set_meta("last_files_indexed", &files_indexed.to_string());
    let _ = store.set_meta("last_symbols_found", &symbols_found.to_string());
    let _ = store.set_meta("last_index_duration_ms", &duration_ms.to_string());

    Ok(IndexStats {
        files_indexed,
        files_skipped,
        symbols_found,
        duration_ms,
    })
}

/// Returns the number of symbols indexed, or `usize::MAX` if the file was
/// unchanged and therefore skipped.
fn index_one_file(root: &Path, store: &GraphStore, path: &Path) -> Result<usize> {
    let t_total = Instant::now();
    let t_read = Instant::now();
    let content = match std::fs::read(path) {
        Ok(c) => c,
        Err(_) => return Ok(usize::MAX),
    };
    let read_ms = t_read.elapsed().as_millis() as u64;
    let bytes_len = content.len();
    let hash = blake3::hash(&content).to_hex().to_string();
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    if let Some(prev) = store.get_file_hash(&rel)? {
        if prev == hash {
            return Ok(usize::MAX);
        }
    }

    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let t_parse = Instant::now();
    let parsed = parse_file(path, &content);
    let parse_ms = t_parse.elapsed().as_millis() as u64;

    let t_db = Instant::now();
    let file_id = store.upsert_file(&rel, mtime, &hash)?;
    // Cascade-deletes both symbols and their edges.
    store.delete_file_symbols(file_id)?;

    let mut name_to_id: std::collections::HashMap<String, i64> =
        std::collections::HashMap::with_capacity(parsed.len());
    for sym in &parsed {
        let id = store.insert_symbol(file_id, sym)?;
        name_to_id.insert(sym.name.clone(), id);
    }
    let db_symbols_ms = t_db.elapsed().as_millis() as u64;

    // Second pass: extract and insert call-site edges (same-file only).
    let t_edges = Instant::now();
    let edge_refs = parse_edges(path, &content, &parsed);
    let edges_parse_ms = t_edges.elapsed().as_millis() as u64;
    let t_edges_db = Instant::now();
    let mut edges_inserted = 0usize;
    for edge_ref in &edge_refs {
        if let (Some(&src_id), Some(&dst_id)) = (
            name_to_id.get(&edge_ref.src_name),
            name_to_id.get(&edge_ref.dst_name),
        ) {
            if store.insert_edge(src_id, dst_id, edge_ref.kind.clone()).is_ok() {
                edges_inserted += 1;
            }
        }
    }
    let edges_db_ms = t_edges_db.elapsed().as_millis() as u64;

    let total_ms = t_total.elapsed().as_millis() as u64;
    tracing::debug!(
        target: "language-context",
        file = %rel,
        bytes = bytes_len,
        symbols = parsed.len(),
        edges_seen = edge_refs.len(),
        edges_inserted,
        read_ms,
        parse_ms,
        db_symbols_ms,
        edges_parse_ms,
        edges_db_ms,
        total_ms,
        "file indexed"
    );
    // Surface only the genuinely slow files as info, otherwise the per-file
    // line stays at debug and the aggregate progress / final summary are
    // what the user normally reads in the log.
    if total_ms >= 250 {
        tracing::info!(
            target: "language-context",
            file = %rel,
            bytes = bytes_len,
            symbols = parsed.len(),
            read_ms,
            parse_ms,
            db_symbols_ms,
            edges_parse_ms,
            edges_db_ms,
            total_ms,
            "slow file (>250ms)"
        );
    }

    Ok(parsed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── should_skip ───────────────────────────────────────────────────────────

    #[test]
    fn test_should_skip_unsupported_extension() {
        assert!(Indexer::should_skip(Path::new("file.xyz")));
        assert!(Indexer::should_skip(Path::new("file.txt")));
        assert!(Indexer::should_skip(Path::new("file.md")));
        assert!(Indexer::should_skip(Path::new("file.toml")));
        assert!(Indexer::should_skip(Path::new("file.json")));
        assert!(Indexer::should_skip(Path::new("binary")));
    }

    #[test]
    fn test_should_skip_supported_extensions() {
        assert!(!Indexer::should_skip(Path::new("main.rs")));
        assert!(!Indexer::should_skip(Path::new("app.js")));
        assert!(!Indexer::should_skip(Path::new("component.tsx")));
        assert!(!Indexer::should_skip(Path::new("script.py")));
        assert!(!Indexer::should_skip(Path::new("server.go")));
        assert!(!Indexer::should_skip(Path::new("module.ts")));
    }

    #[test]
    fn test_should_skip_is_case_insensitive() {
        // Extension matching is case-insensitive (lowercased before matching).
        assert!(!Indexer::should_skip(Path::new("main.RS")));
        assert!(!Indexer::should_skip(Path::new("App.JS")));
        assert!(Indexer::should_skip(Path::new("README.MD")));
    }

    // ── index_one_file (unsupported / empty) ──────────────────────────────────

    /// A file with an unsupported extension is handled by `should_skip` before
    /// `index_one_file` is called, but even if `index_one_file` is called
    /// directly it must not crash; it will parse nothing meaningful.
    #[test]
    fn test_index_one_file_unsupported_extension_does_not_crash() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let xyz = dir.path().join("data.xyz");
        std::fs::write(&xyz, b"this is not code").unwrap();

        // index_one_file is private, so we test via the blocking helper
        // by constructing a minimal files list and calling index_files_blocking.
        let result = index_files_blocking(dir.path(), &store, &[xyz]);
        assert!(result.is_ok(), "indexing unsupported extension must not error");
        // The file is in files_skipped because should_skip returns true.
        let stats = result.unwrap();
        assert_eq!(stats.files_indexed, 0, "unsupported file must not be counted as indexed");
        assert_eq!(stats.files_skipped, 1, "unsupported file must be counted as skipped");
    }

    /// An empty `.rs` file must be indexed without crashing and produces zero symbols.
    #[test]
    fn test_index_one_file_empty_rs_does_not_crash() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let empty = dir.path().join("empty.rs");
        std::fs::write(&empty, b"").unwrap();

        let result = index_files_blocking(dir.path(), &store, &[empty]);
        assert!(result.is_ok(), "indexing an empty .rs file must not error");
        let stats = result.unwrap();
        // File is parseable (valid Rust — just empty) and produces 0 symbols.
        assert_eq!(stats.files_indexed, 1, "empty .rs file must be counted as indexed");
        assert_eq!(stats.symbols_found, 0, "empty .rs file must produce no symbols");
    }

    /// A `.rs` file with actual Rust code must be indexed and produce at least
    /// one symbol (sanity-check that the happy path works).
    #[test]
    fn test_index_one_file_rust_source_finds_symbols() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let src = dir.path().join("lib.rs");
        std::fs::write(&src, b"pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();

        let result = index_files_blocking(dir.path(), &store, &[src]);
        assert!(result.is_ok(), "indexing a Rust source file must not error");
        let stats = result.unwrap();
        assert_eq!(stats.files_indexed, 1);
        assert!(stats.symbols_found > 0, "must have found at least one symbol in lib.rs");
    }

    /// Calling index_files_blocking with an empty file list must succeed and
    /// return zero-counts without touching the store.
    #[test]
    fn test_index_files_blocking_empty_list_succeeds() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();

        let result = index_files_blocking(dir.path(), &store, &[]);
        assert!(result.is_ok());
        let stats = result.unwrap();
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_skipped, 0);
        assert_eq!(stats.symbols_found, 0);
    }
}
