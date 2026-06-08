use anyhow::Result;
use rusqlite::Connection;
use speedy_core::hash_registry::HashRegistry;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::{config, db, tokenize, walk};

const MAX_FILE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// Full re-index. Returns `(indexed, total)` — files written vs. files walked.
pub fn index(conn: &mut Connection, root: &Path) -> Result<(usize, usize)> {
    let allowed_exts = config::load_or_create_extensions(root)?;

    // Clear the shared hash registry so every file is treated as new.
    if let Ok(registry) = HashRegistry::open(root) {
        let _ = registry.delete_context("text");
    }

    {
        let tx = conn.transaction()?;
        db::clear_all(&tx)?;
        tx.commit()?;
    }

    let registry = HashRegistry::open(root).ok();
    let files = walk::walk(root, &allowed_exts)?;
    let total = files.len();
    let mut indexed = 0usize;

    for (i, (path, ext)) in files.iter().enumerate() {
        if i > 0 && i % 100 == 0 {
            eprintln!("[speedy-text] indexing: {}/{}", i, total);
        }

        let norm = config::normalize_path(root, path);

        let content_bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[speedy-text] warning: cannot read {}: {}", norm, e);
                continue;
            }
        };

        if content_bytes.contains(&0u8) {
            continue; // binary content
        }
        if content_bytes.len() > MAX_FILE_BYTES {
            eprintln!("[speedy-text] warning: skipping large file {} ({} bytes)", norm, content_bytes.len());
            continue;
        }

        let content = match std::str::from_utf8(&content_bytes) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[speedy-text] warning: non-UTF-8 file {}, skipping", norm);
                continue;
            }
        };

        let hash = HashRegistry::hash_bytes(&content_bytes);
        let mtime_secs = file_mtime_secs(path);
        let tokens = tokenize::tokenize(content);

        {
            let tx = conn.transaction()?;
            db::insert_occurrences(&*tx, &norm, ext, &tokens)?;
            db::upsert_indexed_file(&*tx, &norm, ext, &hash)?;
            tx.commit()?;
        }

        if let Some(ref reg) = registry {
            let _ = reg.set_indexed(path, "text", &hash, mtime_secs);
        }

        indexed += 1;
    }

    db::set_meta(conn, "last_index_at", &now_secs_str())?;
    eprintln!("[speedy-text] index done. {}/{} files indexed", indexed, total);
    Ok((indexed, total))
}

/// Incremental sync. Returns `(updated, removed)`.
pub fn sync(conn: &mut Connection, root: &Path) -> Result<(usize, usize)> {
    let allowed_exts = config::load_or_create_extensions(root)?;
    let files = walk::walk(root, &allowed_exts)?;
    let total = files.len();

    let walked_set: std::collections::HashSet<String> = files
        .iter()
        .map(|(p, _)| config::normalize_path(root, p))
        .collect();

    let registry = HashRegistry::open(root).ok();
    let mut processed = 0usize;

    for (i, (path, ext)) in files.iter().enumerate() {
        if i > 0 && i % 100 == 0 {
            eprintln!("[speedy-text] sync: {}/{}", i, total);
        }

        let norm = config::normalize_path(root, path);

        // Fast mtime check via shared registry (avoids reading the file at all
        // when the file hasn't changed since last index).
        if let Some(ref reg) = registry {
            if reg.mtime_unchanged(path, "text") {
                continue;
            }
        }

        let content_bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[speedy-text] warning: cannot read {}: {}", norm, e);
                continue;
            }
        };

        if content_bytes.contains(&0u8) || content_bytes.len() > MAX_FILE_BYTES {
            continue;
        }

        let hash = HashRegistry::hash_bytes(&content_bytes);

        // Hash fallback: mtime changed but content is identical.
        if db::get_file_hash(conn, &norm)?.as_deref() == Some(hash.as_str()) {
            // Update stored mtime so the fast-path works next time.
            if let Some(ref reg) = registry {
                let mtime = file_mtime_secs(path);
                let _ = reg.set_indexed(path, "text", &hash, mtime);
            }
            continue;
        }

        let content = match std::str::from_utf8(&content_bytes) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[speedy-text] warning: non-UTF-8 file {}, skipping", norm);
                continue;
            }
        };

        let tokens = tokenize::tokenize(content);

        {
            let tx = conn.transaction()?;
            db::delete_file(&*tx, &norm)?;
            db::insert_occurrences(&*tx, &norm, ext, &tokens)?;
            db::upsert_indexed_file(&*tx, &norm, ext, &hash)?;
            tx.commit()?;
        }

        if let Some(ref reg) = registry {
            let mtime = file_mtime_secs(path);
            let _ = reg.set_indexed(path, "text", &hash, mtime);
        }

        processed += 1;
    }

    // Remove files deleted from disk
    let db_paths = db::all_indexed_paths(conn)?;
    let mut removed = 0usize;
    for db_path_str in db_paths {
        if !walked_set.contains(&db_path_str) {
            db::delete_file(conn, &db_path_str)?;
            removed += 1;
        }
    }

    db::set_meta(conn, "last_sync_at", &now_secs_str())?;
    eprintln!(
        "[speedy-text] sync done. {} updated, {} removed",
        processed, removed
    );
    Ok((processed, removed))
}

