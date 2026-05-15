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
        !matches!(
            ext.as_str(),
            "rs" | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "py" | "pyi" | "go"
        )
    }
}

fn full_index_blocking(root: &Path, store: &GraphStore) -> Result<IndexStats> {
    let started = Instant::now();
    let mut files_indexed = 0usize;
    let mut files_skipped = 0usize;
    let mut symbols_found = 0usize;

    let walker = WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .add_custom_ignore_filename(".speedyignore")
        .follow_links(false)
        .build();

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
    }

    let now = chrono::Utc::now().to_rfc3339();
    let _ = store.set_meta("last_indexed_at", &now);

    Ok(IndexStats {
        files_indexed,
        files_skipped,
        symbols_found,
        duration_ms: started.elapsed().as_millis() as u64,
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
    let _ = store.set_meta("last_indexed_at", &now);

    Ok(IndexStats {
        files_indexed,
        files_skipped,
        symbols_found,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Returns the number of symbols indexed, or `usize::MAX` if the file was
/// unchanged and therefore skipped.
fn index_one_file(root: &Path, store: &GraphStore, path: &Path) -> Result<usize> {
    let content = match std::fs::read(path) {
        Ok(c) => c,
        Err(_) => return Ok(usize::MAX),
    };
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

    let parsed = parse_file(path, &content);
    let file_id = store.upsert_file(&rel, mtime, &hash)?;
    // Cascade-deletes both symbols and their edges.
    store.delete_file_symbols(file_id)?;

    let mut name_to_id: std::collections::HashMap<String, i64> =
        std::collections::HashMap::with_capacity(parsed.len());
    for sym in &parsed {
        let id = store.insert_symbol(file_id, sym)?;
        name_to_id.insert(sym.name.clone(), id);
    }

    // Second pass: extract and insert call-site edges (same-file only).
    let edge_refs = parse_edges(path, &content, &parsed);
    for edge_ref in &edge_refs {
        if let (Some(&src_id), Some(&dst_id)) = (
            name_to_id.get(&edge_ref.src_name),
            name_to_id.get(&edge_ref.dst_name),
        ) {
            let _ = store.insert_edge(src_id, dst_id, edge_ref.kind.clone());
        }
    }

    Ok(parsed.len())
}