/// Incrementally (re)index a single file. Used by the daemon file watcher so a
/// change to one file doesn't trigger a full-tree walk. If the file no longer
/// exists, is binary/too-large, or its extension isn't tracked, the stored
/// occurrences for that path are removed.
pub fn update_file(conn: &mut Connection, root: &Path, file: &Path) -> Result<()> {
    let allowed_exts = config::load_or_create_extensions(root)?;
    let norm = config::normalize_path(root, file);

    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // Untracked extension or deleted file → drop any stale occurrences.
    if !allowed_exts.contains(&ext) || !file.exists() {
        let tx = conn.transaction()?;
        db::delete_file(&*tx, &norm)?;
        tx.commit()?;
        return Ok(());
    }

    let content_bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[speedy-text] warning: cannot read {}: {}", norm, e);
            return Ok(());
        }
    };

    if content_bytes.contains(&0u8) || content_bytes.len() > MAX_FILE_BYTES {
        // Binary or oversized: ensure it isn't lingering in the index.
        let tx = conn.transaction()?;
        db::delete_file(&*tx, &norm)?;
        tx.commit()?;
        return Ok(());
    }

    let hash = HashRegistry::hash_bytes(&content_bytes);
    let registry = HashRegistry::open(root).ok();

    // Unchanged content → only refresh the registry mtime, skip the rewrite.
    if db::get_file_hash(conn, &norm)?.as_deref() == Some(hash.as_str()) {
        if let Some(ref reg) = registry {
            let mtime = file_mtime_secs(file);
            let _ = reg.set_indexed(file, "text", &hash, mtime);
        }
        return Ok(());
    }

    let content = match std::str::from_utf8(&content_bytes) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("[speedy-text] warning: non-UTF-8 file {}, skipping", norm);
            return Ok(());
        }
    };

    let tokens = tokenize::tokenize(content);

    {
        let tx = conn.transaction()?;
        db::delete_file(&*tx, &norm)?;
        db::insert_occurrences(&*tx, &norm, &ext, &tokens)?;
        db::upsert_indexed_file(&*tx, &norm, &ext, &hash)?;
        tx.commit()?;
    }

    if let Some(ref reg) = registry {
        let mtime = file_mtime_secs(file);
        let _ = reg.set_indexed(file, "text", &hash, mtime);
    }

    Ok(())
}

/// Returns mtime as seconds since epoch — must match `HashRegistry::file_mtime_secs`
/// which also uses `as_secs()`, so `set_indexed` and `mtime_unchanged` agree on units.
fn file_mtime_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_secs_str() -> String {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
